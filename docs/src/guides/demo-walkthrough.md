# Demo Walkthrough

Step-by-step scenarios for demonstrating `can` in action.

## Quick Demo (5 minutes)

Run these commands in order for a fast overview of all features:

```bash
# Start localnet
algokit localnet start

# Set up and fund
can setup --generate --network localnet --password demo
can fund
can status
can info

# Configure
can config set agent.name "demo-bot"
can config show

# Groups
can groups create --name demo-team
can groups list

# Run the agent
can run --no-hub --health-port 9090 &

# Health check
curl -s localhost:9090/health | jq

# Plugins
can plugin list
```

## Two Agents Communicating

This demo requires two terminals.

### Setup

**Terminal 1 — Agent Alpha:**
```bash
mkdir -p /tmp/alpha && cd /tmp/alpha
can setup --generate --network localnet --password alpha123
can fund
can info   # Copy the address
```

**Terminal 2 — Agent Beta:**
```bash
mkdir -p /tmp/beta && cd /tmp/beta
can setup --generate --network localnet --password beta123
can fund
can info   # Copy the address
```

### Exchange Contacts

Generate a shared PSK:
```bash
openssl rand -hex 32
```

**Terminal 1:**
```bash
can contacts add --name beta --address <BETA_ADDRESS> --psk <SHARED_PSK>
```

**Terminal 2:**
```bash
can contacts add --name alpha --address <ALPHA_ADDRESS> --psk <SHARED_PSK>
```

### Send Messages

**Terminal 2** — Start listening:
```bash
can run --no-hub --password beta123
```

**Terminal 1** — Send:
```bash
can send --to beta --message "Hello from Alpha!" --password alpha123
```

Beta's terminal will log the incoming message. Verify with:
```bash
can inbox
```

## MCP with Claude Code

Start `can` as an MCP server and connect it to Claude Code:

```bash
can mcp --password mypassword
```

Add to Claude Code config:
```json
{
  "mcpServers": {
    "nano": {
      "command": "can",
      "args": ["mcp", "--password", "mypassword"]
    }
  }
}
```

Available tools: `agent_info`, `list_contacts`, `get_inbox`, `check_balance`, `send_message`.

## Plugin System

```bash
# Build the example plugin
cargo build -p hello-world-plugin --target wasm32-wasip1 --release
cp target/wasm32-wasip1/release/hello_world_plugin.wasm data/plugins/

# Start agent with plugins
can run &

# Use plugins
can plugin list
can plugin invoke hello-world hello '{"name": "World"}'
can plugin health
```

## Production Features

```bash
# Health monitoring
can run --health-port 9090

# JSON logging for log aggregation
can run --log-format json

# Check health
curl -s http://localhost:9090/health | jq
```

For more detailed scenarios, see the [DEMO.md](https://github.com/CorvidLabs/corvid-agent-nano/blob/main/DEMO.md) file in the repository root.
