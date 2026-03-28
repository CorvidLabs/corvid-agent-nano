---
module: nano-cli
version: 3
status: active
files:
  - src/main.rs
  - src/agent.rs
  - src/algorand.rs
depends_on:
  - specs/core/core.spec.md
  - external: algochat (git: https://github.com/CorvidLabs/rs-algochat)
---

# Nano CLI

## Purpose

Binary entry point for corvid-agent-nano. Parses CLI arguments, initializes crypto identity from an Ed25519 seed, creates HTTP-based Algorand clients, starts the AlgoChat message polling loop, and runs until Ctrl+C. Provides a single-binary, instant-startup agent that connects to the corvid-agent ecosystem via the AlgoChat protocol on Algorand.

## Public API

### CLI Arguments (clap)

| Flag | Type | Default | Env Var | Description |
|------|------|---------|---------|-------------|
| `--algod-url` | `String` | `http://localhost:4001` | — | Algorand node REST API URL |
| `--algod-token` | `String` | `aaa...aaa` (64 a's) | — | Algorand node API token |
| `--indexer-url` | `String` | `http://localhost:8980` | — | Algorand indexer REST API URL |
| `--indexer-token` | `String` | `aaa...aaa` (64 a's) | — | Algorand indexer API token |
| `--seed` | `String` | (required) | `NANO_SEED` | Hex-encoded 32-byte Ed25519 private key |
| `--address` | `String` | (required) | `NANO_ADDRESS` | Agent's Algorand address |
| `--name` | `String` | `nano` | — | Agent name for discovery and display |
| `--hub-url` | `String` | `http://localhost:3578` | — | corvid-agent hub API URL |
| `--poll-interval` | `u64` | `5` | — | Message poll interval in seconds |

### Source Modules

| File | Description |
|------|-------------|
| `src/main.rs` | CLI parsing, identity init, client wiring, shutdown |
| `src/algorand.rs` | `HttpAlgodClient` and `HttpIndexerClient` — HTTP adapters for rs-algochat traits |
| `src/agent.rs` | `run_message_loop` — polls indexer for AlgoChat messages and processes them |

### algorand.rs — HTTP Trait Implementations

| Struct | Implements | Description |
|--------|-----------|-------------|
| `HttpAlgodClient` | `algochat::AlgodClient` | HTTP client for algod v2 REST API |
| `HttpIndexerClient` | `algochat::IndexerClient` | HTTP client for indexer v2 REST API |

#### HttpAlgodClient Endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| `get_suggested_params` | `GET /v2/transactions/params` | Fetch network params for transaction building |
| `get_account_info` | `GET /v2/accounts/{addr}` | Fetch account balance and min-balance |
| `submit_transaction` | `POST /v2/transactions` | Submit a signed transaction (binary body) |
| `wait_for_confirmation` | `GET /v2/transactions/pending/{txid}` | Poll until confirmed or timeout |
| `get_current_round` | `GET /v2/status` | Get the latest confirmed round |

#### HttpIndexerClient Endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| `search_transactions` | `GET /v2/transactions?address=&note-prefix=AQ` | Search for AlgoChat txns (note prefix filters for protocol v1) |
| `search_transactions_between` | (filters `search_transactions`) | Filter to only txns between two addresses |
| `get_transaction` | `GET /v2/transactions/{txid}` | Fetch a specific transaction by ID |
| `wait_for_indexer` | (polls `get_transaction`) | Poll until indexed or timeout |

### agent.rs — Message Loop & Hub Forwarding

| Function | Parameters | Description |
|----------|-----------|-------------|
| `run_message_loop` | `Arc<AlgoChat<...>>`, `AgentLoopConfig` | Infinite loop: sync → forward to hub → sleep → repeat |
| `forward_to_hub` | `&Client`, hub_url, sender, content | POST message to hub's A2A task endpoint (fire-and-forget) |

| Struct | Description |
|--------|-------------|
| `AgentLoopConfig` | `poll_interval_secs`, `hub_url`, `agent_name` |
| `HubTaskRequest` | JSON payload: `message` (String), `timeoutMs` (u64) |
| `HubTaskResponse` | JSON response: `id` (String), `state` (String) |

#### Hub Forwarding Protocol

Messages are forwarded to `POST {hub_url}/a2a/tasks/send` with payload:
```json
{
  "message": "[AlgoChat from SENDER_ADDRESS] MESSAGE_CONTENT",
  "timeoutMs": 300000
}
```

Forwarding is fire-and-forget: the agent logs the task ID but does not poll for completion. If the hub is unreachable, a warning is logged and the agent continues polling.

## Invariants

1. `--seed` must be exactly 32 bytes (64 hex characters) — exits with error otherwise
2. Identity is derived deterministically: same seed always produces same X25519 encryption key
3. The AlgoChat client uses in-memory storage (messages and keys are not persisted across restarts)
4. Logging is initialized via `tracing_subscriber` with `RUST_LOG` env filter, defaulting to `info`
5. The binary runs until `Ctrl+C` or message loop panic — `tokio::select!` on both
6. The indexer note-prefix filter `AQ` corresponds to AlgoChat protocol version 1 (first byte 0x01 → base64 `AQ`)
7. All Algorand API calls use reqwest with `X-Algo-API-Token` or `X-Indexer-API-Token` headers
8. Transaction confirmation polling retries once per second up to `rounds` attempts

## Behavioral Examples

### Scenario: Startup with seed and address

- **Given** `--seed 0102...3f40 --address ALGO...ADDR`
- **When** the binary starts
- **Then** it derives X25519 keys from the seed, logs the encryption public key, and starts polling for messages

### Scenario: Message received and forwarded

- **Given** the agent is running and an AlgoChat-encrypted message arrives on-chain
- **When** the sync loop picks up the transaction
- **Then** it decrypts the message, logs sender/recipient/round/content (truncated to 100 chars), and forwards to the hub via `POST /a2a/tasks/send`

### Scenario: Hub unreachable during forwarding

- **Given** the agent receives a valid message but the hub is down
- **When** forwarding is attempted
- **Then** it logs a warning and continues the polling loop (does not crash or block)

### Scenario: Indexer unreachable

- **Given** the indexer URL is wrong or the node is down
- **When** the sync loop attempts to poll
- **Then** it logs a warning and retries on the next interval (does not crash)

### Scenario: Ctrl+C shutdown

- **Given** a running nano agent
- **When** SIGINT is received
- **Then** logs "shutting down (ctrl+c)" and exits cleanly

## Error Cases

| Condition | Behavior |
|-----------|----------|
| Invalid hex in `--seed` | Exits with "Invalid seed hex" error |
| Seed not 32 bytes | Exits with "Seed must be exactly 32 bytes" error |
| Missing `--seed` or `--address` | clap prints help/error and exits with code 2 |
| Algorand node unreachable | sync loop logs warning and retries next interval |
| AlgoChat decryption failure | Message skipped, error logged |
| Hub API unreachable | Warning logged, loop continues |
| Hub returns non-2xx | Warning logged with status code, loop continues |

## Dependencies

### Consumes

| Module | What is used |
|--------|-------------|
| `corvid-core` | (currently unused, available for future AgentIdentity/NanoConfig integration) |
| `algochat` (rs-algochat) | `AlgoChat`, `AlgoChatConfig`, `AlgorandConfig`, `InMemoryKeyStorage`, `InMemoryMessageCache`, trait definitions |
| `reqwest` | HTTP client for Algorand API calls |
| `clap` | CLI argument parsing with `derive` and `env` features |
| `tokio` | Async runtime, signal handling, sleep |
| `hex` | Seed hex decoding |
| `data-encoding` | Base64 decoding for genesis hash |
| `async-trait` | Async trait implementations |

### Consumed By

None — this is the binary entry point.

## Change Log

| Date | Author | Change |
|------|--------|--------|
| 2026-03-28 | CorvidAgent | Initial spec — CLI skeleton with logging and graceful shutdown |
| 2026-03-28 | CorvidAgent | v2: Full implementation — HTTP Algorand clients, AlgoChat identity, message loop |
| 2026-03-28 | CorvidAgent | v3: Hub forwarding — messages forwarded to A2A tasks/send endpoint, unit tests added |
