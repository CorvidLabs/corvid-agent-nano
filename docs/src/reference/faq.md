# Frequently Asked Questions

## Installation & Setup

### How do I install corvid-agent-nano?

Two options:

**From crates.io (recommended):**
```bash
cargo install corvid-agent-nano
```

**From source:**
```bash
git clone https://github.com/CorvidLabs/corvid-agent-nano.git
cd corvid-agent-nano
cargo install --path .
```

The binary is called `can`.

### What version of Rust do I need?

Rust 1.75 or later. Check your version:
```bash
rustc --version
```

Update with:
```bash
rustup update
```

### Do I need to run a local Algorand node?

For **localnet** (default): Yes, use [AlgoKit](https://algokit.io):
```bash
algokit localnet start
```

For **testnet/mainnet**: No, the agent uses public nodes. You can optionally run your own node for privacy.

### How do I choose a network?

```bash
# localnet (for testing, default)
can setup --network localnet

# testnet (for staging/testing with real Algo)
can setup --network testnet

# mainnet (for production)
can setup --network mainnet
```

Localnet is free and instant. Testnet uses real Algo (get from [faucet](https://testnet.algoexplorer.io/dispenser)).

## Wallets & Keys

### What's the difference between setup and import?

- **`can setup`** — Create a new wallet and generate a recovery phrase
- **`can import`** — Import an existing wallet from a recovery phrase or seed

Use `import` if you already have a wallet you want to restore.

### What's a recovery phrase (mnemonic)?

A 25-word phrase that uniquely identifies your wallet. It's the only way to recover your wallet if you lose your keystore file or forget your password.

**Protect it:**
- Write it down on paper
- Store offline (not in files or email)
- Never share it
- If compromised, move your funds to a new wallet immediately

### What if I lose my recovery phrase?

If you've already saved your keystore file and know your password, you're fine. The recovery phrase only matters if you lose the keystore.

To prevent disaster, write it down when you first create your wallet:
```bash
can setup
# The recovery phrase is displayed — write it down!
```

### How do I change my password?

```bash
can change-password --data-dir ~/.corvid
```

This creates a new encrypted keystore with the new password. Your recovery phrase doesn't change.

### Can I export my seed or recovery phrase later?

Not with the current CLI. If you need it, restore from your written-down recovery phrase:

```bash
can import --mnemonic "word1 word2 ... word25" --password new_password
```

### What does "Wallet already exists" mean?

A keystore file already exists in your data directory. To start fresh:

```bash
rm ~/.corvid/keystore.enc
can setup
```

This destroys the old wallet — **make sure you have a backup recovery phrase!**

## Messaging

### Why aren't I receiving messages?

Check:

1. **Same network?** Both agents must be on localnet/testnet/mainnet
   ```bash
   can info  # Check your network
   ```

2. **Mutual contacts?** Both agents must have each other as PSK contacts
   ```bash
   can contacts list
   ```

3. **Same PSK?** Both must use the exact same pre-shared key
   - If not, regenerate one:
   ```bash
   can groups create --name shared
   # Share the PSK with the other agent
   ```

4. **Running?** The agent must be running to receive messages
   ```bash
   can run
   ```

5. **Funded?** Your account must have ALGO for transactions
   ```bash
   can fund  # On localnet, auto-funds from faucet
   ```

### What's a pre-shared key (PSK)?

A 32-byte secret shared between two agents that enables encrypted messaging. It's established out-of-band (you share it manually, not over the network).

Generate one with:
```bash
can groups create --name my-group
# Copy the PSK and share securely
```

Provide it as hex (64 chars) or base64 (44 chars):
```bash
can contacts add --name alice --address ALGO_ADDRESS --psk <PSK>
```

### Can I send messages without a hub?

Yes, use P2P mode:
```bash
can run --no-hub
```

This stores messages locally without forwarding to a hub. Useful for testing or direct agent-to-agent communication.

### What if my hub is unreachable?

Check connectivity:
```bash
can status
```

The output shows hub reachability. If unreachable:
- Is the hub running? `can run` on the hub machine
- Is the URL correct? Check `--hub-url` flag (default: `http://localhost:3578`)
- Firewall blocking? Allow traffic on the hub port

For production, use HTTPS to prevent MITM attacks.

### Can I send messages to multiple agents?

Yes, use groups:
```bash
can groups create --name team
can groups add-member --group team --address ALICE...
can groups add-member --group team --address BOB...

# Send to all members at once
can send --to team --message "Hello team!"
```

Each member of the group can decrypt (they all share the group PSK).

## Hub Integration

### How do I connect to a corvid-agent hub?

1. **Get the hub's Algorand address** from its administrator
2. **Create a PSK contact on the hub** via API:
   ```bash
   curl -X POST http://hub:3000/api/algochat/psk/contacts \
     -H "Content-Type: application/json" \
     -d '{"name": "my-agent", "address": "YOUR_ADDRESS"}'
   ```
3. **Add the hub as a contact** on your side:
   ```bash
   can contacts add --name hub \
     --address HUB_ADDRESS \
     --psk PSK_FROM_STEP_2
   ```
4. **Run with hub forwarding**:
   ```bash
   can run --hub-url http://hub:3578
   ```

The agent will forward messages to the hub for processing.

### What does "hub forwarding" mean?

When the agent receives a message:
1. It decrypts the message
2. Sends it to the hub's A2A endpoint
3. Waits for the hub to process (AI reasoning, tool calls, etc.)
4. Encrypts the hub's response
5. Sends the response back on-chain

Without a hub, messages are just stored locally.

### Can I run without a hub?

Yes:
```bash
can run --no-hub
```

The agent will receive and store messages but won't forward them anywhere. Useful for testing or dedicated agent instances.

## Plugins

### What are plugins?

WASM modules that extend agent capabilities. They can:
- Send/receive messages
- Query blockchain
- Make HTTP requests
- Store data
- Define custom tools

Load them with:
```bash
can plugin load my-plugin.wasm
can plugin invoke my-plugin my-tool '{"param": "value"}'
```

### How do I create a plugin?

See [Plugin Development Guide](../guides/plugin-development.md) for a complete walkthrough.

Quick start:
```bash
cargo new --lib my-plugin
# Edit Cargo.toml and src/lib.rs
cargo build --target wasm32-wasip1 --release
cp target/wasm32-wasip1/release/my_plugin.wasm ~/.corvid/plugins/
can run
can plugin list
```

### What trust tiers mean?

When loading a plugin, specify how much you trust it:

| Tier | Memory | Timeout | Use |
|------|--------|---------|-----|
| `trusted` | 512 MiB | 60s | Your own plugins |
| `verified` | 128 MiB | 30s | Code-reviewed plugins |
| `untrusted` | 32 MiB | 10s | Unknown plugins (default) |

```bash
can plugin load my-plugin.wasm --tier trusted
```

Higher-trust plugins get more resources. Always use the least-permissive tier.

### Can plugins access my wallet or keys?

No. Plugins run in a sandbox with no access to:
- Your keystore or signing key
- Your recovery phrase
- Other plugins' storage
- Raw network or filesystem

They can only use JSON-RPC APIs (messaging, storage, HTTP, Algorand queries).

### How do I disable plugins?

```bash
can run --no-plugins
```

Plugins won't be loaded or executed.

### Can plugins run code from the internet?

No, plugins are WASM binaries you install locally. They can't download code at runtime.

However, plugins can make HTTP requests to external APIs:
```bash
ctx.http_get("https://api.example.com/data")
```

The agent should maintain an HTTP allowlist for security.

## Data & Storage

### Where is my data stored?

By default: `./data/` (relative to current directory)

Change the location:
```bash
can setup --data-dir ~/.corvid
can run --data-dir ~/.corvid
```

**Directory structure:**
```
~/.corvid/
├── keystore.enc       # Encrypted wallet
├── contacts.db        # SQLite contacts
├── messages.db        # SQLite message cache
└── plugins/           # WASM plugins
```

### How do I back up my wallet?

Save your recovery phrase:
```bash
can setup
# Write down the 25-word phrase
```

You can also copy the keystore file:
```bash
cp ~/.corvid/keystore.enc ~/backup/
```

To restore:
```bash
can import --mnemonic "your 25 words..."
```

### Can I use the same wallet on multiple machines?

Yes:

1. **Get recovery phrase** from first machine:
   ```bash
   # It's shown during setup — write it down if you didn't
   ```

2. **Import on second machine**:
   ```bash
   can setup --network testnet --mnemonic "your 25 words..."
   ```

Both machines will use the same Algorand address but have separate keystores (encrypted with different passwords).

### Is my data encrypted?

- **Keystore** — Yes (Argon2id + ChaCha20-Poly1305)
- **Messages** — Yes (in transit, ChaCha20-Poly1305)
- **Contacts** — No, stored in plaintext SQLite
- **Plugins** — No, loaded in plaintext

Consider full-disk encryption for production machines.

### Can I export my contacts?

```bash
can contacts export --output contacts.json
```

This creates a JSON file with all your contacts (cleartext).

Import on another machine:
```bash
can contacts import --file contacts.json
```

## Troubleshooting

### Agent crashes on startup

Check logs:
```bash
RUST_LOG=debug can run
```

Common causes:
- Corrupted database — Delete `~/.corvid/*.db` and re-run
- Missing algod/indexer — Start localnet: `algokit localnet start`
- Wrong network — Check `--network` flag

### "Decryption failed — wrong password?"

Your password is incorrect. Options:
1. Try the correct password
2. If forgotten, restore from recovery phrase:
   ```bash
   rm ~/.corvid/keystore.enc
   can import --mnemonic "your 25 words..."
   ```

### "Contact already exists"

Use `--force` to overwrite:
```bash
can contacts add --name alice --address ADDR --psk KEY --force
```

### Transaction failures

Common causes:
- **Insufficient balance** — Fund your account: `can fund`
- **Wrong network** — Ensure agent and contacts are on same network
- **Transaction too large** — Messages > 1 KB are chunked automatically

### Plugin timeouts

If a plugin tool times out:
- It took longer than the tier's limit (10-60s)
- Optimize the code or break it into smaller tasks
- Consider a higher trust tier for more time

### "Can't connect to Algorand node"

Localnet users:
```bash
algokit localnet start
```

Testnet/mainnet users:
- Check internet connection
- Verify node URL is correct
- Try a different public node

### CPU usage is high

Likely causes:
- Agent polling too frequently — Use larger `--poll-interval`
- Plugin running expensive computation — Optimize or reduce frequency
- Hub unreachable causing retries — Fix hub connection

Try:
```bash
can run --poll-interval 10
```

## Features & Roadmap

### What can the agent do?

See [Introduction](../introduction.md) for full feature list. Quick summary:
- Send/receive encrypted messages
- Hub integration (forward to AI)
- P2P direct agent-to-agent communication
- Group channels for broadcasting
- WASM plugins for extending capabilities
- Contact and wallet management
- Multi-network support (localnet/testnet/mainnet)

### Can the agent sign transactions?

Yes, it can build and submit Algorand transactions. See [Plugin Development Guide](../guides/plugin-development.md) for the `ctx.submit_transaction()` API.

### Can the agent access smart contracts?

Through the Algorand query APIs (via plugins). No direct smart contract calling yet, but you can query contract state.

### What's planned next?

See [GitHub issues](https://github.com/CorvidLabs/corvid-agent-nano/issues) for upcoming features and vote on your favorites!

## Contributing

### How do I report a bug?

Create a [GitHub Issue](https://github.com/CorvidLabs/corvid-agent-nano/issues) with:
- Steps to reproduce
- Expected vs actual behavior
- Version: `can --version`
- Logs: `RUST_LOG=debug can run`

### How do I suggest a feature?

Open a [GitHub Discussion](https://github.com/CorvidLabs/corvid-agent-nano/discussions) to discuss before implementing.

### How do I contribute code?

See [CONTRIBUTING.md](../../CONTRIBUTING.md) for full guidelines. Quick summary:
1. Fork the repo
2. Create a branch (`feature/name`)
3. Make changes and test
4. Open a PR
5. Address review feedback

## Getting Help

- **Documentation** — You're reading it!
- **Issues** — [GitHub Issues](https://github.com/CorvidLabs/corvid-agent-nano/issues) for bugs
- **Discussions** — [GitHub Discussions](https://github.com/CorvidLabs/corvid-agent-nano/discussions) for questions
- **Security** — security@corvidlabs.io for vulnerability reports

## More Questions?

Can't find an answer? [Open a discussion](https://github.com/CorvidLabs/corvid-agent-nano/discussions) — we're here to help!
