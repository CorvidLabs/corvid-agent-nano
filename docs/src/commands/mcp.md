# MCP Server Mode

Start corvid-agent-nano as a JSON-RPC 2.0 MCP (Model Context Protocol) server over stdin/stdout. This makes it available as an MCP server for Claude Code, Cursor, and other MCP-compatible clients.

## Usage

```bash
can mcp [OPTIONS]
```

## Options

| Option | Description |
|--------|-------------|
| `--network NETWORK` | Algorand network: `localnet`, `testnet`, or `mainnet` (default: `localnet`) |
| `--password PASSWORD` | Keystore password for unlocking wallet |
| `--seed HEX` | Wallet seed phrase in hex format (alternative to password) |

## Examples

**Start MCP server with testnet and password prompt:**
```bash
can mcp --network testnet --password mypassword
```

**Start MCP server with localnet (no network access required):**
```bash
can mcp --network localnet --password mypassword
```

**Using a hex seed instead of password:**
```bash
can mcp --network testnet --seed abc123def456...
```

## Exposed Tools

The MCP server exposes five tools for use by MCP clients:

### 1. `agent_info`
Get local agent information without network access.

**Inputs:** None

**Returns:**
- `wallet_address` — Agent's ALGO wallet address
- `contacts_count` — Number of saved contacts
- `messages_cached` — Number of messages in local cache

**Network required:** No

### 2. `list_contacts`
Retrieve all saved contacts with their addresses.

**Inputs:** None

**Returns:**
- `contacts` — Array of contact objects:
  - `name` — Contact name
  - `address` — Algorand address

**Network required:** No

### 3. `get_inbox`
Retrieve recent cached messages, optionally filtered by sender.

**Inputs:**
- `from` (optional) — Filter by sender address or contact name
- `limit` (optional) — Maximum number of messages to return (default: 10)

**Returns:**
- `messages` — Array of message objects:
  - `from` — Sender's address or name
  - `body` — Message content
  - `timestamp` — Message receive time

**Network required:** No

### 4. `check_balance`
Query ALGO balance on-chain.

**Inputs:** None

**Returns:**
- `balance_microalgos` — Balance in microALGO (1 ALGO = 1,000,000 microALGO)
- `balance_algo` — Balance formatted as ALGO

**Network required:** Yes (algod connection)

### 5. `send_message`
Encrypt and send an AlgoChat message to a contact or address.

**Inputs:**
- `recipient` — Contact name or Algorand address
- `message` — Message body (plain text)

**Returns:**
- `transaction_id` — Transaction ID on-chain
- `status` — Send status confirmation

**Network required:** Yes (algod + keystore)

**Authentication:** Either `--password` or `--seed` must be provided at startup.

## MCP Client Configuration

### Claude Code / Cursor

Add this to your MCP client config:

```json
{
  "mcpServers": {
    "corvid-agent-nano": {
      "command": "can",
      "args": ["mcp", "--network", "testnet", "--password", "your_password"],
      "disabled": false
    }
  }
}
```

Replace `your_password` with your actual keystore password, or use a shell variable:
```bash
"args": ["mcp", "--network", "testnet", "--password", "$CORVID_PASSWORD"]
```

Then export the environment variable before starting your MCP client:
```bash
export CORVID_PASSWORD=your_password
```

### Stdin/Stdout Protocol

The MCP server communicates via JSON-RPC 2.0 over stdin/stdout. All requests and responses are newline-delimited JSON.

**Example request:**
```json
{"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "agent_info"}, "id": 1}
```

## Security Notes

- The `--password` flag is passed on the command line, making it visible in process listings. For production use, consider:
  - Using `--seed` with a hex-encoded seed in an environment variable
  - Running the server with restricted file permissions
  - Using a dedicated service account with limited wallet access
- MCP servers run headless (no interactive prompts). If password/seed is missing, tools requiring authentication return an error.
- Network-dependent tools (`check_balance`, `send_message`) require algod connectivity. Verify your network configuration before use.

## Troubleshooting

**"Missing password or seed" error:**
Ensure `--password` or `--seed` is provided when using `send_message` or `check_balance`.

**"Network unreachable" error:**
Verify your Algorand network is reachable. For `localnet`, ensure `algokit localnet start` is running. For testnet/mainnet, check your internet connection.

**Tool appears to hang:**
MCP servers have a timeout. If operations take longer than expected, check your network latency and algod server status.
