//! Hub client for communicating with corvid-agent's API.
//!
//! Handles Flock Directory registration, heartbeats, and A2A task forwarding.

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

/// Client for the corvid-agent hub API.
pub struct HubClient {
    client: Client,
    base_url: String,
    agent_id: Option<String>,
}

/// Registration request for the Flock Directory.
#[derive(Serialize)]
struct RegisterRequest<'a> {
    address: &'a str,
    name: &'a str,
    description: &'a str,
    #[serde(rename = "instanceUrl")]
    instance_url: &'a str,
    capabilities: Vec<&'a str>,
}

/// Registration response from the Flock Directory.
#[derive(Deserialize)]
struct RegisterResponse {
    id: String,
    name: String,
}

/// A2A task submission request.
#[derive(Serialize)]
struct TaskRequest<'a> {
    message: &'a str,
    #[serde(rename = "timeoutMs", skip_serializing_if = "Option::is_none")]
    timeout_ms: Option<u64>,
}

/// A2A task response.
#[derive(Deserialize)]
struct TaskResponse {
    id: String,
    state: String,
    messages: Vec<TaskMessage>,
}

/// A message within an A2A task.
#[derive(Deserialize)]
struct TaskMessage {
    role: String,
    parts: Vec<TaskPart>,
}

/// A part of a task message.
#[derive(Deserialize)]
struct TaskPart {
    #[serde(rename = "type")]
    part_type: String,
    text: Option<String>,
}

impl HubClient {
    /// Creates a new hub client.
    pub fn new(base_url: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            agent_id: None,
        }
    }

    /// Registers this agent with the Flock Directory.
    ///
    /// Returns the assigned agent ID on success.
    pub async fn register(
        &mut self,
        address: &str,
        name: &str,
        encryption_key_hex: &str,
    ) -> Result<String> {
        let description = format!(
            "Lightweight Rust agent (nano). Encryption key: {}",
            encryption_key_hex
        );

        let body = RegisterRequest {
            address,
            name,
            description: &description,
            instance_url: "", // nano doesn't host an HTTP server
            capabilities: vec!["messaging", "lightweight"],
        };

        let url = format!("{}/api/flock-directory/agents", self.base_url);
        debug!(url = %url, name = %name, "registering with flock directory");

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .context("Failed to connect to hub for registration")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Registration failed ({}): {}", status, text);
        }

        let reg: RegisterResponse = resp
            .json()
            .await
            .context("Failed to parse registration response")?;

        info!(id = %reg.id, name = %reg.name, "registered with flock directory");
        self.agent_id = Some(reg.id.clone());
        Ok(reg.id)
    }

    /// Sends a heartbeat to keep this agent's status active.
    pub async fn heartbeat(&self) -> Result<()> {
        let agent_id = self
            .agent_id
            .as_ref()
            .context("Cannot heartbeat: not registered")?;

        let url = format!(
            "{}/api/flock-directory/agents/{}/heartbeat",
            self.base_url, agent_id
        );

        let resp = self
            .client
            .post(&url)
            .send()
            .await
            .context("Failed to send heartbeat")?;

        if !resp.status().is_success() {
            warn!(status = %resp.status(), "heartbeat failed");
        } else {
            debug!("heartbeat sent");
        }

        Ok(())
    }

    /// Forwards a message to corvid-agent via the A2A protocol and polls for a response.
    ///
    /// This submits a task, then polls until the task completes or times out.
    /// Returns the agent's text response.
    pub async fn forward_message(
        &self,
        message: &str,
        sender_address: &str,
    ) -> Result<String> {
        let prefixed = format!("[AlgoChat from {}] {}", sender_address, message);

        let body = TaskRequest {
            message: &prefixed,
            timeout_ms: Some(300_000), // 5 minutes
        };

        let url = format!("{}/a2a/tasks/send", self.base_url);
        debug!(url = %url, "forwarding message to hub");

        let resp = self
            .client
            .post(&url)
            .header("X-Source-Agent", "nano")
            .json(&body)
            .send()
            .await
            .context("Failed to submit A2A task")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("A2A task submission failed ({}): {}", status, text);
        }

        let task: TaskResponse = resp
            .json()
            .await
            .context("Failed to parse A2A task response")?;

        info!(task_id = %task.id, state = %task.state, "A2A task submitted");

        // Poll for completion
        self.poll_task(&task.id).await
    }

    /// Polls an A2A task until it completes or fails.
    async fn poll_task(&self, task_id: &str) -> Result<String> {
        let url = format!("{}/a2a/tasks/{}", self.base_url, task_id);
        let max_polls = 100; // 100 * 3s = 5 minutes

        for i in 0..max_polls {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;

            let resp = self
                .client
                .get(&url)
                .send()
                .await
                .context("Failed to poll A2A task")?;

            if !resp.status().is_success() {
                warn!(status = %resp.status(), poll = i, "task poll failed");
                continue;
            }

            let task: TaskResponse = resp
                .json()
                .await
                .context("Failed to parse task poll response")?;

            match task.state.as_str() {
                "completed" => {
                    // Extract the last agent message's text
                    let response_text = task
                        .messages
                        .iter()
                        .rev()
                        .find(|m| m.role == "agent")
                        .and_then(|m| {
                            m.parts
                                .iter()
                                .find(|p| p.part_type == "text")
                                .and_then(|p| p.text.clone())
                        })
                        .unwrap_or_else(|| "(no response)".to_string());

                    info!(task_id = %task_id, "A2A task completed");
                    return Ok(response_text);
                }
                "failed" => {
                    anyhow::bail!("A2A task failed");
                }
                _ => {
                    debug!(task_id = %task_id, state = %task.state, poll = i, "task still working");
                }
            }
        }

        anyhow::bail!("A2A task timed out after polling")
    }
}
