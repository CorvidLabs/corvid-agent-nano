# Changelog

All notable changes to corvid-agent-nano are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-04-06

### Added

- **MCP server mode** (`can mcp`) — JSON-RPC 2.0 MCP server for Claude Code, Cursor, and other MCP-compatible clients
  - Exposes 5 tools: `agent_info`, `list_contacts`, `get_inbox`, `check_balance`, `send_message`
  - Support for `--network`, `--password`, and `--seed` options
- **Balance command** (`can balance`) — Check agent wallet balance
- **History command** (`can history`) — View transaction history
- **Plugin dependency graph** — Plugins can declare dependencies; host resolves load order automatically
- **DbRead host function** — Plugins can read from the agent's SQLite database
- **FsProjectDir host function** — Plugins can resolve project-relative paths
- **Health check endpoint** (`--health-port`) — HTTP health endpoint for Docker/systemd monitoring
- **JSON logging** (`--log-format json`) — Structured log output for aggregation pipelines

### Changed

- **Enhanced status output** — `can status` now shows balance, registered state, and network details
- **Fund command improvements** — Better error messages and localnet faucet handling
- **Register command improvements** — Clearer registration flow with hub
- **spec-sync upgraded to v3.3.0** — Improved spec validation

### Security

- **Path traversal fix** (critical) — Blocked `../` sequences in plugin file access
- **SSRF bypass fixes** (critical) — Blocked IPv6-mapped IPv4 (`::ffff:*`), link-local (`fe80::/10`), and ULA (`fc00::/8`) addresses
- **Wall-clock timeout enforcement** — Plugin host enforces actual time limits, not just fuel/instruction limits
- **Socket buffer bounds** — Capped incoming message buffer at 64 MiB to prevent heap exhaustion
- **Input validation hardening** — Route parameter validation (`PLUGIN_ID_RE`, `TOOL_NAME_RE`) and JSON parse error reporting
- **Plugin bridge hardening** — Unix socket permissions, request size limits, REST invoke route validation
- **Plugin tool discovery** — Fixed tool listing and enforced invocation timeout

### Fixed

- Plugin bridge wire type normalization and hot-reload push notifications
- Plugin tool discovery returning empty results
- Clippy and fmt CI failures

## [0.1.0] - 2026-03-29

### Added

- **Terminal UI** — Colored output for all 14 subcommands with enhanced readability
- **mdBook documentation** — Comprehensive 28-page guide covering commands, architecture, guides, and reference
- **GitHub Pages deployment** — Automated docs build and deployment workflow (docs.yml)
- **Plugin system (WASM)** — Full WebAssembly plugin infrastructure with three trust tiers
  - `corvid-plugin-sdk` — Plugin SDK for writing WASM plugins
  - `corvid-plugin-host` — Sidecar plugin runtime (WASM interpreter with sandboxing)
  - `corvid-plugin-cli` — Plugin scaffolding and validation tools
  - `corvid-plugin-macros` — Derive macros for plugin development
  - Example `hello-world` plugin included
- **Interactive setup wizard** — Guided wallet and configuration setup with comprehensive validation
- **Fund command** — Fund agent wallet from faucet (localnet) or dispenser link (testnet)
- **Register command** — Register agent with Flock Directory for peer discovery
- **Group channels** — Broadcast encrypted messages to multiple agents via group PSKs
  - `can groups create` — Create a new group
  - `can groups add-member` — Add members to group
  - `can groups remove-member` — Remove members
  - `can groups show` — View group details
  - `can groups list` — List all groups
- **Status command** — Check agent, network, and hub connectivity status
- **Inbox command** — View and manage received messages
- **Send command** — Send direct messages to contacts
- **P2P mode** — Run agent without hub forwarding (--no-hub flag)
- **Multi-network support** — localnet (default), testnet, and mainnet with preset configurations
- **Encrypted keystore** — Argon2id key derivation + ChaCha20-Poly1305 encryption for wallet storage
- **Wallet management**
  - `can setup` / `can init` — Create new wallet with recovery phrase
  - `can import` — Import wallet from mnemonic or seed
  - `can info` — Display wallet and agent details
- **Contact management** — Store and manage PSK-encrypted contacts
  - `can contacts add` — Add encrypted contact
  - `can contacts list` — List all contacts
  - `can contacts remove` — Delete contact
  - `can contacts export` — Backup contacts to JSON
  - `can contacts import` — Restore contacts from JSON
- **Change password** — Rotate keystore encryption password
- **Hub integration** — Connect to corvid-agent platform for AI-powered responses
- **Bidirectional AlgoChat messaging** — Send and receive encrypted messages on-chain
- **SQLite persistence** — Message cache and contact storage
- **Flock Directory integration** — Agent discovery and reputation system
- **Environment variable support** — Configure via `CAN_*` env vars (e.g., `CAN_NETWORK`, `CAN_PASSWORD`)

### Changed

- **Architecture refactor** — Folded corvid-core into main crate, reframed as CLI tool
- **Binary name** — Codebase prepared for crates.io publication

### Security

- **CI hardening** — Restricted GitHub Actions permissions
- **Wasmtime upgrade** — Updated from v22 to v27 (resolves 6 CVEs in WASM runtime)
- **Key logging protection** — Truncated sensitive keys in debug output
- **CodeQL integration** — Static analysis on all PRs

### Fixed

- Spec-sync validation in CI
- Self-hosted runner integration

## Earlier Versions

### [v0.0.1] - Initial Release

- Single-binary Algorand AI agent
- AlgoChat messaging (X25519 + ChaCha20-Poly1305)
- Basic wallet and contact management
- Hub forwarding support

---

For detailed information on each feature, see the [documentation](https://corvidlabs.github.io/corvid-agent-nano/).
