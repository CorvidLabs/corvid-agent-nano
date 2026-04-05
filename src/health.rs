//! Health check HTTP server for the running agent.
//!
//! When enabled via `--health-port`, starts a minimal TCP HTTP server that
//! serves `GET /health` with a JSON status payload. Useful for Docker health
//! checks, systemd watchdog, and monitoring dashboards.

use std::sync::Arc;
use std::time::Instant;

use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio::sync::RwLock;

/// Mutable state tracked during the agent message loop.
pub struct HealthState {
    started_at: Instant,
    /// Network name (e.g. "testnet").
    pub network: String,
    /// Timestamp of the most recently received message, if any.
    pub last_message_at: Option<DateTime<Utc>>,
    /// Whether the last algod call succeeded.
    pub algod_connected: bool,
    /// Whether the indexer is reachable.
    pub indexer_connected: bool,
    /// Whether the hub is reachable (irrelevant in P2P mode).
    pub hub_connected: bool,
}

impl HealthState {
    /// Create a new health state for the given network.
    pub fn new(network: impl Into<String>) -> Self {
        Self {
            started_at: Instant::now(),
            network: network.into(),
            last_message_at: None,
            algod_connected: false,
            indexer_connected: false,
            hub_connected: false,
        }
    }

    fn to_response(&self) -> HealthResponse {
        HealthResponse {
            status: if self.algod_connected {
                "healthy"
            } else {
                "degraded"
            },
            network: self.network.clone(),
            uptime_secs: self.started_at.elapsed().as_secs(),
            last_message_at: self.last_message_at.map(|dt| dt.to_rfc3339()),
            algod_connected: self.algod_connected,
            indexer_connected: self.indexer_connected,
            hub_connected: self.hub_connected,
        }
    }
}

/// JSON response body for `GET /health`.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub network: String,
    pub uptime_secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<String>,
    pub algod_connected: bool,
    pub indexer_connected: bool,
    pub hub_connected: bool,
}

/// Start a minimal HTTP health server on the given port.
///
/// Serves `GET /health` (and `GET /`) with the current health JSON.
/// All other paths return 404. Runs until the task is dropped.
pub async fn run_health_server(port: u16, state: Arc<RwLock<HealthState>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = match TcpListener::bind(("0.0.0.0", port)).await {
        Ok(l) => {
            tracing::info!(port, "health check server listening");
            l
        }
        Err(e) => {
            tracing::error!(error = %e, port, "failed to bind health check server");
            return;
        }
    };

    loop {
        let (mut stream, peer) = match listener.accept().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "health server accept error");
                continue;
            }
        };

        tracing::debug!(peer = %peer, "health request");
        let state = Arc::clone(&state);

        tokio::spawn(async move {
            let mut buf = [0u8; 256];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let request = std::str::from_utf8(&buf[..n]).unwrap_or("");

            let (status_line, body) = if request.starts_with("GET /health")
                || request.starts_with("GET / ")
                || request.starts_with("GET /\r")
            {
                let health = state.read().await.to_response();
                let body = serde_json::to_string(&health).unwrap_or_else(|_| "{}".to_string());
                ("200 OK", body)
            } else {
                ("404 Not Found", r#"{"error":"not found"}"#.to_string())
            };

            let response = format!(
                "HTTP/1.1 {}\r\n\
                 Content-Type: application/json\r\n\
                 Content-Length: {}\r\n\
                 Connection: close\r\n\
                 \r\n\
                 {}",
                status_line,
                body.len(),
                body
            );

            let _ = stream.write_all(response.as_bytes()).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_state_new() {
        let state = HealthState::new("testnet");
        assert_eq!(state.network, "testnet");
        assert!(!state.algod_connected);
        assert!(state.last_message_at.is_none());
    }

    #[test]
    fn health_response_degraded_when_algod_disconnected() {
        let state = HealthState::new("testnet");
        let resp = state.to_response();
        assert_eq!(resp.status, "degraded");
    }

    #[test]
    fn health_response_healthy_when_algod_connected() {
        let mut state = HealthState::new("mainnet");
        state.algod_connected = true;
        let resp = state.to_response();
        assert_eq!(resp.status, "healthy");
        assert_eq!(resp.network, "mainnet");
    }

    #[test]
    fn health_response_includes_uptime() {
        let state = HealthState::new("localnet");
        let resp = state.to_response();
        // Uptime should be very small (< 1s)
        assert!(resp.uptime_secs < 5);
    }

    #[test]
    fn health_response_omits_last_message_when_none() {
        let state = HealthState::new("testnet");
        let resp = state.to_response();
        let json = serde_json::to_string(&resp).unwrap();
        assert!(!json.contains("last_message_at"));
    }

    #[test]
    fn health_response_includes_last_message_when_set() {
        let mut state = HealthState::new("testnet");
        state.last_message_at = Some(Utc::now());
        let resp = state.to_response();
        assert!(resp.last_message_at.is_some());
    }
}
