---
module: core
version: 4
status: stable
files:
  - src/agent.rs
  - src/storage.rs
depends_on:
  - CorvidLabs/rs-algochat@algochat
---

# Core Types

## Purpose

Agent message loop and SQLite-backed persistent storage. The agent module polls for AlgoChat messages, forwards them to the hub, and sends encrypted replies on-chain. The storage module provides `EncryptionKeyStorage` and `MessageCache` trait implementations backed by SQLite, so encryption keys and message history survive restarts.

## Public API

### Exported Structs

| Struct | Description |
|--------|-------------|
| `AgentLoopConfig` | Configuration for the agent message loop: poll interval, hub URL, agent name, agent address, signing key |
| `SqliteKeyStorage` | Persistent X25519 private key storage backed by SQLite |
| `SqliteMessageCache` | Persistent message cache and sync-round bookmarks backed by SQLite |

### Exported Functions

| Function | Parameters | Returns | Description |
|----------|-----------|---------|-------------|
| `run_message_loop` | `client: Arc<AlgoChat<A,I,S,M>>`, `algod: Arc<A>`, `config: AgentLoopConfig` | `()` (runs forever) | Polls for AlgoChat messages, forwards to hub, sends encrypted replies on-chain |
| `send_reply` | `client`, `algod`, `sender_address`, `recipient_address`, `message`, `signing_key` | `anyhow::Result<String>` | Encrypt a message (PSK or X25519) and send it on-chain, returns txid |
| `open` | `path: impl AsRef<Path>` | `algochat::Result<Self>` | Open or create a SQLite database (method on both `SqliteKeyStorage` and `SqliteMessageCache`) |
| `in_memory` | — | `algochat::Result<Self>` | Create an in-memory database for testing (method on both `SqliteKeyStorage` and `SqliteMessageCache`) |

### Struct Methods

#### AgentLoopConfig Fields

| Field | Type | Description |
|-------|------|-------------|
| `poll_interval_secs` | `u64` | How often to poll for new messages (seconds) |
| `hub_url` | `Option<String>` | Hub URL for corvid-agent API. None = P2P mode (no hub forwarding) |
| `agent_name` | `String` | Agent display name |
| `agent_address` | `String` | Agent's Algorand address (for sending replies) |
| `signing_key` | `SigningKey` | Ed25519 signing key (for signing reply transactions) |

#### SqliteKeyStorage Methods

| Method | Parameters | Returns | Description |
|--------|-----------|---------|-------------|
| `open` | `path: impl AsRef<Path>` | `algochat::Result<Self>` | Open or create a SQLite database at the given file path |
| `in_memory` | — | `algochat::Result<Self>` | Create an in-memory database for testing |

#### SqliteMessageCache Methods

| Method | Parameters | Returns | Description |
|--------|-----------|---------|-------------|
| `open` | `path: impl AsRef<Path>` | `algochat::Result<Self>` | Open or create a SQLite database at the given file path |
| `in_memory` | — | `algochat::Result<Self>` | Create an in-memory database for testing |

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

## Invariants

1. `AgentLoopConfig::signing_key` must be a valid 32-byte Ed25519 key
2. `AgentLoopConfig::agent_address` must be a valid Algorand address
3. `AgentLoopConfig::poll_interval_secs` must be greater than 0
4. `SqliteKeyStorage` and `SqliteMessageCache` use `Mutex<Connection>` for thread safety — safe to share across async tasks
5. Message deduplication is enforced by `INSERT OR IGNORE` on the transaction ID primary key
6. Key overwrites are allowed via `INSERT OR REPLACE` — storing a key for an existing address replaces it
7. Database tables are created on open (`CREATE TABLE IF NOT EXISTS`) — no separate migration step needed
8. Keys are stored as raw bytes (not encrypted at rest — use full-disk encryption for data at rest)
9. `AgentLoopConfig` defaults to 5s poll interval, hub at `http://localhost:3578`, agent name "can"
10. `run_message_loop` runs forever — it never returns under normal operation
11. `send_reply` tries PSK encryption first, falls back to X25519 key discovery if no PSK contact found
12. `AgentLoopConfig.hub_url` is `None` in P2P mode (no hub forwarding), `Some(url)` when a hub is configured

## Behavioral Examples

### Scenario: Default AgentLoopConfig

- **Given** `AgentLoopConfig::default()`
- **When** inspected
- **Then** `poll_interval_secs` is 5, `hub_url` is `Some("http://localhost:3578")`, `agent_name` is "can"

### Scenario: P2P mode (no hub)

- **Given** an `AgentLoopConfig` with `hub_url: None`
- **When** `run_message_loop` is started
- **Then** messages are received and stored but not forwarded to any hub

### Scenario: Persistent key storage survives restart

- **Given** `SqliteKeyStorage` opened at `data/keys.db` with a 32-byte private key stored for address "AAAA..."
- **When** the process exits and a new `SqliteKeyStorage` is opened at the same path
- **Then** `retrieve("AAAA...")` returns the same 32-byte key

### Scenario: Key overwrite via INSERT OR REPLACE

- **Given** a `SqliteKeyStorage` with key K1 stored for address "AAAA..."
- **When** `store(&K2, "AAAA...")` is called with a different key
- **Then** `retrieve("AAAA...")` returns K2, replacing K1

### Scenario: Persistent message cache survives restart

- **Given** a `SqliteMessageCache` opened at `data/messages.db` with messages stored for participant "alice" and last_sync_round set to 500
- **When** the process exits and a new `SqliteMessageCache` is opened at the same path
- **Then** `retrieve("alice", None)` returns the previously stored messages
- **And** `get_last_sync_round("alice")` returns `Some(500)`

### Scenario: Message deduplication across stores

- **Given** a `SqliteMessageCache` with message ID "tx1" already stored for "alice"
- **When** `store([msg_with_id_tx1], "alice")` is called again
- **Then** no duplicate is created; `retrieve("alice", None)` still returns exactly 1 message

### Scenario: AgentLoopConfig with defaults

- **Given** CLI defaults
- **When** an `AgentLoopConfig` is constructed
- **Then** `poll_interval_secs` is 5, `hub_url` is `Some("http://localhost:3578")`, `agent_name` is `"can"`

## Error Cases

| Condition | Behavior |
|-----------|----------|
| Hub unreachable during message loop | Sends `[error] Agent hub is unreachable` reply on-chain, continues polling |
| Hub task times out or fails | Sends `[error] Agent did not produce a response` reply on-chain, continues polling |
| No PSK contact and no X25519 key found for recipient | `send_reply` returns `Err` ("No encryption key found for {address}") |
| SQLite database file cannot be opened | `SqliteKeyStorage::open` / `SqliteMessageCache::open` returns `AlgoChatError::StorageFailed` |
| Database mutex is poisoned | Lock returns `AlgoChatError::StorageFailed("Database lock poisoned")` |
| Key not found in SQLite storage | `retrieve()` returns `AlgoChatError::KeyNotFound(address)` |
| Corrupt key blob (wrong length) | `retrieve()` returns `AlgoChatError::StorageFailed("Invalid key length")` |
| Invalid `AgentLoopConfig::signing_key` | Upstream code fails during transaction signing |
| Invalid `AgentLoopConfig::agent_address` | Upstream code fails during message processing |

## Dependencies

### Consumes

| Module | What is used |
|--------|-------------|
| `algochat` (external, git: rs-algochat) | `AlgoChat`, `AlgodClient`, `IndexerClient`, `EncryptionKeyStorage`, `MessageCache`, `Message` types and traits |
| `ed25519_dalek` | `SigningKey` for signing Algorand transactions |
| `serde` | `Serialize`, `Deserialize` derive macros |
| `reqwest` | HTTP client for hub communication |
| `rusqlite` | SQLite database access for persistent storage |
| `async-trait` | Async trait implementations for storage traits |
| `anyhow` | Error handling in `send_reply` |
| `tracing` | Logging and debugging |

### Consumed By

| Module | What is used |
|--------|-------------|
| `src/main.rs` | `AgentLoopConfig`, `run_message_loop` for agent startup; `SqliteKeyStorage`, `SqliteMessageCache` for persistence |
| `src/algorand.rs` | `AlgodClient` and `IndexerClient` trait implementation |

## Change Log

| Date | Author | Change |
|------|--------|--------|
| 2026-03-28 | CorvidAgent | Initial spec |
| 2026-03-28 | CorvidAgent | Replace local crypto/algochat crates with external rs-algochat dependency |
| 2026-03-28 | CorvidAgent | Add SQLite storage module: SqliteKeyStorage + SqliteMessageCache with 16 tests |
| 2026-03-28 | CorvidAgent | Add Exported Modules/Functions sections for spec-sync strict compliance |
| 2026-03-30 | CorvidAgent | Rewrite spec to match current source: remove old types (AgentIdentity, Message, NanoConfig), add AgentLoopConfig, run_message_loop, send_reply |
| 2026-04-06 | Magpie | Update file paths to match actual codebase (src/agent.rs, src/storage.rs) |
| 2026-04-06 | Magpie | Refactor Public API to document only actual exports: AgentLoopConfig, run_message_loop, SqliteKeyStorage, SqliteMessageCache |
| 2026-04-18 | CorvidAgent | Resolve merge conflicts: merge both changelog entries and combine invariants/error cases from both branches |
