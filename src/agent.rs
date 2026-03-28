//! Agent message loop — polls for AlgoChat messages, forwards to hub, sends replies.

use std::sync::Arc;
use std::time::Duration;

use algochat::{AlgoChat, AlgodClient, EncryptionKeyStorage, IndexerClient, MessageCache};
use ed25519_dalek::SigningKey;
use tracing::{debug, error, info, warn};

use crate::hub::HubClient;
use crate::transaction;

/// Configuration for the agent message loop.
pub struct AgentLoopConfig {
    /// How often to poll for new messages (seconds).
    pub poll_interval_secs: u64,
    /// Hub URL for corvid-agent API.
    pub hub_url: String,
    /// Agent display name.
    pub agent_name: String,
    /// This agent's Algorand address.
    pub address: String,
}

impl Default for AgentLoopConfig {
    fn default() -> Self {
        Self {
            poll_interval_secs: 5,
            hub_url: "http://localhost:3578".to_string(),
            agent_name: "nano".to_string(),
            address: String::new(),
        }
    }
}

/// Runs the agent message polling loop.
///
/// Flow: poll AlgoChat → decrypt messages → forward to hub → encrypt response → send on-chain.
pub async fn run_message_loop<A, I, S, M>(
    client: Arc<AlgoChat<A, I, S, M>>,
    algod: Arc<A>,
    signing_key: SigningKey,
    hub: Arc<HubClient>,
    config: AgentLoopConfig,
) where
    A: AlgodClient + 'static,
    I: IndexerClient + 'static,
    S: EncryptionKeyStorage + 'static,
    M: MessageCache + 'static,
{
    let interval = Duration::from_secs(config.poll_interval_secs);
    let heartbeat_interval = Duration::from_secs(60);
    let mut last_heartbeat = std::time::Instant::now();

    info!(
        name = %config.agent_name,
        poll_secs = config.poll_interval_secs,
        hub = %config.hub_url,
        "starting message loop"
    );

    loop {
        // Send periodic heartbeat
        if last_heartbeat.elapsed() >= heartbeat_interval {
            if let Err(e) = hub.heartbeat().await {
                warn!(error = %e, "heartbeat failed");
            }
            last_heartbeat = std::time::Instant::now();
        }

        // Poll for new messages
        match client.sync().await {
            Ok(messages) => {
                for msg in &messages {
                    // Skip our own sent messages
                    if msg.sender == config.address {
                        debug!(to = %msg.recipient, "skipping own sent message");
                        continue;
                    }

                    info!(
                        from = %msg.sender,
                        round = msg.confirmed_round,
                        "received: {}",
                        truncate(&msg.content, 100)
                    );

                    // Forward to hub and get response
                    let response = match hub.forward_message(&msg.content, &msg.sender).await {
                        Ok(resp) => resp,
                        Err(e) => {
                            error!(error = %e, from = %msg.sender, "hub forwarding failed");
                            continue;
                        }
                    };

                    info!(
                        to = %msg.sender,
                        response_len = response.len(),
                        "sending reply"
                    );

                    // Encrypt and send the response back on-chain
                    if let Err(e) = send_reply(
                        &client,
                        algod.as_ref(),
                        &signing_key,
                        &config.address,
                        &msg.sender,
                        &response,
                    )
                    .await
                    {
                        error!(error = %e, to = %msg.sender, "failed to send reply");
                    }
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

/// Encrypts a response and sends it back to the sender on-chain.
async fn send_reply<A, I, S, M>(
    client: &AlgoChat<A, I, S, M>,
    algod: &dyn AlgodClient,
    signing_key: &SigningKey,
    our_address: &str,
    recipient_address: &str,
    message: &str,
) -> anyhow::Result<()>
where
    A: AlgodClient,
    I: IndexerClient,
    S: EncryptionKeyStorage,
    M: MessageCache,
{
    // Discover recipient's encryption key
    let recipient_key = client
        .discover_key(recipient_address)
        .await
        .map_err(|e| anyhow::anyhow!("Key discovery failed: {}", e))?
        .ok_or_else(|| anyhow::anyhow!("No encryption key found for {}", recipient_address))?;

    // Encrypt the message
    let encrypted = client
        .encrypt(message, &recipient_key.public_key)
        .map_err(|e| anyhow::anyhow!("Encryption failed: {}", e))?;

    // Build and submit the transaction
    let txid = transaction::send_note_transaction(
        algod,
        signing_key,
        our_address,
        recipient_address,
        &encrypted,
    )
    .await
    .map_err(|e| anyhow::anyhow!("Transaction failed: {}", e))?;

    info!(txid = %txid, to = %recipient_address, "reply sent on-chain");
    Ok(())
}

/// Truncate a string for logging.
fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}
