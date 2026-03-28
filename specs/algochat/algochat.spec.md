---
module: algochat
version: 1
status: draft
files:
  - crates/algochat/src/lib.rs
  - crates/algochat/src/client.rs
  - crates/algochat/src/listener.rs
depends_on:
  - specs/core/core.spec.md
  - specs/crypto/crypto.spec.md
---

# AlgoChat

## Purpose

AlgoChat protocol implementation for corvid-agent-nano. Sends and receives encrypted messages via Algorand transactions. The client constructs payment transactions with encrypted note fields, and the listener polls the chain for incoming messages. This is the primary communication channel between nano agents and the corvid-agent platform.

Wire-compatible with corvid-agent's TypeScript AlgoChat implementation — messages sent by either side can be received and decrypted by the other.

## Public API

### Exported Structs

| Struct | Description |
|--------|-------------|
| `AlgoChatClient` | Client for sending/receiving AlgoChat messages via Algorand REST API |

### AlgoChatClient Methods

| Method | Parameters | Returns | Description |
|--------|-----------|---------|-------------|
| `new` | `(algod_url: &str, algod_token: &str)` | `Self` | Create a new client connected to an Algorand node |
| `health_check` | `(&self)` | `Result<bool>` | Check connectivity to the Algorand node via `/health` endpoint |

### Planned Methods (TODO — not yet implemented)

| Method | Parameters | Returns | Description |
|--------|-----------|---------|-------------|
| `send_message` | `(&self, from: &KeyPair, to_addr: &str, to_pubkey: &[u8; 32], content: &str)` | `Result<String>` | Encrypt and send a message as an Algorand payment txn; returns txid |
| `poll_messages` | `(&self, address: &str, since_round: u64)` | `Result<Vec<RawTransaction>>` | Fetch transactions to `address` since `since_round` |
| `parse_note` | `(note: &[u8], shared_secret: &[u8; 32])` | `Result<Message>` | Decrypt and decode an AlgoChat note field into a `Message` |

### Re-exports (lib.rs)

| Symbol | Source | Description |
|--------|--------|-------------|
| `AlgoChatClient` | `client::AlgoChatClient` | Re-exported for convenience |

## Invariants

1. `AlgoChatClient` holds an `algod_url`, `algod_token`, and a `reqwest::Client` — it does not hold crypto keys (keys are passed per-call)
2. `health_check()` sends `GET /health` with the `X-Algo-API-Token` header and returns `true` if the response status is 2xx
3. Messages are encrypted using the same X25519 + ChaCha20-Poly1305 protocol as corvid-agent's TypeScript AlgoChat
4. The note field format is: `nonce (12B) || ciphertext` (produced by `corvid-crypto::encrypt`)
5. The listener module will poll by round number, not by timestamp — rounds are the canonical ordering on Algorand
6. All network calls use the stored `http` client (connection pooling)

## Behavioral Examples

### Scenario: Health check against a running localnet

- **Given** an `AlgoChatClient` pointed at `http://localhost:4001` with a valid token
- **When** `health_check()` is called
- **Then** it returns `Ok(true)` and logs the result

### Scenario: Health check against an unreachable node

- **Given** an `AlgoChatClient` pointed at `http://localhost:9999` (no server)
- **When** `health_check()` is called
- **Then** it returns `Err` (reqwest connection error)

### Scenario: Send an encrypted message (planned)

- **Given** a connected client, sender's `KeyPair`, recipient's address and public key
- **When** `send_message()` is called with content "hello"
- **Then** it encrypts the content using the DH shared secret, constructs an Algorand payment txn with the ciphertext as the note field, submits it, and returns the txid

### Scenario: Poll and decrypt incoming messages (planned)

- **Given** a listener watching the agent's Algorand address
- **When** new transactions arrive with note fields
- **Then** they are fetched via `poll_messages()`, decrypted via `parse_note()`, and converted to `Message` structs

## Error Cases

| Condition | Behavior |
|-----------|----------|
| Algorand node unreachable | `health_check()` and all network methods return `Err` (reqwest error) |
| Invalid API token | Node returns 401; methods return `Err` or `Ok(false)` for health check |
| Malformed note field | `parse_note()` returns `Err` (decrypt failure or invalid format) |
| Insufficient funds for txn | `send_message()` returns `Err` (Algorand node rejects transaction) |

## Dependencies

### Consumes

| Module | What is used |
|--------|-------------|
| `corvid-core` | `Message` struct for decoded messages |
| `corvid-crypto` | `KeyPair`, `encrypt`, `decrypt` for message encryption |
| `reqwest` | HTTP client for Algorand REST API |
| `tracing` | Structured logging |
| `anyhow` | Error handling |

### Consumed By

| Module | What is used |
|--------|-------------|
| `src/main.rs` | `AlgoChatClient` for node connectivity and messaging |

## Change Log

| Date | Author | Change |
|------|--------|--------|
| 2026-03-28 | CorvidAgent | Initial spec — health_check implemented, send/poll/parse planned |
