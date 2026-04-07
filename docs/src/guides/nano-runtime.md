# Nano Runtime (Native Plugins)

The **nano-runtime** is an event-driven plugin system for building native Rust plugins that run inside the agent process. Unlike WASM plugins (which run in a sandboxed sidecar), native plugins have full access to the Rust ecosystem and can use async I/O, HTTP clients, and more.

## Overview

The runtime coordinates three things:

1. **Transport** — polls for inbound messages, sends outbound ones
2. **Plugins** — receive events, return actions
3. **State** — scoped key-value storage per plugin

Plugins never mutate state directly. Instead, they return **actions** (like `SendMessage` or `StoreState`) and the runtime executes them. This keeps everything safe and auditable.

## Architecture

```
┌─────────────────────────────────────────────┐
│                  Runtime                     │
│                                              │
│  ┌───────────┐    ┌──────────────────────┐  │
│  │ Transport  │───▶│     Event Loop       │  │
│  │ (AlgoChat) │    │                      │  │
│  └───────────┘    │  poll ─▶ dispatch ─▶  │  │
│                   │  execute actions       │  │
│  ┌───────────┐    │                      │  │
│  │ Event Bus │───▶│  (internal events)   │  │
│  └───────────┘    └──────────────────────┘  │
│                          │                   │
│         ┌────────────────┼────────────┐      │
│         ▼                ▼            ▼      │
│   ┌──────────┐    ┌──────────┐  ┌─────────┐ │
│   │   Hub    │    │AutoReply │  │  Your   │ │
│   │  Plugin  │    │ Plugin   │  │ Plugin  │ │
│   └──────────┘    └──────────┘  └─────────┘ │
│                                              │
│   ┌──────────────────────────────────────┐  │
│   │          StateStore (per-plugin)      │  │
│   └──────────────────────────────────────┘  │
└─────────────────────────────────────────────┘
```

## The Plugin Trait

Every native plugin implements the `Plugin` trait:

```rust
use anyhow::Result;
use async_trait::async_trait;
use nano_runtime::{Action, Event, EventKind, Plugin, PluginContext};

pub struct MyPlugin;

#[async_trait]
impl Plugin for MyPlugin {
    /// Unique plugin name.
    fn name(&self) -> &str {
        "my-plugin"
    }

    /// Semver version string.
    fn version(&self) -> &str {
        "0.1.0"
    }

    /// Called once at startup. Use for one-time setup.
    async fn init(&mut self, ctx: &PluginContext) -> Result<()> {
        // Read config, set up state, etc.
        Ok(())
    }

    /// Handle an event and return zero or more actions.
    async fn handle_event(
        &self,
        event: &Event,
        ctx: &PluginContext,
    ) -> Result<Vec<Action>> {
        Ok(vec![])
    }

    /// Which events this plugin cares about.
    fn subscriptions(&self) -> Vec<EventKind> {
        vec![EventKind::MessageReceived]
    }

    /// Called on graceful shutdown (optional).
    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}
```

## Events

Events flow through the runtime and are dispatched to subscribed plugins:

| Event | Triggered when | Typical use |
|-------|---------------|-------------|
| `MessageReceived(msg)` | A new message arrives from the transport | Reply, forward, log |
| `MessageSent { to, tx_id }` | An outbound message is confirmed sent | Audit trail, receipts |
| `ContactAdded { address, name }` | A contact is added | Welcome message |
| `ContactRemoved { address }` | A contact is removed | Cleanup |
| `PluginLoaded { name }` | A plugin finishes loading | Inter-plugin coordination |
| `PluginUnloaded { name }` | A plugin is removed | Cleanup |
| `Timer { timestamp }` | Periodic tick | Scheduled tasks |
| `Shutdown` | Graceful shutdown starting | Save state, flush buffers |
| `Custom { kind, data }` | Emitted by another plugin | Plugin-to-plugin messaging |

### Subscribing to Events

Return the event kinds you care about from `subscriptions()`:

```rust
fn subscriptions(&self) -> Vec<EventKind> {
    vec![
        EventKind::MessageReceived,
        EventKind::MessageSent,
        EventKind::Custom("my-event".to_string()),
    ]
}
```

Use `EventKind::All` to receive every event (useful for logging/monitoring plugins).

## Actions

Plugins return actions to request the runtime to do things on their behalf:

```rust
pub enum Action {
    /// Send a message through the transport.
    SendMessage { to: String, content: String },

    /// Persist a key-value pair in the plugin's scoped state.
    StoreState { key: String, value: serde_json::Value },

    /// Emit a custom event into the event bus.
    EmitEvent { kind: String, data: serde_json::Value },

    /// Structured log entry.
    Log { level: LogLevel, message: String },
}
```

Return multiple actions from a single event handler:

```rust
async fn handle_event(&self, event: &Event, ctx: &PluginContext) -> Result<Vec<Action>> {
    match event {
        Event::MessageReceived(msg) => Ok(vec![
            Action::SendMessage {
                to: msg.sender.clone(),
                content: "Got it!".into(),
            },
            Action::StoreState {
                key: "last_sender".into(),
                value: serde_json::json!(msg.sender),
            },
            Action::Log {
                level: LogLevel::Info,
                message: format!("Replied to {}", msg.sender),
            },
        ]),
        _ => Ok(vec![]),
    }
}
```

## PluginContext

The `PluginContext` is a read-only snapshot passed to every event handler:

```rust
pub struct PluginContext {
    /// The agent's address on the transport (e.g. Algorand address).
    pub agent_address: String,
    /// The agent's display name.
    pub agent_name: String,
    /// Plugin-scoped state (read-only snapshot from StateStore).
    pub state: HashMap<String, serde_json::Value>,
    /// Plugin-specific config from nano.toml [plugins.<name>].
    pub config: toml::Table,
}
```

### Reading State

State is scoped per plugin. Read from the snapshot in `ctx.state`:

```rust
async fn handle_event(&self, event: &Event, ctx: &PluginContext) -> Result<Vec<Action>> {
    let count = ctx.state
        .get("message_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    // Increment and store
    Ok(vec![Action::StoreState {
        key: "message_count".into(),
        value: serde_json::json!(count + 1),
    }])
}
```

### Reading Config

Plugin config comes from `nano.toml`:

```toml
[plugins.my-plugin]
api_key = "abc123"
max_retries = 3
```

Access it in your plugin:

```rust
async fn init(&mut self, ctx: &PluginContext) -> Result<()> {
    let api_key = ctx.config
        .get("api_key")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let max_retries = ctx.config
        .get("max_retries")
        .and_then(|v| v.as_integer())
        .unwrap_or(5);

    // Store for later use
    Ok(())
}
```

## Registering Plugins

Add your plugin to the runtime at startup:

```rust
use std::sync::Arc;
use nano_runtime::{Runtime, RuntimeConfig};
use nano_transport::AlgoChatTransport; // or any Transport impl

let transport = Arc::new(AlgoChatTransport::new(/* ... */));
let config = RuntimeConfig {
    poll_interval_secs: 5,
    agent_name: "my-agent".into(),
    plugin_configs: HashMap::new(),
};

let mut runtime = Runtime::new(transport, config);

// Register plugins
runtime.add_plugin(Box::new(MyPlugin)).await?;
runtime.add_plugin(Box::new(AutoReplyPlugin::new())).await?;

// Run until shutdown
let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
runtime.run(shutdown_rx).await?;
```

## Built-in Plugins

### Hub Plugin

Forwards messages to a corvid-agent-server and relays responses:

```toml
[plugins.hub]
url = "http://localhost:3578"
```

The hub plugin:
1. Receives incoming AlgoChat messages
2. Forwards them to the hub's A2A task endpoint
3. Polls for the AI-generated response
4. Sends the response back through the transport

### Auto-Reply Plugin

Pattern-matching responder for when no AI hub is connected:

```toml
[plugins.auto-reply]
rules = [
    { match = "ping", reply = "pong" },
    { match = "status", reply = "online and operational" },
    { match = "help", reply = "Available commands: ping, status, help" },
]
```

Rules are matched case-insensitively as substrings. First match wins.

## Plugin-to-Plugin Communication

Plugins can communicate through custom events:

**Plugin A** emits a custom event:
```rust
Ok(vec![Action::EmitEvent {
    kind: "price-update".into(),
    data: serde_json::json!({ "asset": "ALGO", "price": 0.42 }),
}])
```

**Plugin B** subscribes and reacts:
```rust
fn subscriptions(&self) -> Vec<EventKind> {
    vec![EventKind::Custom("price-update".to_string())]
}

async fn handle_event(&self, event: &Event, _ctx: &PluginContext) -> Result<Vec<Action>> {
    match event {
        Event::Custom { kind, data } if kind == "price-update" => {
            let price = data["price"].as_f64().unwrap_or(0.0);
            if price > 1.0 {
                Ok(vec![Action::SendMessage {
                    to: "admin".into(),
                    content: format!("ALGO price alert: ${}", price),
                }])
            } else {
                Ok(vec![])
            }
        }
        _ => Ok(vec![]),
    }
}
```

## Testing Plugins

Use `MockTransport` for deterministic testing without a real blockchain:

```rust
use std::sync::Arc;
use nano_runtime::{Runtime, RuntimeConfig};
use nano_transport::MockTransport;

#[tokio::test]
async fn test_my_plugin() {
    let transport = Arc::new(MockTransport::new("test-agent"));
    let mut runtime = Runtime::new(transport.clone(), RuntimeConfig::default());

    runtime.add_plugin(Box::new(MyPlugin)).await.unwrap();

    // Inject a message
    transport.inject(transport.message_from("alice", "hello"));

    // Run briefly then shut down
    let (tx, rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let _ = tx.send(true);
    });
    runtime.run(rx).await.unwrap();

    // Verify what was sent
    let sent = transport.sent_messages();
    assert!(!sent.is_empty());
    assert_eq!(sent[0].to, "alice");
}
```

See `tests/e2e_runtime.rs` for 31 comprehensive examples.

## Native vs WASM Plugins

| | Native (nano-runtime) | WASM (plugin-host) |
|---|---|---|
| **Language** | Rust only | Any language targeting WASM |
| **Sandboxing** | None (runs in-process) | Full sandbox (memory, CPU, network) |
| **Performance** | Native speed, zero overhead | Small overhead from WASM runtime |
| **Capabilities** | Full Rust ecosystem | Limited to declared capabilities |
| **Hot-reload** | Restart required | Hot-reload with drain pattern |
| **Use case** | Trusted first-party plugins | Third-party/untrusted plugins |
| **Testing** | Standard `cargo test` | Build to WASM, load in host |

Choose **native plugins** for core agent behavior (hub forwarding, auto-reply, monitoring). Choose **WASM plugins** for third-party extensions where sandboxing matters.

## Next Steps

- [Custom Transport](./custom-transport.md) — implement your own transport backend
- [Examples](./examples.md) — complete worked examples
- [Plugin Development (WASM)](./plugin-development.md) — sandboxed WASM plugins
- [Architecture Overview](../architecture/overview.md) — how it all fits together
