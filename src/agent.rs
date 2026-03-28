//! Agent message loop — polls for AlgoChat messages and forwards them to the hub.

use std::sync::Arc;
use std::time::Duration;

use algochat::{AlgoChat, AlgodClient, EncryptionKeyStorage, IndexerClient, MessageCache};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

/// Configuration for the agent message loop.
pub struct AgentLoopConfig {
    /// How often to poll for new messages (seconds).
    pub poll_interval_secs: u64,
    /// Hub URL for corvid-agent API.
    pub hub_url: String,
    /// Agent display name.
    pub agent_name: String,
}

impl Default for AgentLoopConfig {
    fn default() -> Self {
        Self {
            poll_interval_secs: 5,
            hub_url: "http://localhost:3578".to_string(),
            agent_name: "nano".to_string(),
        }
    }
}

/// JSON payload sent to the hub's A2A task endpoint.
#[derive(Debug, Serialize)]
struct HubTaskRequest {
    message: String,
    #[serde(rename = "timeoutMs")]
    timeout_ms: u64,
}

/// JSON response from the hub's A2A task endpoint.
#[derive(Debug, Deserialize)]
struct HubTaskResponse {
    id: String,
    state: String,
}

/// Runs the agent message polling loop.
///
/// Continuously polls the Algorand indexer for new AlgoChat messages,
/// decrypts them, and forwards them to the corvid-agent hub API.
pub async fn run_message_loop<A, I, S, M>(
    client: Arc<AlgoChat<A, I, S, M>>,
    config: AgentLoopConfig,
) where
    A: AlgodClient + 'static,
    I: IndexerClient + 'static,
    S: EncryptionKeyStorage + 'static,
    M: MessageCache + 'static,
{
    let interval = Duration::from_secs(config.poll_interval_secs);
    let http = Client::new();

    info!(
        name = %config.agent_name,
        poll_secs = config.poll_interval_secs,
        hub = %config.hub_url,
        "starting message loop"
    );

    loop {
        match client.sync().await {
            Ok(messages) => {
                for msg in &messages {
                    info!(
                        from = %msg.sender,
                        to = %msg.recipient,
                        round = msg.confirmed_round,
                        "received message: {}",
                        truncate(&msg.content, 100)
                    );

                    forward_to_hub(&http, &config.hub_url, &msg.sender, &msg.content).await;
                }
                if !messages.is_empty() {
                    debug!(count = messages.len(), "processed messages");
                }
            }
            Err(e) => {
                warn!(error = %e, "sync failed — will retry");
            }
        }

        tokio::time::sleep(interval).await;
    }
}

/// Forward a decrypted AlgoChat message to the hub's A2A task endpoint.
///
/// Sends a POST to `{hub_url}/a2a/tasks/send` and logs the result.
/// Does not block on task completion — the hub processes asynchronously.
async fn forward_to_hub(http: &Client, hub_url: &str, sender: &str, content: &str) {
    let url = format!("{}/a2a/tasks/send", hub_url.trim_end_matches('/'));
    let payload = HubTaskRequest {
        message: format!("[AlgoChat from {}] {}", sender, content),
        timeout_ms: 300_000,
    };

    match http.post(&url).json(&payload).send().await {
        Ok(resp) => {
            if resp.status().is_success() {
                match resp.json::<HubTaskResponse>().await {
                    Ok(task) => {
                        info!(
                            task_id = %task.id,
                            state = %task.state,
                            "forwarded message to hub"
                        );
                    }
                    Err(e) => {
                        warn!(error = %e, "hub returned success but response parse failed");
                    }
                }
            } else {
                warn!(status = %resp.status(), "hub rejected message");
            }
        }
        Err(e) => {
            warn!(error = %e, "failed to forward message to hub (hub unreachable?)");
        }
    }
}

/// Truncate a string for logging.
fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_exact_length_unchanged() {
        assert_eq!(truncate("hello", 5), "hello");
    }

    #[test]
    fn truncate_long_string() {
        assert_eq!(truncate("hello world", 5), "hello");
    }

    #[test]
    fn truncate_empty_string() {
        assert_eq!(truncate("", 10), "");
    }

    #[test]
    fn default_config() {
        let config = AgentLoopConfig::default();
        assert_eq!(config.poll_interval_secs, 5);
        assert_eq!(config.hub_url, "http://localhost:3578");
        assert_eq!(config.agent_name, "nano");
    }

    #[test]
    fn hub_task_request_serializes_correctly() {
        let req = HubTaskRequest {
            message: "[AlgoChat from ALGO123] hello".to_string(),
            timeout_ms: 300_000,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["message"], "[AlgoChat from ALGO123] hello");
        assert_eq!(json["timeoutMs"], 300_000);
        // Verify camelCase rename
        assert!(json.get("timeout_ms").is_none());
    }
}
