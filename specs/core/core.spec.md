---
module: core
version: 3
status: active
files:
  - src/agent.rs
  - src/storage.rs
depends_on:
  - CorvidLabs/rs-algochat@algochat
---

# Core Types

## Purpose

Shared types and data structures used across all corvid-agent-nano crates. Provides the foundational domain model: agent identity, AlgoChat messages, and runtime configuration. Re-exports the external `algochat` crate (from `rs-algochat`) which provides X25519 key derivation, ChaCha20-Poly1305 encryption, and the full AlgoChat protocol. The local `corvid-crypto` and `corvid-algochat` crates have been replaced by this external dependency.

## Public API

### Exported Modules

| Module | Description |
|--------|-------------|
| `agent` | Agent identity type |
| `config` | Runtime configuration type |
| `message` | Message type |
| `storage` | SQLite-backed persistent storage implementations |

### Exported Structs

| Struct | Description |
|--------|-------------|
| `AgentIdentity` | An agent's on-chain identity: Algorand address, name, X25519 public key, and capabilities |
| `Message` | A decrypted AlgoChat message with sender, recipient, content, timestamp, and optional txid |
| `NanoConfig` | Runtime configuration: Algorand node URL/token, agent name, hub URL, data directory |
| `SqliteKeyStorage` | Persistent X25519 private key storage backed by SQLite |
| `SqliteMessageCache` | Persistent message cache and sync-round bookmarks backed by SQLite |

### Exported Functions

| Function | Parameters | Returns | Description |
|----------|-----------|---------|-------------|
| `open` | `path: impl AsRef<Path>` | `algochat::Result<Self>` | Open or create a SQLite database at the given file path (on both `SqliteKeyStorage` and `SqliteMessageCache`) |
| `in_memory` | — | `algochat::Result<Self>` | Create an in-memory database for testing (on both `SqliteKeyStorage` and `SqliteMessageCache`) |

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

### SQLite Storage (storage.rs)

| Struct | Trait Implemented | Description |
|--------|-------------------|-------------|
| `SqliteKeyStorage` | `algochat::EncryptionKeyStorage` | Persistent X25519 private key storage backed by SQLite |
| `SqliteMessageCache` | `algochat::MessageCache` | Persistent message cache and sync-round bookmarks backed by SQLite |

#### SqliteKeyStorage API

| Method | Description |
|--------|-------------|
| `open` | Open or create a SQLite database at the given file path |
| `in_memory` | Create an in-memory database (for testing) |
| `store(key, address, _biometric)` | Store a 32-byte private key for an address (INSERT OR REPLACE) |
| `retrieve(address)` | Retrieve a private key, returns `KeyNotFound` if missing |
| `has_key(address)` | Check if a key exists for an address |
| `delete(address)` | Delete a key (no-op if missing) |
| `list_stored_addresses()` | List all stored addresses |

#### SqliteMessageCache API

| Method | Description |
|--------|-------------|
| `open` | Open or create a SQLite database at the given file path |
| `in_memory` | Create an in-memory database (for testing) |
| `store(messages, participant)` | Store messages, deduplicating by message ID (INSERT OR IGNORE) |
| `retrieve(participant, after_round)` | Retrieve messages, optionally filtering by confirmed_round |
| `get_last_sync_round(participant)` | Get the last synced Algorand round for a conversation |
| `set_last_sync_round(round, participant)` | Set the last synced round (INSERT OR REPLACE) |
| `get_cached_conversations()` | List all participant addresses with cached messages |
| `clear()` | Delete all messages and sync rounds |
| `clear_for(participant)` | Delete messages and sync round for one participant |

#### Database Schema

**encryption_keys table:**
- `address TEXT PRIMARY KEY` — Algorand address
- `private_key BLOB NOT NULL` — 32-byte X25519 private key

**messages table:**
- `id TEXT PRIMARY KEY` — Transaction ID (dedup key)
- `participant TEXT NOT NULL` — Conversation partner address
- `sender TEXT NOT NULL`, `recipient TEXT NOT NULL` — Message endpoints
- `content TEXT NOT NULL` — Decrypted message body
- `timestamp_secs INTEGER NOT NULL` — Unix timestamp
- `confirmed_round INTEGER NOT NULL` — Algorand round number
- `direction TEXT NOT NULL` — "sent" or "received"
- `reply_to_id TEXT`, `reply_to_preview TEXT` — Optional reply context

**sync_rounds table:**
- `participant TEXT PRIMARY KEY` — Conversation partner address
- `last_round INTEGER NOT NULL` — Last synced Algorand round

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
5. `lib.rs` re-exports `AgentIdentity` and `Message` but NOT `NanoConfig` (config is accessed via `config::NanoConfig`)
6. `lib.rs` re-exports the external `algochat` crate for convenient access to crypto and protocol types
7. `SqliteKeyStorage` and `SqliteMessageCache` use `Mutex<Connection>` for thread safety — safe to share across async tasks
8. Message deduplication is enforced by `INSERT OR IGNORE` on the transaction ID primary key
9. Key overwrites are allowed via `INSERT OR REPLACE` — storing a key for an existing address replaces it
10. Database tables are created on open (`CREATE TABLE IF NOT EXISTS`) — no separate migration step needed

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

### Scenario: Persistent message cache survives restart

- **Given** a `SqliteMessageCache` opened at `data/messages.db` with messages stored for participant "alice" and last_sync_round set to 500
- **When** the process exits and a new `SqliteMessageCache` is opened at the same path
- **Then** `retrieve("alice", None)` returns the previously stored messages
- **And** `get_last_sync_round("alice")` returns `Some(500)`

### Scenario: Message deduplication across stores

- **Given** a `SqliteMessageCache` with message ID "tx1" already stored for "alice"
- **When** `store([msg_with_id_tx1], "alice")` is called again
- **Then** no duplicate is created; `retrieve("alice", None)` still returns exactly 1 message

## Error Cases

| Condition | Behavior |
|-----------|----------|
| Invalid base64 in `public_key` | Downstream crypto operations fail — core does not validate at construction |
| Empty `agent_name` | Allowed at the type level; validation is the caller's responsibility |
| SQLite database file cannot be opened | `SqliteKeyStorage::open` / `SqliteMessageCache::open` returns `AlgoChatError::StorageFailed` |
| Key not found in SQLite storage | `retrieve()` returns `AlgoChatError::KeyNotFound(address)` |
| Corrupt key blob (wrong length) | `retrieve()` returns `AlgoChatError::StorageFailed("Invalid key length")` |

## Dependencies

### Consumes

| Module | What is used |
|--------|-------------|
| `serde` | `Serialize`, `Deserialize` derive macros |
| `algochat` (external, git: rs-algochat) | Re-exported for crypto, key derivation, and AlgoChat protocol |
| `rusqlite` | SQLite database access for persistent storage |
| `async-trait` | Async trait implementations for storage traits |

### Consumed By

| Module | What is used |
|--------|-------------|
| `src/main.rs` | `NanoConfig` for runtime configuration, `algochat` re-export for crypto/protocol |

## Change Log

| Date | Author | Change |
|------|--------|--------|
| 2026-03-28 | CorvidAgent | Initial spec |
| 2026-03-28 | CorvidAgent | Replace local crypto/algochat crates with external rs-algochat dependency |
| 2026-03-28 | CorvidAgent | Add SQLite storage module: SqliteKeyStorage + SqliteMessageCache with 16 tests |
| 2026-03-28 | CorvidAgent | Add Exported Modules/Functions sections for spec-sync strict compliance |
