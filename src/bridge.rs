//! Plugin bridge — JSON-RPC client for the plugin host sidecar.
//!
//! Connects to the plugin host's Unix domain socket and dispatches
//! JSON-RPC requests for plugin management and tool invocation.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::Mutex;
use tracing::debug;

/// JSON-RPC request envelope.
#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    method: String,
    params: serde_json::Value,
    id: u64,
}

/// JSON-RPC response envelope.
#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    result: Option<serde_json::Value>,
    error: Option<String>,
    #[allow(dead_code)]
    id: Option<serde_json::Value>,
}

/// Plugin manifest returned from plugin.list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    pub id: String,
    pub version: String,
    pub author: String,
    pub description: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub trust_tier: String,
}

/// Health status returned from health.check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    #[serde(default)]
    pub plugins: serde_json::Value,
    #[serde(default)]
    pub uptime_ms: u64,
}

/// Client connection to the plugin host sidecar.
pub struct PluginBridge {
    socket_path: PathBuf,
    conn: Mutex<Option<BridgeConn>>,
    next_id: AtomicU64,
}

struct BridgeConn {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
}

impl PluginBridge {
    /// Create a new bridge pointing at the given socket path.
    pub fn new(socket_path: &Path) -> Self {
        Self {
            socket_path: socket_path.to_path_buf(),
            conn: Mutex::new(None),
            next_id: AtomicU64::new(1),
        }
    }

    /// Connect (or reconnect) to the plugin host socket.
    pub async fn connect(&self) -> Result<()> {
        let stream = UnixStream::connect(&self.socket_path).await?;
        let (reader, writer) = stream.into_split();
        let mut conn = self.conn.lock().await;
        *conn = Some(BridgeConn {
            reader: BufReader::new(reader),
            writer,
        });
        debug!(socket = %self.socket_path.display(), "bridge connected");
        Ok(())
    }

    /// Send a JSON-RPC request and read the response.
    async fn call(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = JsonRpcRequest {
            method: method.to_string(),
            params,
            id,
        };

        let mut conn_guard = self.conn.lock().await;
        let conn = conn_guard
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("bridge not connected"))?;

        // Write request as a single line
        let mut payload = serde_json::to_string(&req)?;
        payload.push('\n');
        conn.writer.write_all(payload.as_bytes()).await?;

        // Read response line
        let mut line = String::new();
        let n = tokio::time::timeout(Duration::from_secs(30), conn.reader.read_line(&mut line))
            .await
            .map_err(|_| anyhow::anyhow!("plugin host response timeout (30s)"))?
            .map_err(|e| anyhow::anyhow!("socket read error: {e}"))?;

        if n == 0 {
            // Connection closed — invalidate
            *conn_guard = None;
            bail!("plugin host closed connection");
        }

        let resp: JsonRpcResponse = serde_json::from_str(&line)?;

        if let Some(err) = resp.error {
            bail!("plugin host error: {err}");
        }

        Ok(resp.result.unwrap_or(serde_json::Value::Null))
    }

    /// List all loaded plugins.
    pub async fn list_plugins(&self) -> Result<Vec<PluginInfo>> {
        let result = self.call("plugin.list", serde_json::json!({})).await?;
        let plugins = result
            .get("plugins")
            .cloned()
            .unwrap_or(serde_json::Value::Array(vec![]));
        let list: Vec<PluginInfo> = serde_json::from_value(plugins)?;
        Ok(list)
    }

    /// Invoke a plugin tool.
    pub async fn invoke(
        &self,
        plugin_id: &str,
        tool: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let result = self
            .call(
                "plugin.invoke",
                serde_json::json!({
                    "plugin_id": plugin_id,
                    "tool": tool,
                    "input": input,
                }),
            )
            .await?;

        // The host wraps the result in {"result": ...}
        Ok(result.get("result").cloned().unwrap_or(result))
    }

    /// Load a plugin from a WASM file path.
    pub async fn load_plugin(&self, wasm_path: &str, tier: &str) -> Result<serde_json::Value> {
        self.call(
            "plugin.load",
            serde_json::json!({
                "path": wasm_path,
                "tier": tier,
            }),
        )
        .await
    }

    /// Unload a plugin by ID.
    pub async fn unload_plugin(&self, plugin_id: &str) -> Result<serde_json::Value> {
        self.call("plugin.unload", serde_json::json!({ "id": plugin_id }))
            .await
    }

    /// Check plugin host health.
    pub async fn health(&self) -> Result<HealthStatus> {
        let result = self.call("health.check", serde_json::json!({})).await?;
        let status: HealthStatus = serde_json::from_value(result)?;
        Ok(status)
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_info_deserializes() {
        let json = r#"{"id":"hello-world","version":"0.1.0","author":"corvid","description":"test","capabilities":[],"trust_tier":"Untrusted","event_filter":[],"min_host_version":"0.1.0"}"#;
        let info: PluginInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.id, "hello-world");
        assert_eq!(info.version, "0.1.0");
    }

    #[test]
    fn plugin_info_deserializes_with_extra_fields() {
        // The manifest has fields we don't care about — serde should ignore them
        let json = r#"{"id":"test","version":"1.0.0","author":"a","description":"d","unknown_field":true}"#;
        let info: PluginInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.id, "test");
    }

    #[test]
    fn health_status_deserializes() {
        let json = r#"{"plugins":{"hello-world":"active"},"uptime_ms":12345}"#;
        let status: HealthStatus = serde_json::from_str(json).unwrap();
        assert_eq!(status.uptime_ms, 12345);
    }
}
