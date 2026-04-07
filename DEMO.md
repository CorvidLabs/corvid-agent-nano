# corvid-agent-nano Demo Guide

Step-by-step scenarios for demonstrating `can` — the lightweight Rust agent for AlgoChat on Algorand.

---

## Demo 1: Hello World — First Message in 2 Minutes

**What it shows:** Install, wallet setup, funding, and agent status.

```bash
# 1. Install
cargo install corvid-agent-nano

# 2. Start localnet
algokit localnet start

# 3. Create wallet (non-interactive for demo speed)
can setup --generate --network localnet --password demo123

# 4. Fund from faucet
can fund

# 5. Check everything
can status

# 6. See your agent identity
can info
```

**Expected output from `can status`:**
```
Agent Status
  Name:     can
  Network:  localnet
  Address:  XXXXX...
  Balance:  10.000000 ALGO
  Algod:    connected (localhost:4001)
  Indexer:  connected (localhost:8980)
  Contacts: 0
  Plugins:  disabled (no plugin host)
```

---

## Demo 2: Two Agents Talking

**What it shows:** Contact exchange, encrypted messaging, on-chain message delivery.

**Setup: Create two agents in separate terminals.**

### Terminal 1 — Agent "Alpha"
```bash
mkdir -p /tmp/alpha && cd /tmp/alpha
can setup --generate --network localnet --password alpha123
can fund
can info   # Note the address — you'll need it for Beta
```

### Terminal 2 — Agent "Beta"
```bash
mkdir -p /tmp/beta && cd /tmp/beta
can setup --generate --network localnet --password beta123
can fund
can info   # Note the address — you'll need it for Alpha
```

### Exchange contacts (both terminals need matching PSKs)

Generate a shared PSK (any 64-char hex string):
```bash
# Generate a random PSK
openssl rand -hex 32
# Example output: a1b2c3d4e5f6...
```

### Terminal 1 — Alpha adds Beta
```bash
can contacts add \
  --name beta \
  --address <BETA_ADDRESS> \
  --psk <SHARED_PSK>
```

### Terminal 2 — Beta adds Alpha
```bash
can contacts add \
  --name alpha \
  --address <ALPHA_ADDRESS> \
  --psk <SHARED_PSK>
```

### Send and receive

**Terminal 2** — Start Beta listening:
```bash
can run --no-hub --password beta123
```

**Terminal 1** — Alpha sends a message:
```bash
can send --to beta --message "Hello Beta, this is Alpha!" --password alpha123
```

**Terminal 2** — Beta should log the received message. Check the inbox:
```bash
# In another terminal, same data dir
can inbox
```

---

## Demo 3: nano.toml Configuration

**What it shows:** File-based configuration instead of CLI flags.

```bash
# See where config lives
can config path

# Show current (default) config
can config show

# Set values
can config set agent.name "demo-agent"
can config set runtime.poll_interval 10
can config set hub.disabled true
can config set logging.level "debug"

# Verify
can config show
```

**Example output:**
```toml
[agent]
name = "demo-agent"

[hub]
disabled = true

[runtime]
poll_interval = 10

[logging]
level = "debug"
```

Now `can run` uses these settings automatically — no flags needed.

---

## Demo 4: Group Broadcast Channel

**What it shows:** One-to-many encrypted messaging via group PSKs.

```bash
# Create a group
can groups create --name scouting-team

# Add members
can groups add-member --group scouting-team --address <AGENT_1_ADDRESS> --label "alpha"
can groups add-member --group scouting-team --address <AGENT_2_ADDRESS> --label "beta"

# View group details
can groups show scouting-team

# List all groups
can groups list

# Broadcast to the group
can send --group scouting-team --message "Status report: all clear"
```

---

## Demo 5: MCP Integration with Claude Code

**What it shows:** Using `can` as an MCP tool provider for AI assistants.

### Start the MCP server
```bash
can mcp --password mypassword
```

### Configure Claude Code

Add to your Claude Code MCP config (`~/.claude/config.json` or project `.mcp.json`):
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

### Available MCP tools

Once connected, Claude Code can use:

| Tool | Description |
|------|-------------|
| `agent_info` | Get agent identity (name, address, network) |
| `list_contacts` | List all PSK contacts |
| `get_inbox` | Read recent messages (optional: filter by sender) |
| `check_balance` | Check ALGO balance |
| `send_message` | Send encrypted message to a contact |

### Example interaction in Claude Code

> "Check my agent's balance and see if I have any messages"

Claude Code calls `check_balance` and `get_inbox`, then reports the results conversationally.

---

## Demo 6: WASM Plugin System

**What it shows:** Loading and invoking a WASM plugin.

### Build the example plugin
```bash
# From the repo root
cargo build -p hello-world-plugin --target wasm32-wasip1 --release

# Copy to plugins directory
mkdir -p data/plugins
cp target/wasm32-wasip1/release/hello_world_plugin.wasm data/plugins/
```

### Use plugins
```bash
# Start the agent with plugins enabled
can run --password mypassword &

# List loaded plugins
can plugin list

# Invoke the hello tool
can plugin invoke hello-world hello '{"name": "Alice"}'
# Output: {"greeting": "Hello, Alice!"}

# Invoke the echo tool
can plugin invoke hello-world echo '{"message": "test"}'
# Output: {"message": "test"}

# Check plugin health
can plugin health

# Unload a plugin at runtime
can plugin unload hello-world
```

### Trust tiers

```bash
# Load with explicit trust tier
can plugin load ./my-plugin.wasm --tier untrusted    # 32 MiB, 10s timeout
can plugin load ./my-plugin.wasm --tier verified      # 128 MiB, 30s timeout
can plugin load ./my-plugin.wasm --tier trusted       # 512 MiB, 60s timeout
```

---

## Demo 7: Hub Integration (Full Stack)

**What it shows:** `can` agent connected to the corvid-agent platform for AI-powered responses.

### Prerequisites
- A running [corvid-agent](https://github.com/CorvidLabs/corvid-agent) server

### Setup

```bash
# 1. Get the can agent's address
can info

# 2. Register the can agent on the hub server
curl -X POST http://localhost:3000/api/algochat/psk/contacts \
  -H "Content-Type: application/json" \
  -d '{"name": "can-demo", "address": "<CAN_ADDRESS>"}'
# Save the returned PSK and server address

# 3. Add the hub as a contact on can
can contacts add \
  --name hub \
  --address <HUB_ADDRESS> \
  --psk <PSK_FROM_STEP_2>

# 4. Register with the hub
can register --hub-url http://localhost:3578

# 5. Run with hub forwarding
can run --hub-url http://localhost:3578
```

Now incoming messages are forwarded to the hub's AI for processing, and responses are sent back encrypted on-chain.

---

## Demo 8: Health Monitoring & JSON Logging

**What it shows:** Production deployment features.

```bash
# Run with health endpoint
can run --health-port 9090 --log-format json
# Note: --log-format is a global flag, so this also works:
# can --log-format json run --health-port 9090

# In another terminal, check health
curl -s http://localhost:9090/health | jq
```

**Health response:**
```json
{
  "status": "healthy",
  "network": "localnet",
  "address": "XXXXX...",
  "uptime_secs": 42,
  "algod_url": "http://localhost:4001",
  "indexer_url": "http://localhost:8980",
  "hub_url": "http://localhost:3578"
}
```

**JSON log output** (structured, pipe to jq or log aggregator):
```json
{"timestamp":"2026-04-07T12:00:00Z","level":"INFO","message":"polling for messages","round":1234}
```

---

## Demo 9: Contact Import/Export & Backup

**What it shows:** Portability of contacts and groups between agents.

```bash
# Export contacts to JSON
can contacts export --output my-contacts.json

# Export groups
can groups export --output my-groups.json

# Import on another machine or data directory
can contacts import --file my-contacts.json --data-dir /tmp/new-agent
can groups import --file my-groups.json --data-dir /tmp/new-agent
```

---

## Demo 10: Multi-Network Deployment

**What it shows:** Running on different Algorand networks.

```bash
# Localnet (development) — default
can setup --network localnet --generate --password dev123 --data-dir ./data-local
can fund --data-dir ./data-local

# Testnet (staging)
can setup --network testnet --generate --password test123 --data-dir ./data-testnet
can fund --data-dir ./data-testnet   # Shows dispenser link

# Mainnet (production)
can setup --network mainnet --generate --password prod123 --data-dir ./data-mainnet
# Fund via exchange or wallet transfer

# Custom endpoints
can run \
  --algod-url https://my-algod.example.com \
  --algod-token mytoken \
  --indexer-url https://my-indexer.example.com
```

---

## Quick Demo Cheat Sheet

For a fast 5-minute demo, run these commands in order:

```bash
algokit localnet start
can setup --generate --network localnet --password demo
can fund
can status
can info
can config set agent.name "demo-bot"
can config show
can groups create --name demo-team
can groups list
can run --no-hub --health-port 9090 &
curl -s localhost:9090/health | jq
can plugin list
```

This covers: setup, funding, status, config, groups, runtime, health, and plugins — all in under 5 minutes.
