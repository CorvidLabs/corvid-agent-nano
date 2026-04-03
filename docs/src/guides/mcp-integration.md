# MCP Integration: Use corvid-agent-nano with Claude Code and Cursor

Model Context Protocol (MCP) makes corvid-agent-nano available as a tool server to Claude-based IDEs and editors. This guide shows how to set up MCP integration with Claude Code and Cursor.

## What You Get

Once configured, your Claude or Cursor assistant will have access to these capabilities:

- **Agent Info** — Wallet address, contact count, cached messages
- **List Contacts** — View all saved contacts
- **Get Inbox** — Read recent messages (with optional filtering)
- **Check Balance** — Look up your ALGO balance on-chain
- **Send Message** — Compose and send encrypted AlgoChat messages

Your assistant can now draft messages, check your wallet status, and retrieve context from your message history without leaving the editor.

## Prerequisites

- `can` installed (Rust binary): `cargo install --git https://github.com/CorvidLabs/corvid-agent-nano --bin can`
- A corvid-agent-nano wallet with `can setup` already completed
- Claude Code, Cursor, or another MCP-compatible editor
- Your wallet password or seed hex ready

## Setup for Claude Code

### Step 1: Locate Your Config

Claude Code reads MCP server configs from:
- **macOS/Linux:** `~/.claude/mcp.json`
- **Windows:** `%APPDATA%\Claude\mcp.json`

If the file doesn't exist, create it.

### Step 2: Add the Server Config

Add corvid-agent-nano to your `mcp.json`:

```json
{
  "mcpServers": {
    "corvid-agent-nano": {
      "command": "can",
      "args": ["mcp", "--network", "localnet", "--password", "your_password_here"],
      "disabled": false
    }
  }
}
```

For **testnet** (real network):
```json
{
  "mcpServers": {
    "corvid-agent-nano": {
      "command": "can",
      "args": ["mcp", "--network", "testnet", "--password", "your_password_here"],
      "disabled": false
    }
  }
}
```

Replace `your_password_here` with your actual keystore password.

### Step 3: Restart Claude Code

Close and reopen Claude Code. The MCP server will start automatically.

### Step 4: Verify Connection

Ask Claude to check your balance or list contacts. If it works, you're connected!

```
What's my current ALGO balance?
```

## Setup for Cursor

Cursor uses the same MCP configuration format. Add the same config to:
- **macOS/Linux:** `~/.cursor/mcp.json`
- **Windows:** `%APPDATA%\Cursor\mcp.json`

Then restart Cursor.

## Security Best Practices

### Avoid Plaintext Passwords

Instead of hardcoding your password in `mcp.json`, use an environment variable:

```json
{
  "mcpServers": {
    "corvid-agent-nano": {
      "command": "can",
      "args": ["mcp", "--network", "testnet", "--password", "$CORVID_PASSWORD"],
      "disabled": false
    }
  }
}
```

Then set the variable in your shell profile:

**macOS/Linux:**
```bash
# Add to ~/.bashrc, ~/.zshrc, or ~/.profile
export CORVID_PASSWORD="your_password"
```

**Windows (PowerShell):**
```powershell
[Environment]::SetEnvironmentVariable("CORVID_PASSWORD", "your_password", "User")
```

After adding it, restart your terminal and IDE.

### Use Seed Instead of Password

For even better security, export your wallet seed hex and use `--seed`:

```bash
# First, get your seed hex from your wallet
can setup --show-seed

# Then update your MCP config
{
  "mcpServers": {
    "corvid-agent-nano": {
      "command": "can",
      "args": ["mcp", "--network", "testnet", "--seed", "$CORVID_SEED"],
      "disabled": false
    }
  }
}
```

And set `CORVID_SEED` in your environment.

### Localnet for Development

When developing, use `localnet` instead of testnet — no real funds at risk:

```json
{
  "mcpServers": {
    "corvid-agent-nano": {
      "command": "can",
      "args": ["mcp", "--network", "localnet", "--password", "$CORVID_PASSWORD"],
      "disabled": false
    }
  }
}
```

Requires: `algokit localnet start` running in the background.

## Troubleshooting

### MCP Server Not Starting

**Symptom:** Claude/Cursor doesn't recognize the tool.

**Fix:** Verify the `can` binary is in your PATH:
```bash
which can
```

If not found, install it:
```bash
cargo install --git https://github.com/CorvidLabs/corvid-agent-nano --bin can
```

### "Network Unreachable" Errors

**Symptom:** Tools like `check_balance` and `send_message` fail with network errors.

**Fix:**
- For **localnet:** Ensure `algokit localnet start` is running
- For **testnet:** Check your internet connection and verify algod is accessible
- Try a simpler tool first (e.g., `agent_info`) to isolate network issues

### "Missing Password or Seed"

**Symptom:** `send_message` and `check_balance` fail with authentication errors.

**Fix:** Make sure `--password` or `--seed` is set in your MCP config. Note: `agent_info`, `list_contacts`, and `get_inbox` don't require authentication.

### MCP Config Syntax Errors

**Symptom:** IDE complains about invalid JSON.

**Fix:** Validate your JSON:
```bash
jq . ~/.claude/mcp.json  # macOS/Linux
```

Common issues:
- Trailing commas in JSON arrays/objects
- Unescaped backslashes in paths
- Single quotes instead of double quotes

## Advanced: Multiple Instances

If you have multiple wallets or networks, configure multiple servers:

```json
{
  "mcpServers": {
    "corvid-agent-nano-localnet": {
      "command": "can",
      "args": ["mcp", "--network", "localnet", "--password", "$CORVID_PASSWORD_LOCAL"],
      "disabled": false
    },
    "corvid-agent-nano-testnet": {
      "command": "can",
      "args": ["mcp", "--network", "testnet", "--password", "$CORVID_PASSWORD_TESTNET"],
      "disabled": false
    }
  }
}
```

This lets you switch between networks by asking your assistant which one to use.

## Examples

### Get Wallet Status
```
What's my ALGO balance and how many contacts do I have?
```

### Send a Message
```
Send a message to alice@example saying "Hello from Claude!"
```

### Check Recent Messages
```
Show me my last 5 messages from bob.
```

### List All Contacts
```
Give me a list of all my contacts with their addresses.
```
