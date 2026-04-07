//! AlgoChat transport adapter — wraps the algochat crate into the Transport trait.

use std::sync::Arc;

use algochat::{AlgoChat, AlgodClient, EncryptionKeyStorage, IndexerClient, MessageCache};
use anyhow::Result;
use async_trait::async_trait;
use ed25519_dalek::SigningKey;
use nano_transport::{Message, OutboundMessage, SendResult, Transport};
use tracing::info;

/// AlgoChat transport — polls for encrypted messages on Algorand and sends replies.
pub struct AlgoChatTransport<
    A: AlgodClient,
    I: IndexerClient,
    S: EncryptionKeyStorage,
    M: MessageCache,
> {
    client: Arc<AlgoChat<A, I, S, M>>,
    algod: Arc<A>,
    address: String,
    signing_key: SigningKey,
}

impl<A, I, S, M> AlgoChatTransport<A, I, S, M>
where
    A: AlgodClient + Send + Sync + 'static,
    I: IndexerClient + Send + Sync + 'static,
    S: EncryptionKeyStorage + Send + Sync + 'static,
    M: MessageCache + Send + Sync + 'static,
{
    pub fn new(
        client: Arc<AlgoChat<A, I, S, M>>,
        algod: Arc<A>,
        address: String,
        signing_key: SigningKey,
    ) -> Self {
        Self {
            client,
            algod,
            address,
            signing_key,
        }
    }
}

#[async_trait]
impl<A, I, S, M> Transport for AlgoChatTransport<A, I, S, M>
where
    A: AlgodClient + Send + Sync + 'static,
    I: IndexerClient + Send + Sync + 'static,
    S: EncryptionKeyStorage + Send + Sync + 'static,
    M: MessageCache + Send + Sync + 'static,
{
    fn name(&self) -> &str {
        "algochat"
    }

    async fn recv(&self) -> Result<Vec<Message>> {
        let messages = self
            .client
            .sync()
            .await
            .map_err(|e| anyhow::anyhow!("AlgoChat sync failed: {}", e))?;

        Ok(messages
            .into_iter()
            .map(|msg| Message {
                sender: msg.sender,
                recipient: msg.recipient,
                content: msg.content,
                timestamp: chrono::Utc::now(),
                metadata: serde_json::json!({
                    "confirmed_round": msg.confirmed_round,
                }),
            })
            .collect())
    }

    async fn send(&self, msg: OutboundMessage) -> Result<SendResult> {
        let txid = crate::agent::send_reply(
            &self.client,
            &*self.algod,
            &self.address,
            &msg.to,
            &msg.content,
            &self.signing_key,
        )
        .await?;

        info!(to = %msg.to, txid = %txid, "sent via AlgoChat");
        Ok(SendResult { id: txid })
    }

    fn local_address(&self) -> &str {
        &self.address
    }
}
