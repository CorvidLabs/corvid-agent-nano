# Examples & Demos

This page contains complete, runnable examples demonstrating common use cases for corvid-agent-nano.

## Example 1: Echo Bot

The simplest possible agent — echoes back every message it receives.

### Plugin Code

```rust
use anyhow::Result;
use async_trait::async_trait;
use nano_runtime::{Action, Event, EventKind, Plugin, PluginContext};

pub struct EchoPlugin;

#[async_trait]
impl Plugin for EchoPlugin {
    fn name(&self) -> &str { "echo" }
    fn version(&self) -> &str { "1.0.0" }

    async fn init(&mut self, _ctx: &PluginContext) -> Result<()> {
        Ok(())
    }

    async fn handle_event(
        &self,
        event: &Event,
        _ctx: &PluginContext,
    ) -> Result<Vec<Action>> {
        match event {
            Event::MessageReceived(msg) => Ok(vec![Action::SendMessage {
                to: msg.sender.clone(),
                content: format!("echo: {}", msg.content),
            }]),
            _ => Ok(vec![]),
        }
    }

    fn subscriptions(&self) -> Vec<EventKind> {
        vec![EventKind::MessageReceived]
    }
}
```

### Running It

```rust
use std::sync::Arc;
use nano_runtime::{Runtime, RuntimeConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let transport = Arc::new(/* your transport */);
    let mut runtime = Runtime::new(transport, RuntimeConfig::default());
    runtime.add_plugin(Box::new(EchoPlugin)).await?;

    let (_tx, rx) = tokio::sync::watch::channel(false);
    runtime.run(rx).await
}
```

### Testing It

```rust
#[tokio::test]
async fn echo_replies_to_sender() {
    use std::sync::Arc;
    use nano_runtime::{Runtime, RuntimeConfig};
    use nano_transport::MockTransport;

    let transport = Arc::new(MockTransport::new("echo-agent"));
    let mut runtime = Runtime::new(transport.clone(), RuntimeConfig::default());
    runtime.add_plugin(Box::new(EchoPlugin)).await.unwrap();

    // Inject a message
    transport.inject(transport.message_from("alice", "hello"));

    // Run briefly
    let (tx, rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let _ = tx.send(true);
    });
    runtime.run(rx).await.unwrap();

    // Verify the reply
    let sent = transport.sent_messages();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].to, "alice");
    assert_eq!(sent[0].content, "echo: hello");
}
```

---

## Example 2: Auto-Reply Bot with Config

A configurable keyword responder using `nano.toml` config.

### nano.toml

```toml
[agent]
name = "support-bot"
network = "testnet"

[plugins.auto-reply]
rules = [
    { match = "ping", reply = "pong" },
    { match = "hours", reply = "We're available 9am-5pm UTC, Monday-Friday." },
    { match = "help", reply = "Commands: ping, hours, help, status" },
    { match = "status", reply = "All systems operational." },
]
```

### Running It

```rust
use nano_runtime::plugins::auto_reply::AutoReplyPlugin;

let mut runtime = Runtime::new(transport, config);
runtime.add_plugin(Box::new(AutoReplyPlugin::new())).await?;
```

The auto-reply plugin reads its rules from `ctx.config` during `init()`. Rules match case-insensitively as substrings — "what are your hours?" matches the "hours" rule.

### Testing It

```rust
#[tokio::test]
async fn auto_reply_responds_to_keywords() {
    let plugin = AutoReplyPlugin::with_rules(vec![
        ("ping".into(), "pong".into()),
        ("status".into(), "online".into()),
    ]);

    let ctx = PluginContext {
        agent_address: "test".into(),
        agent_name: "test".into(),
        state: Default::default(),
        config: Default::default(),
    };

    let msg = Event::MessageReceived(Message {
        sender: "alice".into(),
        recipient: "test".into(),
        content: "ping".into(),
        timestamp: chrono::Utc::now(),
        metadata: serde_json::Value::Null,
    });

    let actions = plugin.handle_event(&msg, &ctx).await.unwrap();
    assert_eq!(actions.len(), 1);
    match &actions[0] {
        Action::SendMessage { to, content } => {
            assert_eq!(to, "alice");
            assert_eq!(content, "pong");
        }
        _ => panic!("expected SendMessage"),
    }
}
```

---

## Example 3: Stateful Counter Plugin

A plugin that counts messages per sender and persists the counts.

```rust
use anyhow::Result;
use async_trait::async_trait;
use nano_runtime::{Action, Event, EventKind, LogLevel, Plugin, PluginContext};

pub struct CounterPlugin;

#[async_trait]
impl Plugin for CounterPlugin {
    fn name(&self) -> &str { "counter" }
    fn version(&self) -> &str { "1.0.0" }

    async fn init(&mut self, _ctx: &PluginContext) -> Result<()> {
        Ok(())
    }

    async fn handle_event(
        &self,
        event: &Event,
        ctx: &PluginContext,
    ) -> Result<Vec<Action>> {
        match event {
            Event::MessageReceived(msg) => {
                // Read current count from state
                let key = format!("count:{}", msg.sender);
                let count = ctx.state
                    .get(&key)
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                let new_count = count + 1;

                Ok(vec![
                    // Persist the updated count
                    Action::StoreState {
                        key,
                        value: serde_json::json!(new_count),
                    },
                    // Reply with the count
                    Action::SendMessage {
                        to: msg.sender.clone(),
                        content: format!(
                            "Message #{} from you. Total messages tracked.",
                            new_count
                        ),
                    },
                    // Log it
                    Action::Log {
                        level: LogLevel::Info,
                        message: format!(
                            "{} has sent {} messages",
                            msg.sender, new_count
                        ),
                    },
                ])
            }
            _ => Ok(vec![]),
        }
    }

    fn subscriptions(&self) -> Vec<EventKind> {
        vec![EventKind::MessageReceived]
    }
}
```

**Key concept**: State is read from `ctx.state` (a snapshot) and written via `Action::StoreState`. The updated value appears in the next event's context.

---

## Example 4: Multi-Plugin Pipeline

Chain plugins together using custom events. This example implements a message filter + responder pipeline.

### Filter Plugin

Validates incoming messages and emits a custom event for valid ones:

```rust
pub struct FilterPlugin {
    allowed_senders: Vec<String>,
}

#[async_trait]
impl Plugin for FilterPlugin {
    fn name(&self) -> &str { "filter" }
    fn version(&self) -> &str { "1.0.0" }

    async fn init(&mut self, ctx: &PluginContext) -> Result<()> {
        // Load allowed senders from config
        if let Some(toml::Value::Array(arr)) = ctx.config.get("allowed_senders") {
            self.allowed_senders = arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
        }
        Ok(())
    }

    async fn handle_event(
        &self,
        event: &Event,
        _ctx: &PluginContext,
    ) -> Result<Vec<Action>> {
        match event {
            Event::MessageReceived(msg) => {
                if self.allowed_senders.contains(&msg.sender) {
                    // Forward valid messages as a custom event
                    Ok(vec![Action::EmitEvent {
                        kind: "validated-message".into(),
                        data: serde_json::json!({
                            "sender": msg.sender,
                            "content": msg.content,
                        }),
                    }])
                } else {
                    Ok(vec![Action::Log {
                        level: LogLevel::Warn,
                        message: format!("Blocked message from {}", msg.sender),
                    }])
                }
            }
            _ => Ok(vec![]),
        }
    }

    fn subscriptions(&self) -> Vec<EventKind> {
        vec![EventKind::MessageReceived]
    }
}
```

### Responder Plugin

Only processes messages that passed the filter:

```rust
pub struct ResponderPlugin;

#[async_trait]
impl Plugin for ResponderPlugin {
    fn name(&self) -> &str { "responder" }
    fn version(&self) -> &str { "1.0.0" }

    async fn init(&mut self, _ctx: &PluginContext) -> Result<()> { Ok(()) }

    async fn handle_event(
        &self,
        event: &Event,
        _ctx: &PluginContext,
    ) -> Result<Vec<Action>> {
        match event {
            Event::Custom { kind, data } if kind == "validated-message" => {
                let sender = data["sender"].as_str().unwrap_or("unknown");
                let content = data["content"].as_str().unwrap_or("");

                Ok(vec![Action::SendMessage {
                    to: sender.to_string(),
                    content: format!("Validated and processed: {}", content),
                }])
            }
            _ => Ok(vec![]),
        }
    }

    fn subscriptions(&self) -> Vec<EventKind> {
        vec![EventKind::Custom("validated-message".to_string())]
    }
}
```

### Config

```toml
[plugins.filter]
allowed_senders = ["alice", "bob", "ALGO_ADDRESS_HERE"]
```

### Wiring It Up

```rust
let mut runtime = Runtime::new(transport, config);
runtime.add_plugin(Box::new(FilterPlugin { allowed_senders: vec![] })).await?;
runtime.add_plugin(Box::new(ResponderPlugin)).await?;
```

---

## Example 5: Hub Forwarding (AI-Powered Agent)

Connect your nano agent to a corvid-agent-server for AI-powered responses.

### Setup

```bash
# 1. Set up the agent
can setup

# 2. Fund it
can fund

# 3. Add the hub server as a contact
can contacts add \
  --name corvidagent \
  --address SERVER_ALGO_ADDRESS \
  --psk PSK_HEX_FROM_SERVER

# 4. Register with the hub
can register --hub-url http://localhost:3578
```

### nano.toml

```toml
[agent]
name = "nano-scout"
network = "localnet"

[plugins.hub]
url = "http://localhost:3578"
```

### Running

```bash
# Start the agent with hub forwarding
can run
```

The hub plugin automatically:
1. Picks up AlgoChat messages from the transport
2. Forwards them to `POST {hub_url}/a2a/tasks/send`
3. Polls `GET {hub_url}/a2a/tasks/{id}` for the AI response
4. Sends the response back on-chain to the original sender

### Message Flow

```
Alice (on-chain) ──AlgoChat──▶ nano-agent ──HTTP──▶ corvid-agent-server
                                                          │
                                                    (AI processes)
                                                          │
Alice (on-chain) ◀──AlgoChat── nano-agent ◀──HTTP── response
```

---

## Example 6: MCP Server (Claude Code Integration)

Expose your agent's messaging capabilities as tools in Claude Code or Cursor.

### Setup

Add to your Claude Code config (`~/.claude.json` or project `.claude/settings.json`):

```json
{
  "mcpServers": {
    "nano": {
      "command": "can",
      "args": ["mcp", "--data-dir", "/path/to/data"]
    }
  }
}
```

Or for Cursor (`.cursor/mcp.json`):

```json
{
  "mcpServers": {
    "nano": {
      "command": "can",
      "args": ["mcp", "--network", "testnet"],
      "env": {
        "CAN_PASSWORD": "your-keystore-password"
      }
    }
  }
}
```

### Available MCP Tools

Once configured, Claude Code / Cursor can use:

| Tool | Description |
|------|-------------|
| `send_message` | Send an encrypted AlgoChat message to a contact |
| `list_contacts` | List all PSK contacts |
| `get_inbox` | View received messages (with optional filters) |
| `check_balance` | Check the agent's ALGO balance |
| `agent_info` | Get agent identity, address, and network info |

### Example Interaction

In Claude Code:
```
User: Send a message to alice saying "meeting at 3pm"
Claude: [calls send_message tool with to="alice", message="meeting at 3pm"]
        Message sent to alice (tx: ABCD1234...)
```

---

## Example 7: CLI Walkthrough (End-to-End)

A complete walkthrough of setting up two agents and having them communicate.

### Terminal 1: Agent A

```bash
# Set up Agent A
can setup --generate --network localnet --password secret --data-dir ./agent-a
can fund --data-dir ./agent-a
can info --data-dir ./agent-a
# Note the Algorand address (e.g. AAAA...)
```

### Terminal 2: Agent B

```bash
# Set up Agent B
can setup --generate --network localnet --password secret --data-dir ./agent-b
can fund --data-dir ./agent-b
can info --data-dir ./agent-b
# Note the Algorand address (e.g. BBBB...)
```

### Exchange PSK Keys

```bash
# Generate a shared PSK (any 64-char hex string works)
openssl rand -hex 32
# Output: a1b2c3d4...64 chars

# Agent A adds Agent B as a contact
can contacts add \
  --name agent-b \
  --address BBBB_ADDRESS \
  --psk a1b2c3d4... \
  --data-dir ./agent-a

# Agent B adds Agent A as a contact
can contacts add \
  --name agent-a \
  --address AAAA_ADDRESS \
  --psk a1b2c3d4... \
  --data-dir ./agent-b
```

### Start Both Agents

```bash
# Terminal 1
can run --data-dir ./agent-a

# Terminal 2
can run --data-dir ./agent-b
```

### Send Messages

```bash
# From a third terminal, send from Agent A to Agent B
can send --to agent-b --message "Hello from Agent A!" --data-dir ./agent-a

# Check Agent B's inbox
can inbox --data-dir ./agent-b
```

### Verify

```bash
# Check status of both agents
can status --data-dir ./agent-a
can status --data-dir ./agent-b

# View message history
can history --contact agent-b --data-dir ./agent-a
```

---

## Example 8: Group Broadcast

Send a single message to multiple agents via a group channel.

```bash
# Create a group
can groups create --name team-alpha --data-dir ./agent-a

# Add members
can groups add-member --group team-alpha --member agent-b --data-dir ./agent-a
can groups add-member --group team-alpha --member agent-c --data-dir ./agent-a

# View group
can groups show --group team-alpha --data-dir ./agent-a

# Broadcast to all members
can send --to team-alpha --message "Team standup in 5 minutes" --data-dir ./agent-a

# List all groups
can groups list --data-dir ./agent-a
```

---

## Quick Reference

### Common Command Patterns

```bash
# Setup & wallet
can setup                          # Interactive wizard
can setup --generate --network testnet  # Non-interactive
can import --mnemonic "word1 word2 ..."  # Import existing
can info                           # Show agent details
can change-password                # Rotate keystore password

# Funding
can fund                           # Localnet faucet
can fund --network testnet         # Shows dispenser URL
can balance                        # Quick balance check

# Messaging
can send --to alice --message "hi" # Send direct message
can inbox                          # View all messages
can inbox --from alice             # Filter by sender
can history --contact alice        # Full history with contact

# Contacts
can contacts add --name X --address Y --psk Z
can contacts list
can contacts remove alice
can contacts export --output backup.json
can contacts import --file backup.json

# Agent
can run                            # Start message loop
can status                         # Health check
can register --hub-url URL         # Register with hub

# Plugins
can plugin list                    # List loaded plugins
can plugin invoke P tool '{}'      # Invoke a plugin tool

# Server modes
can mcp                            # MCP server (stdio)
can config                         # View/edit nano.toml
```

### Environment Variables

All config can be set via environment variables with the `CAN_` prefix:

```bash
export CAN_DATA_DIR=~/.corvid
export CAN_NETWORK=testnet
export CAN_PASSWORD=mysecret
export CAN_HUB_URL=http://localhost:3578
export CAN_LOG_LEVEL=debug
export CAN_LOG_FORMAT=json
```

## Next Steps

- [Getting Started](../getting-started/quick-start.md) — first-time setup
- [Nano Runtime](./nano-runtime.md) — build native plugins
- [Custom Transport](./custom-transport.md) — implement your own transport
- [Plugin Development (WASM)](./plugin-development.md) — sandboxed WASM plugins
- [MCP Integration](./mcp-integration.md) — detailed MCP setup
- [Connecting to a Hub](./hub-connection.md) — AI-powered responses
