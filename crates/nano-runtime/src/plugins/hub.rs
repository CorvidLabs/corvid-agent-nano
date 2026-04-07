//! Hub plugin — forwards messages to the corvid-agent-server and relays responses.
//!
//! This extracts the hub-forwarding logic from agent.rs into a proper plugin.

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::action::Action;
use crate::event::{Event, EventKind};
use crate::plugin::{Plugin, PluginContext};

/// Hub plugin — bridges nano to a corvid-agent-server instance.
pub struct HubPlugin {
    hub_url: String,
    http: reqwest::Client,
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
    #[allow(dead_code)]
    state: String,
}

/// Full task status from the hub.
#[derive(Debug, Deserialize)]
struct HubTaskStatus {
    state: String,
    #[serde(default)]
    response: Option<String>,
}

const HUB_POLL_INTERVAL_MS: u64 = 3000;
const HUB_POLL_MAX_ATTEMPTS: u32 = 100;

impl HubPlugin {
    pub fn new(hub_url: impl Into<String>) -> Self {
        Self {
            hub_url: hub_url.into(),
            http: reqwest::Client::new(),
        }
    }

    /// Forward a message to the hub and poll for a response.
    async fn forward_and_get_response(
        &self,
        sender: &str,
        content: &str,
    ) -> Option<String> {
        let url = format!("{}/a2a/tasks/send", self.hub_url.trim_end_matches('/'));
        let payload = HubTaskRequest {
            message: format!("[AlgoChat from {}] {}", sender, content),
            timeout_ms: 300_000,
        };

        // Step 1: Forward to hub
        let task_id = match self.http.post(&url).json(&payload).send().await {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<HubTaskResponse>().await {
                    Ok(task) => {
                        info!(task_id = %task.id, "forwarded message to hub");
                        task.id
                    }
                    Err(e) => {
                        warn!(error = %e, "hub response parse failed");
                        return None;
                    }
                }
            }
            Ok(resp) => {
                warn!(status = %resp.status(), "hub rejected message");
                return None;
            }
            Err(e) => {
                warn!(error = %e, "hub unreachable");
                return None;
            }
        };

        // Step 2: Poll for result
        let status_url = format!(
            "{}/a2a/tasks/{}",
            self.hub_url.trim_end_matches('/'),
            task_id
        );

        for attempt in 1..=HUB_POLL_MAX_ATTEMPTS {
            tokio::time::sleep(std::time::Duration::from_millis(HUB_POLL_INTERVAL_MS)).await;

            match self.http.get(&status_url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(status) = resp.json::<HubTaskStatus>().await {
                        debug!(task_id = %task_id, state = %status.state, attempt, "polled hub");
                        match status.state.as_str() {
                            "completed" => return status.response,
                            "failed" | "cancelled" => {
                                warn!(task_id = %task_id, state = %status.state, "hub task failed");
                                return None;
                            }
                            _ => {} // still running
                        }
                    }
                }
                _ => {
                    debug!(attempt, "hub poll failed, retrying");
                }
            }
        }

        warn!(task_id = %task_id, "hub poll timed out");
        None
    }
}

#[async_trait]
impl Plugin for HubPlugin {
    fn name(&self) -> &str {
        "hub"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    async fn init(&mut self, ctx: &PluginContext) -> Result<()> {
        // Allow config override of hub URL
        if let Some(url) = ctx.config.get("url").and_then(|v| v.as_str()) {
            self.hub_url = url.to_string();
        }
        info!(url = %self.hub_url, "hub plugin initialized");
        Ok(())
    }

    async fn handle_event(&self, event: &Event, _ctx: &PluginContext) -> Result<Vec<Action>> {
        match event {
            Event::MessageReceived(msg) => {
                match self.forward_and_get_response(&msg.sender, &msg.content).await {
                    Some(response) => Ok(vec![Action::SendMessage {
                        to: msg.sender.clone(),
                        content: response,
                    }]),
                    None => Ok(vec![Action::SendMessage {
                        to: msg.sender.clone(),
                        content: "[error] Agent hub is unreachable or timed out.".to_string(),
                    }]),
                }
            }
            _ => Ok(vec![]),
        }
    }

    fn subscriptions(&self) -> Vec<EventKind> {
        vec![EventKind::MessageReceived]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hub_plugin_name_and_version() {
        let plugin = HubPlugin::new("http://localhost:3578");
        assert_eq!(plugin.name(), "hub");
        assert!(!plugin.version().is_empty());
    }

    #[test]
    fn hub_task_request_serialization() {
        let req = HubTaskRequest {
            message: "test".to_string(),
            timeout_ms: 300_000,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["timeoutMs"], 300_000);
        assert!(json.get("timeout_ms").is_none());
    }

    #[tokio::test]
    async fn hub_plugin_init_with_config_override() {
        let mut plugin = HubPlugin::new("http://default:3578");
        let mut config = toml::Table::new();
        config.insert(
            "url".to_string(),
            toml::Value::String("http://custom:9999".into()),
        );
        let ctx = PluginContext {
            agent_address: "test".into(),
            agent_name: "test".into(),
            state: Default::default(),
            config,
        };
        plugin.init(&ctx).await.unwrap();
        assert_eq!(plugin.hub_url, "http://custom:9999");
    }

    #[tokio::test]
    async fn hub_plugin_ignores_non_message_events() {
        let plugin = HubPlugin::new("http://localhost:3578");
        let ctx = PluginContext {
            agent_address: "test".into(),
            agent_name: "test".into(),
            state: Default::default(),
            config: Default::default(),
        };
        let actions = plugin
            .handle_event(
                &Event::Timer {
                    timestamp: chrono::Utc::now(),
                },
                &ctx,
            )
            .await
            .unwrap();
        assert!(actions.is_empty());
    }
}
