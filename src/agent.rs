//! Agent message loop — polls for AlgoChat messages and processes them.

use std::sync::Arc;
use std::time::Duration;

use algochat::{AlgoChat, AlgodClient, IndexerClient, EncryptionKeyStorage, MessageCache};
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

/// Runs the agent message polling loop.
///
/// This continuously polls the Algorand indexer for new AlgoChat messages,
/// decrypts them, and logs them. In the future this will forward messages
/// to the hub or invoke local processing.
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

                    // TODO: Forward to hub API or process locally
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

/// Truncate a string for logging.
fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}
