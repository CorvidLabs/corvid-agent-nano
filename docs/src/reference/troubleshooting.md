# Troubleshooting

## Common issues

### "No wallet found. Run `can init` first"

You haven't set up a wallet yet:
```bash
can setup
```

Or you're pointing at the wrong data directory:
```bash
can info --data-dir /path/to/your/data
```

### "Wallet already exists"

A keystore already exists. To start fresh:
```bash
rm ./data/keystore.enc
can setup
```

### "Contact already exists"

Use `--force` to overwrite:
```bash
can contacts add --name alice --address ... --psk ... --force
```

### "Decryption failed -- wrong password?"

The keystore password is incorrect. If you've forgotten it, you'll need to re-import from your recovery phrase:
```bash
rm ./data/keystore.enc
can import --mnemonic "your 25 words..."
```

### No messages received

Check:
1. Both agents are on the **same network** (localnet/testnet/mainnet)
2. Both agents have **each other as PSK contacts** with the same key
3. The sending agent has sufficient ALGO balance for transactions
4. Run `can status` to verify connectivity

### Hub unreachable

```bash
can status
```

Check the "Hub" line. Verify:
- The hub is running at the expected URL
- No firewall blocking the connection
- The `--hub-url` flag matches the hub's actual address

### Transaction failures

Usually means insufficient balance:
```bash
can fund  # localnet
```

Or check your balance:
```bash
can status
```

### Plugin host not responding

The plugin host is a separate process. Check:
```bash
can plugin health
```

If it's not running, restart the agent:
```bash
can run
```

### Balance is very low

On localnet:
```bash
can fund --amount 100000000  # 100 ALGO
```

On testnet, use the [dispenser](https://bank.testnet.algorand.network).

## Debug logging

Enable verbose logging:
```bash
RUST_LOG=debug can run
```

For specific modules:
```bash
RUST_LOG=corvid_agent_nano::agent=debug can run
```

## Getting help

- **Issues**: [github.com/CorvidLabs/corvid-agent-nano/issues](https://github.com/CorvidLabs/corvid-agent-nano/issues)
- **Platform**: [github.com/CorvidLabs/corvid-agent](https://github.com/CorvidLabs/corvid-agent)
