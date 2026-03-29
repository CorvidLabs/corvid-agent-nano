# import

Import an existing wallet from a mnemonic or hex seed.

```bash
can import [OPTIONS]
```

## Options

| Flag | Description |
|------|-------------|
| `--mnemonic <WORDS>` | 25-word Algorand mnemonic |
| `--seed <HEX>` | Hex-encoded 32-byte Ed25519 seed |
| `--password <PASSWORD>` | Keystore encryption password (min 8 chars) |

One of `--mnemonic` or `--seed` must be provided. If `--password` is not provided, prompts interactively.

## Examples

```bash
# Import from mnemonic
can import --mnemonic "abandon abandon ... about"

# Import from hex seed
can import --seed aabbccdd...

# Non-interactive
can import --seed aabbccdd... --password "mysecurepassword"
```

## Notes

- Fails if a wallet already exists in the data directory
- Prefer `can setup` for first-time configuration (it includes import as an option)
