# Custom Transport

The transport abstraction lets you swap out how messages are sent and received without changing any plugin code. The default transport is AlgoChat (encrypted on-chain messaging via Algorand), but you can implement your own.

## The Transport Trait

```rust
use anyhow::Result;
use async_trait::async_trait;
use nano_transport::{Message, OutboundMessage, SendResult, Transport};

#[async_trait]
pub trait Transport: Send + Sync {
    /// Human-readable name (e.g. "algochat", "websocket", "mqtt").
    fn name(&self) -> &str;

    /// Poll for new messages since the last sync.
    /// Return an empty vec if there are no new messages.
    async fn recv(&self) -> Result<Vec<Message>>;

    /// Send a message through this transport.
    async fn send(&self, msg: OutboundMessage) -> Result<SendResult>;

    /// The local agent's address on this transport.
    fn local_address(&self) -> &str;
}
```

## Message Types

**Inbound messages** use the `Message` struct:

```rust
pub struct Message {
    pub sender: String,           // Who sent it
    pub recipient: String,        // Who it's for (your agent)
    pub content: String,          // Plaintext content (already decrypted)
    pub timestamp: DateTime<Utc>, // When received/confirmed
    pub metadata: serde_json::Value, // Transport-specific extras
}
```

**Outbound messages** use `OutboundMessage`:

```rust
pub struct OutboundMessage {
    pub to: String,      // Recipient address
    pub content: String, // Plaintext (transport handles encryption)
}
```

**Send results** return an ID:

```rust
pub struct SendResult {
    pub id: String, // Transport-assigned ID (tx hash, message ID, etc.)
}
```

## Example: WebSocket Transport

Here's a complete example of a WebSocket-based transport:

```rust
use std::sync::Arc;
use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::Mutex;
use nano_transport::{Message, OutboundMessage, SendResult, Transport};

pub struct WebSocketTransport {
    address: String,
    ws_url: String,
    inbox: Arc<Mutex<Vec<Message>>>,
}

impl WebSocketTransport {
    pub fn new(address: String, ws_url: String) -> Self {
        Self {
            address,
            ws_url,
            inbox: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Spawn a background listener that pushes messages into the inbox.
    pub async fn connect(&self) -> Result<()> {
        let inbox = self.inbox.clone();
        let url = self.ws_url.clone();
        let address = self.address.clone();

        tokio::spawn(async move {
            // Connect to WebSocket and push incoming messages to inbox
            // (implementation depends on your WS library)
            let (mut ws, _) = tokio_tungstenite::connect_async(&url)
                .await
                .expect("ws connect failed");

            use futures_util::StreamExt;
            while let Some(Ok(msg)) = ws.next().await {
                if let Ok(text) = msg.into_text() {
                    inbox.lock().await.push(Message {
                        sender: "ws-peer".into(),
                        recipient: address.clone(),
                        content: text,
                        timestamp: chrono::Utc::now(),
                        metadata: serde_json::Value::Null,
                    });
                }
            }
        });

        Ok(())
    }
}

#[async_trait]
impl Transport for WebSocketTransport {
    fn name(&self) -> &str {
        "websocket"
    }

    async fn recv(&self) -> Result<Vec<Message>> {
        let mut inbox = self.inbox.lock().await;
        let messages = inbox.drain(..).collect();
        Ok(messages)
    }

    async fn send(&self, msg: OutboundMessage) -> Result<SendResult> {
        // Send via WebSocket connection
        // (simplified — real impl would hold a write handle)
        Ok(SendResult {
            id: format!("ws-{}", uuid::Uuid::new_v4()),
        })
    }

    fn local_address(&self) -> &str {
        &self.address
    }
}
```

## Example: HTTP Polling Transport

A simpler transport that polls an HTTP endpoint:

```rust
pub struct HttpTransport {
    address: String,
    poll_url: String,
    send_url: String,
    http: reqwest::Client,
    last_seen: Arc<Mutex<Option<String>>>,
}

#[async_trait]
impl Transport for HttpTransport {
    fn name(&self) -> &str {
        "http"
    }

    async fn recv(&self) -> Result<Vec<Message>> {
        let mut url = self.poll_url.clone();
        if let Some(cursor) = self.last_seen.lock().await.as_ref() {
            url = format!("{}?after={}", url, cursor);
        }

        let resp: Vec<Message> = self.http
            .get(&url)
            .send().await?
            .json().await?;

        if let Some(last) = resp.last() {
            *self.last_seen.lock().await = Some(
                last.metadata["id"].as_str().unwrap_or("").to_string()
            );
        }

        Ok(resp)
    }

    async fn send(&self, msg: OutboundMessage) -> Result<SendResult> {
        let resp = self.http
            .post(&self.send_url)
            .json(&msg)
            .send().await?;

        let id = resp.json::<serde_json::Value>().await?
            ["id"].as_str().unwrap_or("unknown").to_string();

        Ok(SendResult { id })
    }

    fn local_address(&self) -> &str {
        &self.address
    }
}
```

## Built-in Transports

### NullTransport

A no-op transport for offline mode and testing. `recv()` always returns empty, `send()` always succeeds.

```rust
use nano_transport::NullTransport;

let transport = NullTransport::new("my-address");
```

### MockTransport

A test transport that lets you inject messages and capture outbound ones:

```rust
use nano_transport::MockTransport;

let transport = MockTransport::new("test-agent");

// Inject a message that recv() will return
transport.inject(transport.message_from("alice", "hello"));

// After running, check what was sent
let sent = transport.sent_messages();
assert_eq!(sent[0].to, "alice");

// Inject multiple messages at once
transport.inject_many(vec![
    transport.message_from("bob", "msg 1"),
    transport.message_from("charlie", "msg 2"),
]);

// Clear captured messages
transport.clear_sent();

// Check total send count (persists across clears)
assert_eq!(transport.send_count(), 1);
```

`MockTransport::clone()` shares state — both clones see the same inbox and outbox. This is useful for passing one clone to the runtime and keeping another for assertions.

## Using Your Transport

Pass your transport to the runtime:

```rust
use std::sync::Arc;
use nano_runtime::{Runtime, RuntimeConfig};

let transport = Arc::new(WebSocketTransport::new(
    "my-agent".into(),
    "ws://localhost:8080".into(),
));
transport.connect().await?;

let mut runtime = Runtime::new(transport, RuntimeConfig::default());
runtime.add_plugin(Box::new(MyPlugin)).await?;

let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
runtime.run(shutdown_rx).await?;
```

The runtime calls `transport.recv()` on every poll tick (default: every 5 seconds, configurable via `RuntimeConfig::poll_interval_secs`). Each returned message becomes a `MessageReceived` event dispatched to plugins.

## Design Guidelines

1. **`recv()` should drain** — return all pending messages and clear them. The runtime calls `recv()` on a timer; returning the same messages twice will cause duplicate processing.

2. **`send()` should be idempotent-safe** — if the runtime retries a failed send, your transport should handle it gracefully.

3. **Use `metadata` for transport-specific data** — round numbers, transaction hashes, channel IDs, etc. Plugins can read `msg.metadata` for transport-specific context without the transport leaking into the core API.

4. **Handle errors gracefully** — the runtime logs transport errors and continues. Don't panic in `recv()` or `send()`.

## Next Steps

- [Nano Runtime](./nano-runtime.md) — the event-driven plugin system
- [Examples](./examples.md) — complete worked examples
- [Architecture Overview](../architecture/overview.md) — system design
