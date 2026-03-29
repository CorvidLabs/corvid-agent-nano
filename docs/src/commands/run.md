# run

Start the agent and listen for AlgoChat messages.

```bash
can run [OPTIONS]
```

## Description

Starts the agent message loop that:
1. Polls for incoming AlgoChat messages on-chain
2. Decrypts messages from known PSK contacts
3. Forwards messages to the hub's A2A task endpoint (unless `--no-hub`)
4. Polls the hub for responses
5. Encrypts and sends replies back on-chain

## Options

| Flag | Default | Description |
|------|---------|-------------|
| `--network` | `localnet` | Network preset |
| `--algod-url` | from network | Override algod URL |
| `--algod-token` | from network | Override algod token |
| `--indexer-url` | from network | Override indexer URL |
| `--indexer-token` | from network | Override indexer token |
| `--seed` | from keystore | Agent seed (hex) |
| `--address` | from keystore | Agent Algorand address |
| `--password` | interactive | Keystore password |
| `--name` | `can` | Agent name for discovery |
| `--hub-url` | `http://localhost:3578` | Hub URL |
| `--poll-interval` | `5` | Seconds between polls |
| `--no-plugins` | `false` | Disable plugin host |
| `--no-hub` | `false` | P2P mode (no hub forwarding) |

## Examples

```bash
# Default: localnet, with hub
can run

# Testnet, custom hub
can run --network testnet --hub-url https://hub.example.com

# P2P mode (store messages locally only)
can run --no-hub

# Custom poll interval
can run --poll-interval 10

# With environment variables
CAN_NETWORK=testnet CAN_PASSWORD=mypass can run
```

## Startup output

On startup, CAN displays a summary showing:
- Agent name, network, address
- Contact and group counts
- Hub URL or P2P mode
- Plugin status

## Shutdown

Press `Ctrl+C` to gracefully shut down. The agent will stop the plugin host sidecar and exit cleanly.
