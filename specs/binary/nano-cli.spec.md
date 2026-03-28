---
module: can-cli
version: 3
status: active
files:
  - src/main.rs
depends_on:
  - specs/core/core.spec.md
  - specs/vault/vault.spec.md
  - specs/hub/hub.spec.md
  - specs/identity/identity.spec.md
  - specs/transaction/transaction.spec.md
  - external: algochat (git: https://github.com/CorvidLabs/rs-algochat)
---

# CAN CLI

## Purpose

Binary entry point for Corvid Agent Nano (CAN). Provides a subcommand-based CLI for managing agent identity, contacts, and running the AlgoChat message loop. Secrets are stored in an encrypted vault (Argon2id + ChaCha20-Poly1305) — no plaintext seeds, no env vars with secrets.

## Public API

### CLI Structure

Binary name: `can` (set in `Cargo.toml [[bin]]`)

```
can [--vault <path>] <subcommand>
```

### Global Flags

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--vault` | `PathBuf` | `~/.nano/vault.enc` | Path to encrypted vault file |

### Subcommands

| Subcommand | Description |
|------------|-------------|
| `init` | Generate a new identity and create encrypted vault |
| `run` | Decrypt vault, start AlgoChat message loop |
| `add-contact` | Add/update a contact with PSK in the vault |
| `remove-contact` | Remove a contact from the vault |
| `show-identity` | Display address and vault info (no secrets) |
| `list-contacts` | List contact names and addresses (no PSKs) |

### `run` Flags

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--algod-url` | `String` | `http://localhost:4001` | Algorand node REST API URL |
| `--algod-token` | `String` | `aaa...aaa` (64 a's) | Algorand node API token |
| `--indexer-url` | `String` | `http://localhost:8980` | Algorand indexer REST API URL |
| `--indexer-token` | `String` | `aaa...aaa` (64 a's) | Algorand indexer API token |
| `--name` | `String` | `nano` | Agent name for discovery |
| `--hub-url` | `String` | `http://localhost:3578` | corvid-agent hub API URL |
| `--poll-interval` | `u64` | `5` | Message poll interval in seconds |
| `--no-hub` | `bool` | `false` | Skip hub registration |

### `add-contact` Flags

| Flag | Type | Required | Description |
|------|------|----------|-------------|
| `--name` | `String` | yes | Contact name |
| `--address` | `String` | yes | Contact's Algorand address |
| `--psk` | `String` | no | PSK as base64 (interactive prompt if omitted) |
| `--psk-file` | `PathBuf` | no | Read PSK from file |

### `remove-contact` Arguments

| Argument | Type | Description |
|----------|------|-------------|
| `name` | `String` | Contact name to remove (positional) |

### Source Modules

| File | Description |
|------|-------------|
| `src/main.rs` | CLI parsing, subcommand dispatch, passphrase prompts |
| `src/vault.rs` | Encrypted vault (Argon2id + ChaCha20-Poly1305) |
| `src/identity.rs` | Seed generation, Algorand address derivation |
| `src/algorand.rs` | HTTP clients implementing rs-algochat traits |
| `src/agent.rs` | Message polling loop with hub forwarding |
| `src/hub.rs` | Flock Directory registration, A2A task forwarding |
| `src/transaction.rs` | Algorand transaction construction and signing |

## Invariants

1. Vault passphrase is always read via `rpassword` (no echo) — never from CLI args or env vars
2. `init` refuses to overwrite an existing vault — user must delete manually
3. `run` zeroizes vault contents and seed bytes from memory after extracting what's needed
4. `show-identity` and `list-contacts` never print secrets (seed, PSKs)
5. `add-contact` with an existing name replaces the old entry (update semantics)
6. The binary runs until `Ctrl+C` or message loop panic — `tokio::select!` on both
7. Logging is initialized via `tracing_subscriber` with `RUST_LOG` env filter, defaulting to `info`
8. Hub registration failure is non-fatal — agent runs without hub in degraded mode

## Behavioral Examples

### Scenario: First-time setup

- **Given** no vault exists at `~/.nano/vault.enc`
- **When** `can init` is run
- **Then** prompts for passphrase (with confirmation), generates Ed25519 seed, derives Algorand address, creates encrypted vault, prints the address

### Scenario: Run with vault

- **Given** a vault exists with identity and contacts
- **When** `can run` is executed
- **Then** prompts for passphrase, decrypts vault, initializes AlgoChat client, registers with hub, starts polling for messages

### Scenario: Add contact interactively

- **Given** a vault exists
- **When** `can add-contact --name corvid-agent --address ALGO...` is run without `--psk`
- **Then** prompts for PSK (no echo), validates base64, prompts for vault passphrase, stores contact

### Scenario: Init with existing vault

- **Given** a vault already exists at the path
- **When** `can init` is run
- **Then** exits with error: "Vault already exists... Delete it first"

## Error Cases

| Condition | Behavior |
|-----------|----------|
| Vault already exists on `init` | Exits with error |
| No vault found on `run`/`add-contact`/etc. | Exits with "Run `can init` first" |
| Wrong passphrase | Exits with "Decryption failed — wrong passphrase?" |
| Invalid base64 PSK | Exits with "Invalid base64 PSK" |
| Empty passphrase on `init` | Reprompts |
| Passphrase mismatch on `init` | Reprompts |
| Hub unreachable on `run` | Warns and continues without hub |
| AlgoChat sync failure | Warns and retries next interval |

## Dependencies

### Consumes

| Module | What is used |
|--------|-------------|
| `vault` | `Vault`, `VaultContents`, `Contact` — encrypted secret storage |
| `identity` | `generate_seed`, `address_from_seed` — key generation |
| `agent` | `run_message_loop`, `AgentLoopConfig` — message processing |
| `algorand` | `HttpAlgodClient`, `HttpIndexerClient` — Algorand API adapters |
| `hub` | `HubClient` — Flock Directory and A2A forwarding |
| `algochat` (external) | `AlgoChat`, `AlgoChatConfig`, `AlgorandConfig`, `InMemoryKeyStorage`, `InMemoryMessageCache` |
| `clap` | CLI argument parsing |
| `rpassword` | Secure passphrase input |
| `zeroize` | Memory zeroization |

### Consumed By

None — this is the binary entry point.

## Change Log

| Date | Author | Change |
|------|--------|--------|
| 2026-03-28 | CorvidAgent | Initial spec — CLI skeleton with logging and graceful shutdown |
| 2026-03-28 | CorvidAgent | v2: Full implementation — HTTP Algorand clients, AlgoChat identity, message loop |
| 2026-03-28 | CorvidAgent | v3: Renamed binary to `can`, vault-based secret management, subcommand CLI, hub integration |
