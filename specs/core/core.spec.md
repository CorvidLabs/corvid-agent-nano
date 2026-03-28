---
module: core
version: 1
status: active
files:
  - crates/core/src/lib.rs
  - crates/core/src/agent.rs
  - crates/core/src/message.rs
  - crates/core/src/config.rs
depends_on: []
---

# Core Types

## Purpose

Shared types and data structures used across all corvid-agent-nano crates. Provides the foundational domain model: agent identity, AlgoChat messages, and runtime configuration. This crate has zero network or crypto dependencies — it is pure data.

## Public API

### Exported Structs

| Struct | Description |
|--------|-------------|
| `AgentIdentity` | An agent's on-chain identity: Algorand address, name, X25519 public key, and capabilities |
| `Message` | A decrypted AlgoChat message with sender, recipient, content, timestamp, and optional txid |
| `NanoConfig` | Runtime configuration: Algorand node URL/token, agent name, hub URL, data directory |

### AgentIdentity Fields

| Field | Type | Description |
|-------|------|-------------|
| `address` | `String` | Algorand address |
| `name` | `String` | Human-readable agent name |
| `public_key` | `String` | X25519 public key (base64-encoded) |
| `capabilities` | `Vec<String>` | Agent capability tags for discovery |

### Message Fields

| Field | Type | Description |
|-------|------|-------------|
| `from` | `String` | Sender's Algorand address |
| `to` | `String` | Recipient's Algorand address |
| `content` | `String` | Decrypted message body |
| `timestamp` | `u64` | Unix timestamp |
| `txid` | `Option<String>` | Algorand transaction ID (None for outgoing pre-send) |

### NanoConfig Fields

| Field | Type | Description |
|-------|------|-------------|
| `algod_url` | `String` | Algorand node REST API URL |
| `algod_token` | `String` | Algorand node API token |
| `agent_name` | `String` | Agent display name |
| `hub_url` | `String` | corvid-agent hub API URL |
| `data_dir` | `String` | Local data directory path |

### Re-exports (lib.rs)

| Symbol | Source | Description |
|--------|--------|-------------|
| `AgentIdentity` | `agent::AgentIdentity` | Re-exported for convenience |
| `Message` | `message::Message` | Re-exported for convenience |

## Invariants

1. All structs derive `Debug`, `Clone`, `Serialize`, `Deserialize` — they must be printable, cloneable, and serializable to/from JSON
2. `AgentIdentity.public_key` must be a valid base64-encoded 32-byte X25519 public key when used in crypto operations
3. `Message.txid` is `None` for outgoing messages before submission, `Some(txid)` for confirmed on-chain messages
4. `NanoConfig.algod_url` must be a valid HTTP(S) URL
5. This crate has no side effects — no I/O, no networking, no filesystem access
6. `lib.rs` re-exports `AgentIdentity` and `Message` but NOT `NanoConfig` (config is accessed via `config::NanoConfig`)

## Behavioral Examples

### Scenario: Serialize and deserialize an AgentIdentity

- **Given** an `AgentIdentity` with name "nano-test", a valid Algorand address, a base64 X25519 public key, and capabilities `["messaging"]`
- **When** serialized to JSON via `serde_json::to_string` and deserialized back
- **Then** the roundtrip produces an identical struct

### Scenario: Create a pre-send message

- **Given** a sender address and recipient address
- **When** a `Message` is constructed with `txid: None`
- **Then** it represents an outgoing message not yet submitted to the chain

### Scenario: NanoConfig with defaults

- **Given** CLI defaults (localnet algod, default hub URL)
- **When** a `NanoConfig` is constructed
- **Then** `algod_url` is `http://localhost:4001`, `hub_url` is `http://localhost:3578`

## Error Cases

| Condition | Behavior |
|-----------|----------|
| Invalid base64 in `public_key` | Downstream crypto operations fail — core does not validate at construction |
| Empty `agent_name` | Allowed at the type level; validation is the caller's responsibility |

## Dependencies

### Consumes

| Module | What is used |
|--------|-------------|
| `serde` | `Serialize`, `Deserialize` derive macros |

### Consumed By

| Module | What is used |
|--------|-------------|
| `crates/crypto/src/identity.rs` | (future) `AgentIdentity` for key binding |
| `crates/algochat/src/client.rs` | `Message` for send/receive |
| `crates/algochat/src/listener.rs` | `Message` for incoming message construction |
| `src/main.rs` | `NanoConfig` for runtime configuration |

## Change Log

| Date | Author | Change |
|------|--------|--------|
| 2026-03-28 | CorvidAgent | Initial spec |
