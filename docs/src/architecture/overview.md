# Architecture Overview

## Project structure

```
corvid-agent-nano/
├── src/
│   ├── main.rs              # CLI entry point + command handlers
│   ├── ui.rs                # Terminal colors and formatting
│   ├── wizard.rs            # Interactive setup wizard
│   ├── agent.rs             # Message loop + hub forwarding
│   ├── algorand.rs          # HTTP clients (algod, indexer)
│   ├── contacts.rs          # PSK contact management (SQLite)
│   ├── groups.rs            # Group channel management (SQLite)
│   ├── keystore.rs          # Encrypted wallet (Argon2id + ChaCha20-Poly1305)
│   ├── storage.rs           # Key storage + message cache (SQLite)
│   ├── transaction.rs       # Algorand transaction building/signing
│   ├── wallet.rs            # Wallet generation + mnemonics
│   ├── bridge.rs            # JSON-RPC plugin host client
│   └── sidecar.rs           # Plugin host process manager
├── crates/
│   ├── corvid-plugin-sdk/   # WASM plugin SDK
│   ├── corvid-plugin-host/  # Plugin runtime host
│   ├── corvid-plugin-cli/   # Plugin CLI tools
│   └── corvid-plugin-macros/# Proc macros for plugins
├── plugins/                 # Example plugins
├── specs/                   # Module specifications
└── docs/                    # This documentation (mdBook)
```

## Message flow

```
                    ┌─────────────┐
                    │   Algorand  │
                    │  Blockchain │
                    └──────┬──────┘
                           │ AlgoChat txns
                    ┌──────▼──────┐
                    │  can agent  │
                    │  (message   │
                    │   loop)     │
                    └──┬──────┬───┘
                       │      │
              ┌────────▼─┐  ┌─▼────────┐
              │  SQLite   │  │   Hub    │
              │  (cache)  │  │  (A2A)   │
              └───────────┘  └──────────┘
```

## Key dependencies

| Crate | Purpose |
|-------|---------|
| `algochat` | AlgoChat protocol (encryption, key exchange, message format) |
| `clap` | CLI argument parsing with derive macros |
| `tokio` | Async runtime |
| `rusqlite` | SQLite for contacts, groups, messages |
| `argon2` | Password hashing (Argon2id) |
| `chacha20poly1305` | Authenticated encryption |
| `ed25519-dalek` | EdDSA signing for Algorand transactions |
| `wasmtime` | WebAssembly plugin runtime |
| `dialoguer` | Interactive terminal prompts |
| `colored` | Terminal color output |
