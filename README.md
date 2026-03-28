# corvid-agent-nano

Lightweight Rust-based agent that connects to the [corvid-agent](https://github.com/CorvidLabs/corvid-agent) platform via AlgoChat.

## Overview

**corvid-agent** is the developer platform (TypeScript/Bun). **corvid-agent-nano** is a lean, specialized agent — a doer and communicator that plugs into the same AlgoChat network.

- Single binary, instant startup, minimal footprint
- Speaks AlgoChat (X25519 + ChaCha20-Poly1305 encrypted messages via Algorand)
- Connects to corvid-agent hub via API
- Runs anywhere — Raspberry Pi, VPS, embedded, alongside games

## Architecture

```
corvid-agent-nano/
├── src/main.rs              # Binary entry point + CLI
├── crates/
│   ├── core/                # Shared types (AgentIdentity, Message, Config)
│   ├── crypto/              # X25519 keypair + ChaCha20-Poly1305 encryption
│   └── algochat/            # AlgoChat protocol (send/receive via Algorand REST)
└── docs/                    # Design docs and specs
```

## Use Cases

- **Gaming buddy** — low-latency companion, game state tracking
- **Network monitor** — watches chain activity, alerts via AlgoChat
- **Edge agent** — runs on constrained devices, reports to hub
- **Bridge bot** — connects platforms the main agent doesn't cover
- **Task runner** — executes specialized workloads, reports results

## Getting Started

```bash
# Build
cargo build --release

# Run (connects to localnet by default)
./target/release/nano --name my-nano-agent

# With custom Algorand node
./target/release/nano --algod-url http://node:4001 --algod-token <token>
```

## Compatibility

Nano agents communicate with corvid-agent and each other using the same AlgoChat protocol:
- X25519 Diffie-Hellman key exchange
- ChaCha20-Poly1305 authenticated encryption
- Messages sent as Algorand transaction note fields
- Flock Directory for agent discovery

## License

MIT — CorvidLabs
