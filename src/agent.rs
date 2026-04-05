//! Agent message loop — polls for AlgoChat messages, forwards them to the hub,
//! waits for responses, and relays replies back on-chain.

use std::sync::Arc;
use std::time::Duration;

use algochat::{AlgoChat, AlgodClient, EncryptionKeyStorage, IndexerClient, MessageCache};
use chrono::Utc;
use ed25519_dalek::SigningKey;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::transaction;

/// Configuration for the agent message loop.
pub struct AgentLoopConfig {
    /// How often to poll for new messages (seconds).
    pub poll_interval_secs: u64,
    /// Hub URL for corvid-agent API. None = P2P mode (no hub forwarding).
    pub hub_url: Option<String>,
    /// Agent display name.
    pub agent_name: String,
    /// Agent's Algorand address (for sending replies).
    pub agent_address: String,
    /// Ed25519 signing key (for signing reply transactions).
    pub signing_key: SigningKey,
    /// Optional shared health state updated by the message loop.
    pub health_state: Option<Arc<tokio::sync::RwLock<crate::health::HealthState>>>,
}

impl Default for AgentLoopConfig {
    fn default() -> Self {
        Self {
            poll_interval_secs: 5,
            hub_url: Some("http://localhost:3578".to_string()),
            agent_name: "can".to_string(),
            agent_address: String::new(),
            signing_key: SigningKey::from_bytes(&[0u8; 32]),
            health_state: None,
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

/// Full task status from the hub (includes response text when completed).
#[derive(Debug, Deserialize)]
struct HubTaskStatus {
    state: String,
    #[serde(default)]
    response: Option<String>,
}

/// Hub task polling configuration.
const HUB_POLL_INTERVAL: Duration = Duration::from_secs(3);
const HUB_POLL_MAX_ATTEMPTS: u32 = 100; // 5 minutes at 3s intervals

/// Runs the agent message polling loop with bidirectional messaging.
///
/// Flow: poll → decrypt → forward to hub → poll for response → encrypt → send on-chain
pub async fn run_message_loop<A, I, S, M>(
    client: Arc<AlgoChat<A, I, S, M>>,
    algod: Arc<A>,
    config: AgentLoopConfig,
) where
    A: AlgodClient + Send + Sync + 'static,
    I: IndexerClient + 'static,
    S: EncryptionKeyStorage + 'static,
    M: MessageCache + 'static,
{
    let interval = Duration::from_secs(config.poll_interval_secs);
    let http = Client::new();
    let p2p_mode = config.hub_url.is_none();

    if p2p_mode {
        info!(
            name = %config.agent_name,
            poll_secs = config.poll_interval_secs,
            address = %config.agent_address,
            "starting message loop (P2P — no hub)"
        );
    } else {
        info!(
            name = %config.agent_name,
            poll_secs = config.poll_interval_secs,
            hub = config.hub_url.as_deref().unwrap_or(""),
            address = %config.agent_address,
            "starting message loop (bidirectional)"
        );
    }

    loop {
        match client.sync().await {
            Ok(messages) => {
                // Update health: algod is reachable
                if let Some(ref hs) = config.health_state {
                    let mut s = hs.write().await;
                    s.algod_connected = true;
                    if !messages.is_empty() {
                        s.last_message_at = Some(Utc::now());
                    }
                }

                for msg in &messages {
                    info!(
                        from = %msg.sender,
                        to = %msg.recipient,
                        round = msg.confirmed_round,
                        "received message: {}",
                        truncate(&msg.content, 100)
                    );

                    // P2P mode: receive and store only, no hub forwarding
                    if p2p_mode {
                        debug!(from = %msg.sender, "message stored (P2P mode)");
                        continue;
                    }

                    let hub_url = config.hub_url.as_deref().unwrap();

                    // Step 1: Forward to hub
                    let task_id =
                        match forward_to_hub(&http, hub_url, &msg.sender, &msg.content)
                            .await
                        {
                            Some(id) => id,
                            None => {
                                // Hub unreachable — send error reply on-chain
                                let error_msg = "[error] Agent hub is unreachable. Please try again later.";
                                warn!(recipient = %msg.sender, "hub unreachable — sending error reply");
                                if let Err(e) = send_reply(
                                    &client, &*algod, &config.agent_address,
                                    &msg.sender, error_msg, &config.signing_key,
                                ).await {
                                    warn!(error = %e, "failed to send hub-unreachable error reply");
                                }
                                continue;
                            }
                        };

                    // Step 2: Poll for hub response
                    let response = match poll_hub_task(&http, hub_url, &task_id).await {
                        Some(text) => text,
                        None => {
                            // Hub timed out or task failed — send error reply on-chain
                            let error_msg = "[error] Agent did not produce a response. The request may have timed out.";
                            warn!(task_id = %task_id, recipient = %msg.sender, "hub task failed — sending error reply");
                            if let Err(e) = send_reply(
                                &client, &*algod, &config.agent_address,
                                &msg.sender, error_msg, &config.signing_key,
                            ).await {
                                warn!(error = %e, "failed to send hub-timeout error reply");
                            }
                            continue;
                        }
                    };

                    info!(
                        reply_to = %msg.sender,
                        length = response.len(),
                        "sending reply: {}",
                        truncate(&response, 100)
                    );

                    // Step 3: Encrypt and send reply on-chain
                    if let Err(e) = send_reply(
                        &client,
                        &*algod,
                        &config.agent_address,
                        &msg.sender,
                        &response,
                        &config.signing_key,
                    )
                    .await
                    {
                        warn!(error = %e, recipient = %msg.sender, "failed to send on-chain reply");
                    }
                }
                if !messages.is_empty() {
                    debug!(count = messages.len(), "processed messages");
                }
            }
            Err(e) => {
                warn!(error = %e, "sync failed — will retry");
                // Update health: algod unreachable
                if let Some(ref hs) = config.health_state {
                    let mut s = hs.write().await;
                    s.algod_connected = false;
                }
            }
        }

        tokio::time::sleep(interval).await;
    }
}

/// Forward a decrypted AlgoChat message to the hub's A2A task endpoint.
///
/// Returns the task ID if the hub accepted the message, or None on failure.
async fn forward_to_hub(
    http: &Client,
    hub_url: &str,
    sender: &str,
    content: &str,
) -> Option<String> {
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
                        Some(task.id)
                    }
                    Err(e) => {
                        warn!(error = %e, "hub returned success but response parse failed");
                        None
                    }
                }
            } else {
                warn!(status = %resp.status(), "hub rejected message");
                None
            }
        }
        Err(e) => {
            warn!(error = %e, "failed to forward message to hub (hub unreachable?)");
            None
        }
    }
}

/// Poll the hub for task completion and return the response text.
///
/// Polls `GET {hub_url}/a2a/tasks/{task_id}` until the task reaches a terminal
/// state ("completed", "failed", "cancelled") or the poll limit is reached.
async fn poll_hub_task(http: &Client, hub_url: &str, task_id: &str) -> Option<String> {
    let url = format!("{}/a2a/tasks/{}", hub_url.trim_end_matches('/'), task_id);

    for attempt in 1..=HUB_POLL_MAX_ATTEMPTS {
        tokio::time::sleep(HUB_POLL_INTERVAL).await;

        match http.get(&url).send().await {
            Ok(resp) => {
                if !resp.status().is_success() {
                    debug!(
                        status = %resp.status(),
                        attempt,
                        "hub task poll returned non-success"
                    );
                    continue;
                }

                match resp.json::<HubTaskStatus>().await {
                    Ok(status) => {
                        debug!(
                            task_id = %task_id,
                            state = %status.state,
                            attempt,
                            "polled hub task"
                        );

                        match status.state.as_str() {
                            "completed" => {
                                return status.response;
                            }
                            "failed" | "cancelled" => {
                                warn!(
                                    task_id = %task_id,
                                    state = %status.state,
                                    "hub task terminated without response"
                                );
                                return None;
                            }
                            _ => {
                                // Still running, keep polling
                            }
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, attempt, "failed to parse hub task status");
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, attempt, "failed to poll hub task");
            }
        }
    }

    warn!(
        task_id = %task_id,
        max_attempts = HUB_POLL_MAX_ATTEMPTS,
        "hub task poll timed out"
    );
    None
}

/// Encrypt a message and send it on-chain via AlgoChat.
///
/// Uses PSK encryption if the recipient is a PSK contact, otherwise falls back
/// to standard X25519 encryption.
pub async fn send_reply<A, I, S, M>(
    client: &AlgoChat<A, I, S, M>,
    algod: &A,
    sender_address: &str,
    recipient_address: &str,
    message: &str,
    signing_key: &SigningKey,
) -> anyhow::Result<String>
where
    A: AlgodClient,
    I: IndexerClient,
    S: EncryptionKeyStorage,
    M: MessageCache,
{
    // Try PSK first (preferred for contacts with pre-shared keys)
    let encrypted = if client.get_psk_contact(recipient_address).await.is_some() {
        let (bytes, counter) = client
            .send_psk(recipient_address, message)
            .await
            .map_err(|e| anyhow::anyhow!("PSK encryption failed: {}", e))?;
        info!(recipient = %recipient_address, counter, "encrypted reply with PSK");
        bytes
    } else {
        // Standard X25519 encryption — need recipient's public key
        let discovered = client
            .discover_key(recipient_address)
            .await
            .map_err(|e| anyhow::anyhow!("Key discovery failed: {}", e))?
            .ok_or_else(|| anyhow::anyhow!("No encryption key found for {}", recipient_address))?;

        client
            .encrypt(message, &discovered.public_key)
            .map_err(|e| anyhow::anyhow!("Encryption failed: {}", e))?
    };

    // Submit the encrypted message as a 0-ALGO payment transaction
    let txid = transaction::send_note_transaction(
        algod,
        sender_address,
        recipient_address,
        &encrypted,
        signing_key,
    )
    .await?;

    info!(
        txid = %txid,
        recipient = %recipient_address,
        "reply sent on-chain"
    );

    Ok(txid)
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
        assert_eq!(
            config.hub_url.as_deref(),
            Some("http://localhost:3578")
        );
        assert_eq!(config.agent_name, "can");
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

    #[test]
    fn hub_task_status_deserializes_with_response() {
        let json = r#"{"state":"completed","response":"Hello from the hub!"}"#;
        let status: HubTaskStatus = serde_json::from_str(json).unwrap();
        assert_eq!(status.state, "completed");
        assert_eq!(status.response.as_deref(), Some("Hello from the hub!"));
    }

    #[test]
    fn hub_task_status_deserializes_without_response() {
        let json = r#"{"state":"running"}"#;
        let status: HubTaskStatus = serde_json::from_str(json).unwrap();
        assert_eq!(status.state, "running");
        assert!(status.response.is_none());
    }

    #[test]
    fn hub_task_status_deserializes_with_null_response() {
        let json = r#"{"state":"failed","response":null}"#;
        let status: HubTaskStatus = serde_json::from_str(json).unwrap();
        assert_eq!(status.state, "failed");
        assert!(status.response.is_none());
    }
}
