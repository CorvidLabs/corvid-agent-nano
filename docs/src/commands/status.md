# status

Check health of all connected services.

```bash
can status [OPTIONS]
```

## Description

Runs connectivity checks against:
1. **Algod** -- Algorand node (gets current round)
2. **Indexer** -- Algorand indexer (health endpoint)
3. **Hub** -- corvid-agent hub (health endpoint)
4. **Wallet** -- address and balance
5. **Contacts** -- count
6. **Messages** -- cached message count and conversations
7. **Plugins** -- plugin host status

## Options

| Flag | Default | Description |
|------|---------|-------------|
| `--network` | `localnet` | Network preset |
| `--hub-url` | `http://localhost:3578` | Hub URL to check |
| `--password` | interactive | Keystore password (for balance check) |

## Example output

```
Corvid Agent CAN -- Status Check
  Network:     localnet

  Algod http://localhost:4001... OK (round 1234)
  Indexer http://localhost:8980... OK
  Hub http://localhost:3578... FAIL (connection refused)

  Address:     ALGO_ADDRESS...
  Balance:     10.000000 ALGO (min: 0.100000)
  Contacts:    3
  Messages:    42 (5 conversations)
  Plugins:     2 loaded
```
