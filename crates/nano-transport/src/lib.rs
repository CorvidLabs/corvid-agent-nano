//! Transport abstraction for corvid-agent-nano.
//!
//! Defines the [`Transport`] trait that messaging backends implement,
//! plus the common [`Message`] type that flows through the system.

use std::fmt;

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// An inbound or outbound message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Sender address (e.g. Algorand address).
    pub sender: String,
    /// Recipient address.
    pub recipient: String,
    /// Plaintext content (already decrypted for inbound).
    pub content: String,
    /// When the message was confirmed/received.
    pub timestamp: DateTime<Utc>,
    /// Transport-specific metadata (round number, tx ID, etc.).
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// An outbound message to send.
#[derive(Debug, Clone)]
pub struct OutboundMessage {
    /// Recipient address.
    pub to: String,
    /// Plaintext content (will be encrypted by the transport).
    pub content: String,
}

/// Result of sending a message.
#[derive(Debug, Clone)]
pub struct SendResult {
    /// Transport-assigned ID (e.g. transaction ID).
    pub id: String,
}

/// The transport trait — implemented by AlgoChat, and potentially others.
///
/// A transport knows how to poll for inbound messages and send outbound ones.
#[async_trait]
pub trait Transport: Send + Sync {
    /// Human-readable name for this transport (e.g. "algochat").
    fn name(&self) -> &str;

    /// Poll for new messages since the last sync.
    /// Returns an empty vec if no new messages.
    async fn recv(&self) -> Result<Vec<Message>>;

    /// Send a message through this transport.
    async fn send(&self, msg: OutboundMessage) -> Result<SendResult>;

    /// The local agent's address on this transport.
    fn local_address(&self) -> &str;
}

/// A no-op transport for testing and offline mode.
pub struct NullTransport {
    address: String,
}

impl NullTransport {
    pub fn new(address: impl Into<String>) -> Self {
        Self {
            address: address.into(),
        }
    }
}

#[async_trait]
impl Transport for NullTransport {
    fn name(&self) -> &str {
        "null"
    }

    async fn recv(&self) -> Result<Vec<Message>> {
        Ok(vec![])
    }

    async fn send(&self, _msg: OutboundMessage) -> Result<SendResult> {
        Ok(SendResult {
            id: "null-0".to_string(),
        })
    }

    fn local_address(&self) -> &str {
        &self.address
    }
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] {} -> {}: {}",
            self.timestamp.format("%H:%M:%S"),
            truncate(&self.sender, 8),
            truncate(&self.recipient, 8),
            truncate(&self.content, 60),
        )
    }
}

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
    fn message_display_format() {
        let msg = Message {
            sender: "ABCDEFGHIJKLMNOP".to_string(),
            recipient: "ZYXWVUTS".to_string(),
            content: "hello world".to_string(),
            timestamp: DateTime::from_timestamp(1700000000, 0).unwrap(),
            metadata: serde_json::Value::Null,
        };
        let display = format!("{}", msg);
        assert!(display.contains("ABCDEFGH"));
        assert!(display.contains("ZYXWVUTS"));
        assert!(display.contains("hello world"));
    }

    #[test]
    fn message_serialization_roundtrip() {
        let msg = Message {
            sender: "alice".to_string(),
            recipient: "bob".to_string(),
            content: "test".to_string(),
            timestamp: Utc::now(),
            metadata: serde_json::json!({"round": 42}),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(back.sender, "alice");
        assert_eq!(back.recipient, "bob");
        assert_eq!(back.content, "test");
        assert_eq!(back.metadata["round"], 42);
    }

    #[test]
    fn outbound_message_construction() {
        let msg = OutboundMessage {
            to: "bob".to_string(),
            content: "hello".to_string(),
        };
        assert_eq!(msg.to, "bob");
        assert_eq!(msg.content, "hello");
    }

    #[tokio::test]
    async fn null_transport_recv_returns_empty() {
        let transport = NullTransport::new("test-addr");
        let msgs = transport.recv().await.unwrap();
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn null_transport_send_returns_ok() {
        let transport = NullTransport::new("test-addr");
        let result = transport
            .send(OutboundMessage {
                to: "bob".into(),
                content: "hi".into(),
            })
            .await
            .unwrap();
        assert_eq!(result.id, "null-0");
    }

    #[test]
    fn null_transport_address() {
        let transport = NullTransport::new("my-address");
        assert_eq!(transport.local_address(), "my-address");
        assert_eq!(transport.name(), "null");
    }
}
