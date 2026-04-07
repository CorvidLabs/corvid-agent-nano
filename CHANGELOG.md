# Changelog

All notable changes to corvid-agent-nano are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-04-06

### Added

- **Balance command** (`can balance`) — Quick ALGO balance check without running full status
- **History command** (`can history`) — View message history filtered by contact (alias for inbox with `--contact`)
- **Fund command** (`can fund`) — Fund agent wallet from localnet faucet or testnet dispenser link
- **Register command** (`can register`) — Register agent with Flock Directory for peer discovery
- **MCP server mode** (`can mcp`) — Start as a JSON-RPC 2.0 MCP server for Claude Code, Cursor, and other MCP-compatible clients
  - Exposes 5 tools: `agent_info`, `list_contacts`, `get_inbox`, `check_balance`, `send_message`
  - Comprehensive MCP integration guide with Claude Code and Cursor setup instructions
- **Health check endpoint** — HTTP health check for Docker/systemd monitoring (`--health-port`)
- **JSON logging** — Structured JSON log output for log aggregation (`--log-format json`)
- **Plugin dependency graph** — Plugins can declare dependencies; host resolves load order
- **Plugin host functions** — `DbRead` and `FsProjectDir` host functions for plugin data access
- **Plugin tool discovery** — Automatic discovery of tools exposed by loaded plugins
- **Plugin hot-reload notifications** — Push notifications when plugins are updated
- **Enhanced status output** — Status command now shows balance, contact count, and plugin health

### Security

- **Path traversal fix** (critical) — Blocked `../` in plugin IDs and file paths
- **SSRF bypass fixes** (critical) — Blocked IPv6-mapped IPv4 (`::ffff:*`), link-local (`fe80::/10`), and ULA (`fc00::/8`) addresses
- **Wall-clock timeout enforcement** — Plugin host enforces actual time limits, not just fuel/instruction limits
- **Socket permissions** — Plugin bridge socket restricted to owner-only access
- **Request size limits** — Capped incoming plugin bridge messages at 64 MiB
- **Input validation hardening** — Route parameter validation (`PLUGIN_ID_RE`, `TOOL_NAME_RE`) and JSON parse error reporting
- **Plugin bridge hardening** — REST invoke route validation and sandboxing improvements

### Changed

- **16 CLI commands** — Added `balance`, `history`, `fund`, `register` (was 12 in v0.1.0)
- **Wire type normalization** — Standardized plugin bridge JSON-RPC wire types
- **spec-sync upgraded to v3.3.0** — Improved spec validation and sync tooling

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
