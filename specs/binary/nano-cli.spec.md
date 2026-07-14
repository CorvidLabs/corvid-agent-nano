---
module: nano-cli
version: 10
status: stable
files:
  - src/main.rs
  - src/agent.rs
  - src/algorand.rs
  - src/transaction.rs
  - src/a2a.rs
  - src/algochat_transport.rs
  - src/bridge.rs
  - src/config.rs
  - src/groups.rs
  - src/mcp.rs
  - src/sidecar.rs
  - src/ui.rs
  - src/wizard.rs
depends_on:
  - specs/core/core.spec.md
  - specs/binary/transaction.spec.md
  - CorvidLabs/rs-algochat@algochat
---

# Nano CLI

## Purpose

Binary entry point for corvid-agent-nano. Parses CLI arguments, initializes crypto identity from an Ed25519 seed, creates HTTP-based Algorand clients, starts the bidirectional AlgoChat message loop, and runs until Ctrl+C. Provides a single-binary, instant-startup agent that connects to the corvid-agent ecosystem via the AlgoChat protocol on Algorand. Supports full two-way messaging: receives encrypted messages on-chain, forwards them to the corvid-agent hub for processing, polls for the hub's response, then encrypts and sends the reply back on-chain.

## Public API

### Exported Structs

| Struct | Description |
|--------|-------------|
| `AgentLoopConfig` | Configuration for the message loop: poll interval, hub URL, agent name, agent address, signing key |
| `HttpAlgodClient` | HTTP adapter implementing `algochat::AlgodClient` for algod v2 REST API |
| `HttpIndexerClient` | HTTP adapter implementing `algochat::IndexerClient` for indexer v2 REST API |

### Exported Functions

| Function | Parameters | Returns | Description |
|----------|-----------|---------|-------------|
| `run_message_loop` | `Arc<AlgoChat<...>>`, `Arc<AlgodClient>`, `AgentLoopConfig` | `!` | Infinite loop: sync → forward to hub → poll response → encrypt reply → send on-chain → sleep → repeat |
| `send_reply` | `algod`, `message_id`, `response`, `config` | async | Sends an encrypted reply message back on-chain after hub processing |
| `new` | `base_url: &str`, `token: &str` | `Self` | Constructor for `HttpAlgodClient` and `HttpIndexerClient` |
| `decode` | `s: &str` | `Result<Vec<u8>, DecodeError>` | Decode a base64 string to bytes |
| `send_note_transaction` | `algod`, `sender`, `receiver`, `note`, `signing_key` | `Result<String>` | Build, sign, and submit a 0-ALGO payment transaction |
| `decode_address` | `address: &str` | `Result<[u8; 32]>` | Decode Algorand address to 32 raw bytes |
| `DiscoveredAgent` | — | struct | Agent discovered from on-chain registration records |
| `ChainMessage` | — | struct | On-chain AlgoChat message representation |
| `scan_all_algochat` | client parameters | async result | Scan the chain for AlgoChat messages |
| `discover_agents` | client parameters | async result | Discover registered agents from chain state |
| `build_payment_transaction_with_amount` | transaction parameters | encoded transaction | Build a payment transaction with an explicit amount |

### CLI Subcommands

| Command | Description |
|---------|-------------|
| `setup` / `init` | Interactive wallet setup wizard |
| `import` | Import wallet from mnemonic or seed |
| `run` | Start the agent message loop |
| `send` | Send direct message to a contact or group |
| `inbox` | View and manage received messages |
| `history` | View message history filtered by contact (alias for inbox with --contact) |
| `balance` | Quick ALGO balance check |
| `status` | Check agent, network, and hub connectivity |
| `contacts` | Manage PSK-encrypted contacts (add, list, remove, export, import) |
| `groups` | Manage group PSK channels (create, add-member, remove-member, show, list) |
| `change-password` | Rotate keystore encryption password |
| `info` | Display wallet and agent details |
| `fund` | Fund wallet from faucet (localnet) or dispenser (testnet) |
| `register` | Register agent with Flock Directory for peer discovery |
| `mcp` | Start MCP server for Claude Code / Cursor integration |
| `plugin` | List, invoke, and manage WASM plugins |

### Global CLI Arguments (clap)

| Flag | Type | Default | Env Var | Description |
|------|------|---------|---------|-------------|
| `--data-dir` | `String` | `./data` | — | Data directory for persistent SQLite storage |
| `--log-format` | `text\|json` | `text` | — | Log output format |
| `--log-level` | `String` | `info` | `RUST_LOG` | Log level override |

### Run Command Arguments

| Flag | Type | Default | Env Var | Description |
|------|------|---------|---------|-------------|
| `--network` | `Network` | `localnet` | `CAN_NETWORK` | Algorand network preset |
| `--algod-url` | `String` | (from preset) | `CAN_ALGOD_URL` | Algorand node REST API URL |
| `--algod-token` | `String` | (from preset) | `CAN_ALGOD_TOKEN` | Algorand node API token |
| `--indexer-url` | `String` | (from preset) | `CAN_INDEXER_URL` | Algorand indexer REST API URL |
| `--indexer-token` | `String` | (from preset) | `CAN_INDEXER_TOKEN` | Algorand indexer API token |
| `--seed` | `String` | (from keystore) | `CAN_SEED` | Hex-encoded 32-byte Ed25519 private key |
| `--address` | `String` | (from keystore) | `CAN_ADDRESS` | Agent's Algorand address |
| `--password` | `String` | (interactive) | `CAN_PASSWORD` | Keystore password |
| `--name` | `String` | `can` | — | Agent name for discovery and display |
| `--hub-url` | `String` | `http://localhost:3578` | — | corvid-agent hub API URL |
| `--poll-interval` | `u64` | `5` | — | Message poll interval in seconds |
| `--no-plugins` | `bool` | `false` | — | Disable the plugin host sidecar |
| `--no-hub` | `bool` | `false` | — | Run in direct P2P mode (no hub forwarding) |
| `--health-port` | `u16` | (disabled) | `CAN_HEALTH_PORT` | Enable health check HTTP endpoint on this port |

### Source Modules

| File | Description |
|------|-------------|
| `src/main.rs` | CLI parsing, identity init, client wiring, shutdown |
| `src/algorand.rs` | `HttpAlgodClient` and `HttpIndexerClient` — HTTP adapters for rs-algochat traits |
| `src/agent.rs` | `run_message_loop` — bidirectional message loop: receive, forward, poll, reply |
| `src/transaction.rs` | Algorand transaction building, signing, and submission (see transaction.spec.md) |
| `src/a2a.rs` | Local A2A HTTP routes, task storage, agent-card responses, and hub forwarding |
| `src/algochat_transport.rs` | Runtime transport adapter between Nano messages and the AlgoChat protocol |
| `src/bridge.rs` | Plugin-host bridge client and plugin discovery/invocation models |
| `src/config.rs` | Persistent Nano configuration defaults, parsing, and serialization |
| `src/groups.rs` | Persistent group membership and pre-shared-key management |
| `src/mcp.rs` | MCP server transport and Nano tool dispatch |
| `src/sidecar.rs` | Plugin-host sidecar discovery, launch, socket readiness, and shutdown |
| `src/ui.rs` | Terminal output helpers used by interactive and non-interactive commands |
| `src/wizard.rs` | First-run wallet import/generation and network-selection workflow |

### Exported Supporting API

| Export | Contract |
|--------|----------|
| `AgentCard` | Serializable A2A agent-card representation. |
| `AgentCapabilities` | Capabilities advertised by the A2A endpoint. |
| `AgentSkill` | One skill advertised in an agent card. |
| `AgentAuth` | Authentication metadata advertised for the agent. |
| `TaskState` | A2A task lifecycle state. |
| `Task` | Stored A2A task and its current response state. |
| `CreateTaskRequest` | Validated request body for A2A task creation. |
| `TaskStore` | Capacity-bounded, identifier-addressable task storage. |
| `insert` | Insert a task while enforcing store capacity. |
| `get` | Read a task by identifier. |
| `get_mut` | Mutably access a task while it is being advanced. |
| `list` | Return stored tasks newest first. |
| `A2aServerConfig` | Bind address, advertised identity, and optional hub forwarding configuration. |
| `serve_a2a` | Serve the documented A2A routes until shutdown. |
| `AlgoChatTransport` | Runtime transport adapter backed by AlgoChat and the configured signing identity. |
| `PluginInfo` | Plugin metadata returned by the host bridge. |
| `HealthStatus` | Typed plugin-host health response. |
| `PluginBridge` | Unix-socket client for plugin lifecycle and invocation requests. |
| `connect` | Connect a bridge client to the configured sidecar socket. |
| `list_plugins` | Return the plugins currently reported by the host. |
| `invoke` | Invoke a named tool on a loaded plugin. |
| `load_plugin` | Request host validation and loading for a plugin artifact. |
| `unload_plugin` | Request orderly unloading of a plugin. |
| `health` | Query the sidecar health response. |
| `NanoConfig` | Root persisted Nano configuration. |
| `AgentConfig` | Agent identity and local behavior settings. |
| `NetworkConfig` | Algod and indexer endpoint settings. |
| `HubConfig` | Optional hub-forwarding settings. |
| `RuntimeConfig` | Runtime polling and service settings. |
| `PluginsConfig` | Plugin enablement and per-plugin configuration. |
| `LoggingConfig` | Log format and level settings. |
| `load` | Load configuration from the default location. |
| `load_from` | Parse configuration from an explicit path. |
| `save` | Persist configuration to the default location. |
| `save_to` | Persist configuration to an explicit path. |
| `plugin_config` | Resolve configuration for one plugin. |
| `for_new_agent` | Construct the initial configuration for a validated agent identity. |
| `Group` | Named group and its pre-shared-key metadata. |
| `GroupMember` | Addressed member of a group. |
| `GroupStore` | Persistent group and membership storage. |
| `open` | Open the persistent group database. |
| `in_memory` | Open an isolated in-memory group store. |
| `create` | Create a group with a generated key. |
| `create_with_psk` | Create a group using validated supplied key material. |
| `remove` | Remove a group and its memberships. |
| `add_member` | Add a non-duplicate member to an existing group. |
| `remove_member` | Remove a member from an existing group. |
| `members` | List the members of a group. |
| `count` | Count stored groups. |
| `export_json` | Serialize group data in the documented exchange format. |
| `import_json` | Validate and import group exchange data. |
| `serve_health` | Serve the lightweight Nano health endpoint. |
| `cmd_mcp` | Start the MCP command transport and dispatch Nano tools. |
| `SidecarConfig` | Plugin-host binary, data-directory, socket, and startup settings. |
| `SidecarHandle` | Owned child-process handle for the running host. |
| `shutdown` | Stop the owned sidecar process cleanly. |
| `socket_path` | Derive the plugin-host socket path from its data directory. |
| `find_plugin_host_binary` | Resolve an executable plugin-host binary from supported locations. |
| `spawn_sidecar` | Launch the plugin host with the configured arguments. |
| `wait_for_socket` | Wait up to the startup deadline for socket readiness. |
| `header` | Render a CLI section heading. |
| `success` | Render a successful CLI result. |
| `error` | Render a CLI error. |
| `warn` | Render a CLI warning. |
| `field` | Render a labeled CLI field. |
| `separator` | Render a terminal separator. |
| `table_header` | Render a terminal table heading. |
| `banner` | Render the Nano startup banner. |
| `dir_arrow` | Render the directional marker used in message views. |
| `balance` | Render an ALGO balance consistently. |
| `WizardConfig` | Non-interactive and interactive setup inputs. |
| `WizardResult` | Validated identity and network result from setup. |
| `run_wizard` | Generate or import a wallet and select its network settings. |
| `check_first_run` | Detect whether first-run setup is required. |

### algorand.rs — HTTP Trait Implementations

| Struct | Implements | Description |
|--------|-----------|-------------|
| `HttpAlgodClient` | `algochat::AlgodClient` | HTTP client for algod v2 REST API |
| `HttpIndexerClient` | `algochat::IndexerClient` | HTTP client for indexer v2 REST API |

#### Constructors

| Method | Parameters | Description |
|--------|-----------|-------------|
| `HttpAlgodClient::new` | `base_url: &str`, `token: &str` | Create a new HTTP algod client with the given URL and API token |
| `HttpIndexerClient::new` | `base_url: &str`, `token: &str` | Create a new HTTP indexer client with the given URL and API token |

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

### agent.rs — Bidirectional Message Loop

| Function | Parameters | Description |
|----------|-----------|-------------|
| `run_message_loop` | `Arc<AlgoChat<...>>`, `Arc<AlgodClient>`, `AgentLoopConfig` | Bidirectional loop: sync → forward to hub → poll response → encrypt reply → send on-chain |
| `forward_to_hub` | `&Client`, hub_url, sender, content | POST message to hub's A2A task endpoint. Returns task ID or None |
| `poll_hub_task` | `&Client`, hub_url, task_id | Poll `GET /a2a/tasks/{id}` until completed/failed/cancelled. Returns response text |
| `send_reply` | `&AlgoChat`, `&AlgodClient`, sender, recipient, message, signing_key | Encrypt reply (PSK or X25519) and submit as 0-ALGO payment transaction |

| Struct | Description |
|--------|-------------|
| `AgentLoopConfig` | `poll_interval_secs`, `hub_url`, `agent_name`, `agent_address`, `signing_key` |
| `HubTaskRequest` | JSON payload: `message` (String), `timeoutMs` (u64) |
| `HubTaskResponse` | JSON response: `id` (String), `state` (String) |
| `HubTaskStatus` | Full task status: `state` (String), `response` (Option<String>) |

#### Hub Protocol

**Step 1 — Forward:** POST to `{hub_url}/a2a/tasks/send`:
```json
{
  "message": "[AlgoChat from SENDER_ADDRESS] MESSAGE_CONTENT",
  "timeoutMs": 300000
}
```

**Step 2 — Poll:** GET `{hub_url}/a2a/tasks/{task_id}` every 3 seconds (up to 100 attempts / ~5 minutes) until `state` is `completed`, `failed`, or `cancelled`.

**Step 3 — Reply:** If the hub returns a `response` string, encrypt it for the original sender and submit as a 0-ALGO Algorand payment transaction with the encrypted message in the note field. Uses PSK encryption if the sender is a known PSK contact, otherwise standard X25519.

If any step fails (hub unreachable, no response, encryption failure, transaction rejection), a warning is logged and the loop continues with the next message.

## Invariants

1. `--seed` must be exactly 32 bytes (64 hex characters) — exits with error otherwise
2. Identity is derived deterministically: same seed always produces same X25519 encryption key
3. The AlgoChat client uses SQLite storage (`data_dir/keys.db` and `data_dir/messages.db`) — messages, keys, and sync-round bookmarks persist across restarts
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

### Scenario: Message received, processed, and reply sent

- **Given** the agent is running and an AlgoChat-encrypted message arrives on-chain
- **When** the sync loop picks up the transaction
- **Then** it decrypts the message, forwards to the hub, polls for the response, encrypts the reply for the sender, and submits it as a 0-ALGO transaction on-chain

### Scenario: Hub unreachable during forwarding

- **Given** the agent receives a valid message but the hub is down
- **When** forwarding is attempted
- **Then** it logs a warning and continues the polling loop (no reply is sent)

### Scenario: Hub task times out

- **Given** the agent has forwarded a message and is polling for the response
- **When** the hub does not complete within ~5 minutes (100 polls at 3-second intervals)
- **Then** it logs a warning and continues with the next message

### Scenario: Reply encryption with PSK contact

- **Given** the sender is a known PSK contact
- **When** the agent sends a reply
- **Then** it uses PSK encryption with ratcheted counter (not standard X25519)

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
| Data directory creation fails | Exits with filesystem error |
| SQLite database cannot be opened | Exits with "Failed to open key storage" / "Failed to open message cache" |
| Hub API unreachable | Warning logged, no reply sent, loop continues |
| Hub returns non-2xx | Warning logged with status code, no reply sent, loop continues |
| Hub task fails or is cancelled | Warning logged, no reply sent, loop continues |
| Hub task poll times out | Warning logged after 100 attempts, loop continues |
| Recipient encryption key not found | Warning logged, reply not sent |
| PSK encryption failure | Warning logged, reply not sent |
| Transaction submission failure | Warning logged with error, reply not sent |

## Dependencies

### Consumes

| Module | What is used |
|--------|-------------|
| `corvid-core` | `SqliteKeyStorage`, `SqliteMessageCache` for persistent storage |
| `algochat` (rs-algochat) | `AlgoChat`, `AlgoChatConfig`, `AlgorandConfig`, trait definitions |
| `reqwest` | HTTP client for Algorand API calls |
| `clap` | CLI argument parsing with `derive` and `env` features |
| `tokio` | Async runtime, signal handling, sleep |
| `hex` | Seed hex decoding |
| `data-encoding` | Base64 decoding for genesis hash, base32 for Algorand addresses |
| `async-trait` | Async trait implementations |
| `ed25519-dalek` | Ed25519 signing key derivation and transaction signing |
| `sha2` | SHA-512/256 for Algorand address checksums |
| `rmp` | Low-level msgpack encoding for Algorand transactions |

### Consumed By

None — this is the binary entry point.

## Change Log

| Date | Author | Change |
|------|--------|--------|
| 2026-03-28 | CorvidAgent | Initial spec — CLI skeleton with logging and graceful shutdown |
| 2026-03-28 | CorvidAgent | v2: Full implementation — HTTP Algorand clients, AlgoChat identity, message loop |
| 2026-03-28 | CorvidAgent | v3: Hub forwarding — messages forwarded to A2A tasks/send endpoint, unit tests added |
| 2026-03-28 | CorvidAgent | v4: SQLite persistence — replace in-memory storage with SqliteKeyStorage/SqliteMessageCache, add --data-dir flag |
| 2026-03-28 | CorvidAgent | v5: Add Exported Structs/Functions sections for spec-sync strict compliance |
| 2026-03-28 | CorvidAgent | v6: Bidirectional messaging — hub response polling, encrypted on-chain replies, PSK/X25519 encryption, transaction building |
| 2026-04-06 | CorvidAgent | v7→8: Update CLI subcommands table to reflect all 16 commands (balance, history, fund, register, mcp, plugin added) |
| 2026-07-14 | SpecSync | CHG-0001-adopt-specsync-5-0-1-and-the-unified-trust-1-0-0-governance-gate: Adopt SpecSync 5.0.1 and the unified Trust 1.0.0 governance gate |
