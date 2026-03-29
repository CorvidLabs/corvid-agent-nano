# setup / init

Interactive setup wizard for first-run configuration.

```bash
can setup [OPTIONS]
can init [OPTIONS]     # alias
```

## Description

Guides you through network selection, wallet creation or import, and password encryption. All steps can be driven by CLI flags for non-interactive use.

See [Setup Wizard](../getting-started/setup-wizard.md) for full details.

## Options

| Flag | Description |
|------|-------------|
| `--network <NETWORK>` | Network preset: `localnet`, `testnet`, `mainnet` |
| `--generate` | Generate a new wallet (non-interactive) |
| `--mnemonic <WORDS>` | Import from 25-word mnemonic |
| `--seed <HEX>` | Import from 64-char hex seed |
| `--password <PASSWORD>` | Keystore encryption password (min 8 chars) |

## Examples

```bash
# Interactive setup
can setup

# Non-interactive: generate wallet on localnet
can setup --network localnet --generate --password "mysecurepassword"

# Non-interactive: import from mnemonic on testnet
can setup --network testnet --mnemonic "abandon abandon ... about" --password "mysecurepassword"
```

## Errors

- **"Wallet already exists"** -- A keystore already exists. Delete `./data/keystore.enc` to re-run.
- **"Password must be at least 8 characters"** -- Choose a longer password.
