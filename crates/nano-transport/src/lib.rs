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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// A test transport that allows injecting inbound messages and capturing outbound ones.
///
/// Use this for integration/e2e tests where you need to simulate message flow.
pub struct MockTransport {
    address: String,
    inbound: std::sync::Arc<std::sync::Mutex<Vec<Message>>>,
    outbound: std::sync::Arc<std::sync::Mutex<Vec<OutboundMessage>>>,
    send_counter: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl MockTransport {
    /// Create a new mock transport with the given local address.
    pub fn new(address: impl Into<String>) -> Self {
        Self {
            address: address.into(),
            inbound: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            outbound: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            send_counter: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    /// Queue a message to be returned by the next `recv()` call.
    pub fn inject(&self, msg: Message) {
        self.inbound.lock().unwrap().push(msg);
    }

    /// Queue multiple messages to be returned by the next `recv()` call.
    pub fn inject_many(&self, msgs: Vec<Message>) {
        self.inbound.lock().unwrap().extend(msgs);
    }

    /// Get all outbound messages that were sent through this transport.
    pub fn sent_messages(&self) -> Vec<OutboundMessage> {
        self.outbound.lock().unwrap().clone()
    }

    /// Get the number of messages sent through this transport.
    pub fn send_count(&self) -> u64 {
        self.send_counter
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Clear captured outbound messages.
    pub fn clear_sent(&self) {
        self.outbound.lock().unwrap().clear();
    }

    /// Create a message from a sender to this transport's address.
    pub fn message_from(&self, sender: &str, content: &str) -> Message {
        Message {
            sender: sender.to_string(),
            recipient: self.address.clone(),
            content: content.to_string(),
            timestamp: chrono::Utc::now(),
            metadata: serde_json::Value::Null,
        }
    }
}

impl Clone for MockTransport {
    fn clone(&self) -> Self {
        Self {
            address: self.address.clone(),
            inbound: self.inbound.clone(),
            outbound: self.outbound.clone(),
            send_counter: self.send_counter.clone(),
        }
    }
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

#[async_trait]
impl Transport for MockTransport {
    fn name(&self) -> &str {
        "mock"
    }

    async fn recv(&self) -> Result<Vec<Message>> {
        let mut inbox = self.inbound.lock().unwrap();
        let messages = inbox.drain(..).collect();
        Ok(messages)
    }

    async fn send(&self, msg: OutboundMessage) -> Result<SendResult> {
        let id = self
            .send_counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.outbound.lock().unwrap().push(msg);
        Ok(SendResult {
            id: format!("mock-{}", id),
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

    #[tokio::test]
    async fn mock_transport_recv_empty_by_default() {
        let transport = MockTransport::new("test-addr");
        let msgs = transport.recv().await.unwrap();
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn mock_transport_inject_and_recv() {
        let transport = MockTransport::new("test-addr");
        transport.inject(Message {
            sender: "alice".into(),
            recipient: "test-addr".into(),
            content: "hello".into(),
            timestamp: Utc::now(),
            metadata: serde_json::Value::Null,
        });

        let msgs = transport.recv().await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "hello");

        // Second recv should be empty (messages are drained)
        let msgs = transport.recv().await.unwrap();
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn mock_transport_captures_sent_messages() {
        let transport = MockTransport::new("test-addr");
        transport
            .send(OutboundMessage {
                to: "bob".into(),
                content: "hi bob".into(),
            })
            .await
            .unwrap();
        transport
            .send(OutboundMessage {
                to: "charlie".into(),
                content: "hi charlie".into(),
            })
            .await
            .unwrap();

        assert_eq!(transport.send_count(), 2);
        let sent = transport.sent_messages();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].to, "bob");
        assert_eq!(sent[1].to, "charlie");
    }

    #[tokio::test]
    async fn mock_transport_message_from_helper() {
        let transport = MockTransport::new("my-agent");
        let msg = transport.message_from("alice", "test content");
        assert_eq!(msg.sender, "alice");
        assert_eq!(msg.recipient, "my-agent");
        assert_eq!(msg.content, "test content");
    }

    #[tokio::test]
    async fn mock_transport_clear_sent() {
        let transport = MockTransport::new("test-addr");
        transport
            .send(OutboundMessage {
                to: "bob".into(),
                content: "hi".into(),
            })
            .await
            .unwrap();
        assert_eq!(transport.sent_messages().len(), 1);
        transport.clear_sent();
        assert!(transport.sent_messages().is_empty());
        // Counter persists
        assert_eq!(transport.send_count(), 1);
    }

    #[test]
    fn mock_transport_clone_shares_state() {
        let t1 = MockTransport::new("addr");
        let t2 = t1.clone();
        t1.inject(Message {
            sender: "alice".into(),
            recipient: "addr".into(),
            content: "shared".into(),
            timestamp: Utc::now(),
            metadata: serde_json::Value::Null,
        });
        // t2 should see the injected message
        let inbox = t2.inbound.lock().unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].content, "shared");
    }
}
