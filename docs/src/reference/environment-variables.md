# Environment Variables

All environment variables are optional. CLI flags take precedence.

## Network configuration

| Variable | Description | Default |
|----------|-------------|---------|
| `CAN_NETWORK` | Network preset: `localnet`, `testnet`, `mainnet` | `localnet` |
| `CAN_ALGOD_URL` | Override algod URL | from network |
| `CAN_ALGOD_TOKEN` | Override algod API token | from network |
| `CAN_INDEXER_URL` | Override indexer URL | from network |
| `CAN_INDEXER_TOKEN` | Override indexer API token | from network |

## Identity

| Variable | Description | Default |
|----------|-------------|---------|
| `CAN_SEED` | Agent seed (hex-encoded 32 bytes) | from keystore |
| `CAN_ADDRESS` | Agent Algorand address | from keystore |
| `CAN_PASSWORD` | Keystore password | interactive prompt |

## Logging

| Variable | Description | Default |
|----------|-------------|---------|
| `RUST_LOG` | Log level filter | `info` |

Examples:
```bash
# Show debug logs
RUST_LOG=debug can run

# Show only warnings
RUST_LOG=warn can run

# Module-specific logging
RUST_LOG=corvid_agent_nano=debug,algochat=info can run
```

## Docker / CI usage

```bash
CAN_NETWORK=testnet \
CAN_PASSWORD=mypassword \
CAN_SEED=aabbccdd... \
CAN_ADDRESS=ALGO_ADDRESS... \
can run
```
