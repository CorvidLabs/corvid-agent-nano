---
module: agent
version: 1
status: active
files:
  - src/agent.rs
depends_on:
  - specs/hub/hub.spec.md
  - specs/transaction/transaction.spec.md
  - external: algochat (git: https://github.com/CorvidLabs/rs-algochat)
---

# Agent Message Loop

## Purpose

Core message processing loop — polls AlgoChat for new encrypted messages, forwards them to the corvid-agent hub via A2A, encrypts the response, and sends it back on-chain. Also handles periodic heartbeats to keep the agent's Flock Directory entry active.

## Public API

| Function | Signature | Description |
|----------|-----------|-------------|
| `run_message_loop` | `(client, algod, signing_key, hub, config) -> ()` | Infinite loop: sync → process → reply → sleep |

| Struct | Description |
|--------|-------------|
| `AgentLoopConfig` | Loop configuration: poll interval, hub URL, agent name, address |

### AgentLoopConfig Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `poll_interval_secs` | `u64` | `5` | Seconds between sync polls |
| `hub_url` | `String` | `http://localhost:3578` | Hub API URL |
| `agent_name` | `String` | `nano` | Agent display name |
| `address` | `String` | `""` | This agent's Algorand address |

## Invariants

1. Own messages (sender == our address) are skipped
2. Heartbeats are sent every 60 seconds
3. Sync failures log a warning and retry next interval (never crash)
4. Hub forwarding failures log an error and skip the message (don't block the loop)
5. Reply sending failures log an error and continue processing
6. Received messages are logged with sender, round, and content (truncated to 100 chars)
7. The loop runs until externally cancelled (no self-exit condition)

## Message Flow

1. `client.sync()` — poll indexer for new AlgoChat transactions
2. For each message where `sender != our_address`:
   a. `hub.forward_message(content, sender)` — submit A2A task, poll for response
   b. `client.discover_key(recipient)` — find recipient's encryption key
   c. `client.encrypt(response, pubkey)` — encrypt the response
   d. `transaction::send_note_transaction(...)` — submit encrypted response on-chain

## Dependencies

### Consumes

| Module | What is used |
|--------|-------------|
| `hub` | `HubClient::heartbeat`, `HubClient::forward_message` |
| `transaction` | `send_note_transaction` for sending replies |
| `algochat` | `AlgoChat::sync`, `AlgoChat::discover_key`, `AlgoChat::encrypt` |

### Consumed By

| Module | What is used |
|--------|-------------|
| `src/main.rs` | `run_message_loop`, `AgentLoopConfig` in `cmd_run` |

## Change Log

| Date | Author | Change |
|------|--------|--------|
| 2026-03-28 | CorvidAgent | Initial spec |
