# Commands Overview

The `can` CLI binary provides the following commands:

## Wallet Management

| Command | Description |
|---------|-------------|
| [`setup`](./setup.md) | Interactive setup wizard (alias: `init`) |
| [`import`](./import.md) | Import wallet from mnemonic or hex seed |
| [`change-password`](./change-password.md) | Change keystore encryption password |
| [`info`](./info.md) | Show agent identity and wallet info |

## Messaging

| Command | Description |
|---------|-------------|
| [`run`](./run.md) | Start the agent and listen for messages |
| [`send`](./send.md) | Send an encrypted message |
| [`inbox`](./inbox.md) | Read cached messages |

## Contacts & Groups

| Command | Description |
|---------|-------------|
| [`contacts`](./contacts.md) | Manage PSK contacts (add, remove, list, export, import) |
| [`groups`](./groups.md) | Manage group channels (create, members, export, import) |

## Infrastructure

| Command | Description |
|---------|-------------|
| [`fund`](./fund.md) | Fund wallet from localnet faucet or show instructions |
| [`register`](./register.md) | Register agent with the hub |
| [`status`](./status.md) | Health check (algod, indexer, hub, balance, plugins) |
| [`plugin`](./plugin.md) | Manage WASM plugins |

## Global Flags

All commands accept:

| Flag | Default | Description |
|------|---------|-------------|
| `--data-dir` | `./data` | Data directory for persistent storage |
