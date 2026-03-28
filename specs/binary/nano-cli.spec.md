---
module: nano-cli
version: 1
status: draft
files:
  - src/main.rs
depends_on:
  - specs/core/core.spec.md
  - specs/crypto/crypto.spec.md
  - specs/algochat/algochat.spec.md
---

# Nano CLI

## Purpose

Binary entry point for corvid-agent-nano. Parses CLI arguments, initializes logging, and orchestrates the agent's lifecycle: crypto identity setup, Algorand node connection, Flock Directory registration, and the AlgoChat message loop. Provides a single-binary, instant-startup agent that connects to the corvid-agent ecosystem.

## Public API

### CLI Arguments (clap)

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--algod-url` | `String` | `http://localhost:4001` | Algorand node REST API URL |
| `--algod-token` | `String` | `aaa...aaa` (64 a's) | Algorand node API token (localnet default) |
| `--name` | `String` | `nano` | Agent name for discovery and display |
| `--hub-url` | `String` | `http://localhost:3578` | corvid-agent hub API URL |

### Structs

| Struct | Description |
|--------|-------------|
| `Cli` | clap `Parser` struct holding all CLI arguments |

### Functions

| Function | Parameters | Returns | Description |
|----------|-----------|---------|-------------|
| `main` | `()` | `Result<()>` | Async entry point — parses args, initializes agent, runs event loop |

## Invariants

1. Logging is initialized via `tracing_subscriber` with `RUST_LOG` env filter, defaulting to `info` level
2. The binary runs until `Ctrl+C` (`tokio::signal::ctrl_c`) — no other shutdown mechanism currently
3. CLI argument parsing happens before any I/O or initialization
4. The `--algod-token` default is the standard Algorand localnet token (64 'a' characters)
5. All errors propagate via `anyhow::Result` — the binary exits with a non-zero code on unhandled errors

## Behavioral Examples

### Scenario: Default startup on localnet

- **Given** no CLI arguments provided
- **When** `nano` binary is executed
- **Then** it connects to `http://localhost:4001` with localnet token, names itself "nano", logs "starting corvid-agent-nano" and "nano agent ready", then waits for Ctrl+C

### Scenario: Custom name and hub

- **Given** `--name scout --hub-url http://hub.example.com:3578`
- **When** the binary starts
- **Then** the agent identifies as "scout" and targets the specified hub URL

### Scenario: Ctrl+C shutdown

- **Given** a running nano agent
- **When** the user sends SIGINT (Ctrl+C)
- **Then** it logs "shutting down" and exits cleanly with code 0

## Error Cases

| Condition | Behavior |
|-----------|----------|
| Invalid CLI arguments | clap prints help/error and exits with code 2 |
| Logging init failure | Panic (tracing_subscriber failure is unrecoverable) |
| Algorand node unreachable at startup | Currently no-op (TODO: health check on startup) |

## Dependencies

### Consumes

| Module | What is used |
|--------|-------------|
| `corvid-core` | `AgentIdentity`, `Message`, `NanoConfig` (future) |
| `corvid-crypto` | `KeyPair` for identity initialization (future) |
| `corvid-algochat` | `AlgoChatClient` for node connectivity (future) |
| `clap` | `Parser` derive macro for CLI argument parsing |
| `tokio` | Async runtime, `signal::ctrl_c` for graceful shutdown |
| `tracing` | `info!` macro for structured logging |
| `tracing-subscriber` | `fmt()` with `EnvFilter` for log initialization |
| `anyhow` | `Result` for error propagation |

### Consumed By

None — this is the binary entry point.

## Change Log

| Date | Author | Change |
|------|--------|--------|
| 2026-03-28 | CorvidAgent | Initial spec — CLI skeleton with logging and graceful shutdown |
