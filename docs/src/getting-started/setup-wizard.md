# Setup Wizard

The `can setup` (or `can init`) command runs an interactive wizard that guides you through initial configuration.

## Interactive mode

```bash
can setup
```

The wizard will prompt for:

1. **Network** -- localnet, testnet, or mainnet
2. **Wallet** -- generate a new wallet or import an existing one
3. **Password** -- encrypts your wallet with Argon2id + ChaCha20-Poly1305

After completion, it prints next steps specific to your chosen network.

## Non-interactive mode

All wizard steps can be driven by CLI flags for CI/automation:

```bash
# Generate a new wallet on testnet
can setup --network testnet --generate --password "your_secure_password"

# Import from mnemonic
can setup --network localnet --mnemonic "word1 word2 ... word25" --password "your_password"

# Import from hex seed
can setup --network mainnet --seed <64_hex_chars> --password "your_password"
```

## Flags

| Flag | Description |
|------|-------------|
| `--network` | Network preset: `localnet`, `testnet`, `mainnet` |
| `--generate` | Generate a new wallet (non-interactive) |
| `--mnemonic` | Import from 25-word Algorand mnemonic |
| `--seed` | Import from hex-encoded 32-byte Ed25519 seed |
| `--password` | Password for keystore encryption (min 8 chars) |
| `--data-dir` | Data directory (default: `./data`) |

## Recovery phrase

When generating a new wallet, the wizard displays a 25-word recovery phrase. **Write it down and store it securely** -- it is the only way to recover your wallet if you lose the keystore file or forget your password.

## Re-running setup

If a wallet already exists in the data directory, `can setup` will refuse to overwrite it. To start fresh:

```bash
rm ./data/keystore.enc
can setup
```
