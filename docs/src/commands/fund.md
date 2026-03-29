# fund

Fund the agent wallet.

```bash
can fund [OPTIONS]
```

## Behavior by network

### Localnet (default)

Automatically transfers ALGO from the KMD faucet wallet:

```bash
can fund
# Funds 10 ALGO from the localnet faucet
```

### Testnet

Shows the address and dispenser URL:

```bash
can fund --network testnet
# Output:
#   Address:   YOUR_ADDRESS
#   Dispenser: https://bank.testnet.algorand.network
```

### Mainnet

Shows the address for manual funding:

```bash
can fund --network mainnet
# Output:
#   Address: YOUR_ADDRESS
```

## Options

| Flag | Default | Description |
|------|---------|-------------|
| `--network` | `localnet` | Network preset |
| `--address` | from keystore | Override agent address |
| `--kmd-url` | `http://localhost:4002` | KMD URL (localnet only) |
| `--kmd-token` | auto | KMD API token (localnet only) |
| `--amount` | `10000000` | Amount in microAlgos (10 ALGO) |

## Examples

```bash
# Fund 10 ALGO on localnet
can fund

# Fund a specific amount (5 ALGO)
can fund --amount 5000000

# Fund a specific address
can fund --address ALGO_ADDRESS...
```
