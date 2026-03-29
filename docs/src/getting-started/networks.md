# Networks

CAN supports three Algorand network presets. Set the network with `--network` on any command that connects to the chain.

## Localnet (default)

Local sandbox for development. Start with `algokit localnet start`.

| Service | URL |
|---------|-----|
| Algod | `http://localhost:4001` |
| Indexer | `http://localhost:8980` |
| KMD | `http://localhost:4002` |

```bash
can run --network localnet
```

## Testnet

Algorand TestNet via Nodely public APIs. Free to use, requires testnet ALGO from the [dispenser](https://bank.testnet.algorand.network).

| Service | URL |
|---------|-----|
| Algod | `https://testnet-api.4160.nodely.dev` |
| Indexer | `https://testnet-idx.4160.nodely.dev` |

```bash
can run --network testnet
```

## Mainnet

Algorand MainNet via Nodely public APIs. Uses real ALGO.

| Service | URL |
|---------|-----|
| Algod | `https://mainnet-api.4160.nodely.dev` |
| Indexer | `https://mainnet-idx.4160.nodely.dev` |

```bash
can run --network mainnet
```

## Custom URLs

Override any network preset with explicit URLs:

```bash
can run \
  --network localnet \
  --algod-url http://custom-node:4001 \
  --algod-token mytoken \
  --indexer-url http://custom-indexer:8980 \
  --indexer-token mytoken
```

All URL overrides can also be set via environment variables:

| Variable | Description |
|----------|-------------|
| `CAN_NETWORK` | Network preset |
| `CAN_ALGOD_URL` | Algod URL override |
| `CAN_ALGOD_TOKEN` | Algod API token |
| `CAN_INDEXER_URL` | Indexer URL override |
| `CAN_INDEXER_TOKEN` | Indexer API token |
