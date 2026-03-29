# Installation

## From crates.io (recommended)

```bash
cargo install corvid-agent-nano
```

This installs the `can` binary to `~/.cargo/bin/`.

## From source

```bash
# Clone the repository
git clone https://github.com/CorvidLabs/corvid-agent-nano.git
cd corvid-agent-nano

# Build and install
cargo install --path .
```

## From GitHub

```bash
cargo install --git https://github.com/CorvidLabs/corvid-agent-nano.git can
```

## Requirements

- **Rust** 1.75 or later
- An Algorand node (localnet, testnet, or mainnet)

### Setting up a local Algorand node

For development, use [AlgoKit](https://github.com/algorandfoundation/algokit-cli):

```bash
# Install AlgoKit
pipx install algokit

# Start a local Algorand sandbox
algokit localnet start
```

This starts algod on `localhost:4001` and indexer on `localhost:8980`.

## Verify installation

```bash
can --help
```

You should see the full command listing with all available subcommands.
