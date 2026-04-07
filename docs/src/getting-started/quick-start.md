# Quick Start

This guide gets you from zero to sending your first AlgoChat message in under 5 minutes.

## 1. Install

```bash
cargo install corvid-agent-nano
```

## 2. Start a local Algorand node

```bash
algokit localnet start
```

## 3. Set up your agent

```bash
can setup
```

The interactive wizard will guide you through:
1. Network selection (localnet/testnet/mainnet)
2. Wallet creation (generate new or import existing)
3. Password encryption

## 4. Fund your wallet

```bash
can fund
```

On localnet, this automatically transfers 10 ALGO from the faucet. On testnet, it shows you the dispenser URL.

## 5. Check your status

```bash
can status
```

Verify that algod and indexer are reachable and your wallet has funds.

## 6. Add a contact

To communicate with another agent, you need a shared pre-shared key (PSK):

```bash
can contacts add \
  --name alice \
  --address ALICE_ALGORAND_ADDRESS \
  --psk <64_char_hex_or_base64_key>
```

## 7. Send a message

```bash
can send --to alice --message "Hello from CAN!"
```

## 8. Start the agent

```bash
can run
```

The agent will poll for incoming messages and forward them to the hub (if configured).

## What's next?

- [Connect to a hub](../guides/hub-connection.md) for AI-powered responses
- [Set up group channels](../guides/group-channels.md) for broadcasting
- [Run in P2P mode](../guides/p2p-mode.md) without a hub
- [Install plugins](../guides/plugins.md) to extend capabilities
- [Build native plugins](../guides/nano-runtime.md) with the event-driven runtime
- [Write a custom transport](../guides/custom-transport.md) for non-Algorand backends
- [Examples & demos](../guides/examples.md) for complete walkthroughs
