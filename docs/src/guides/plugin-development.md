# Plugin Development Guide

This guide walks you through creating a custom WASM plugin for corvid-agent-nano using the `corvid-plugin-sdk`.

## Overview

Plugins extend agent capabilities by running in a sandboxed WebAssembly environment. They can:
- Send and receive encrypted messages
- Query Algorand blockchain state
- Perform HTTP requests (with allowlist)
- Store persistent key-value data
- Define tools that other agents can invoke

## Quick Start

### Prerequisites

- **Rust** — 1.75+ with `wasm32-wasip1` target installed
- **corvid-agent-nano** — Build and install the CLI

### Step 1: Create a plugin project

```bash
cargo new --lib my-plugin
cd my-plugin
```

Edit `Cargo.toml`:

```toml
[package]
name = "my-plugin"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]  # Required for WASM output

[dependencies]
corvid-plugin-sdk = { path = "/path/to/corvid-agent-nano/crates/corvid-plugin-sdk" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

### Step 2: Write your plugin

Edit `src/lib.rs`:

```rust
use corvid_plugin_sdk::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct GreetInput {
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GreetOutput {
    pub message: String,
}

#[corvid_plugin]
struct MyPlugin;

#[corvid_tool(name = "greet", description = "Greet someone by name")]
fn greet(input: GreetInput) -> Result<GreetOutput, String> {
    Ok(GreetOutput {
        message: format!("Hello, {}!", input.name),
    })
}
```

### Step 3: Build

```bash
cargo build --lib --target wasm32-wasip1 --release
```

The WASM binary appears at: `target/wasm32-wasip1/release/my_plugin.wasm`

### Step 4: Load into your agent

```bash
# Copy to plugins directory
mkdir -p ~/.corvid/plugins
cp target/wasm32-wasip1/release/my_plugin.wasm ~/.corvid/plugins/

# Start the agent
can run

# In another terminal, invoke the plugin
can plugin invoke my-plugin greet '{"name": "Leif"}'
```

Expected output:
```json
{
  "message": "Hello, Leif!"
}
```

## SDK Concepts

### #[corvid_plugin] Macro

Marks the plugin's entry point. Must be applied to exactly one struct per plugin:

```rust
#[corvid_plugin]
struct MyPlugin;
```

The struct name is used to identify your plugin but doesn't need to match the crate name.

### #[corvid_tool] Macro

Defines a tool (function) that other agents can invoke:

```rust
#[corvid_tool(name = "tool-name", description = "What this tool does")]
fn tool_name(input: ToolInput) -> Result<ToolOutput, String> {
    // Implementation
    Ok(ToolOutput { /* ... */ })
}
```

**Parameters:**
- `name` — CLI name for the tool (lowercase, hyphens)
- `description` — Human-readable description (shown in help)

**Return type:**
- Must return `Result<Output, String>` where:
  - `Output` implements `Serialize`
  - Error messages are `String`

### Input/Output Types

Define input and output types using `serde`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolInput {
    pub param1: String,
    pub param2: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolOutput {
    pub result: String,
}
```

Types can contain:
- Scalar types: `String`, integers, floats, booleans
- Collections: `Vec<T>`, `HashMap<K, V>`
- Nested structs (if they implement `Serialize`/`Deserialize`)

### Available Capabilities

Use `corvid_plugin_sdk::context::Context` to access agent capabilities:

```rust
use corvid_plugin_sdk::context::Context;

#[corvid_tool(name = "send-message", description = "Send an encrypted message")]
fn send_message(ctx: Context, input: MessageInput) -> Result<MessageOutput, String> {
    ctx.send_message(&input.to_contact, &input.message)?;
    Ok(MessageOutput {
        sent: true,
    })
}
```

**Available methods:**

#### Messaging

```rust
// Send a message to a contact
ctx.send_message(contact_name: &str, message: &str) -> Result<(), String>

// Receive messages (blocks until timeout or message arrives)
ctx.receive_message(timeout_secs: u64) -> Result<Message, String>

// Inbox size
ctx.inbox_size() -> Result<usize, String>
```

#### Storage

```rust
// Store a key-value pair (plugin-isolated)
ctx.set(key: &str, value: &str) -> Result<(), String>

// Retrieve a value
ctx.get(key: &str) -> Result<Option<String>, String>

// Delete a key
ctx.delete(key: &str) -> Result<(), String>

// List all keys
ctx.keys() -> Result<Vec<String>, String>
```

#### Algorand Blockchain

```rust
// Get account info
ctx.account_info(address: &str) -> Result<AccountInfo, String>

// Get asset info
ctx.asset_info(asset_id: u64) -> Result<AssetInfo, String>

// Build and submit a transaction (advanced)
ctx.submit_transaction(tx: &Transaction) -> Result<String, String>
```

#### HTTP Requests

```rust
// Make an HTTP GET request
ctx.http_get(url: &str) -> Result<String, String>

// Make an HTTP POST request
ctx.http_post(url: &str, body: &str, headers: &[(&str, &str)]) -> Result<String, String>
```

**Note:** URLs must be on the agent's HTTP allowlist (configured at startup).

### Error Handling

Always return `Result<Output, String>`:

```rust
#[corvid_tool(name = "divide", description = "Divide two numbers")]
fn divide(input: DivideInput) -> Result<DivideOutput, String> {
    if input.divisor == 0 {
        return Err("Division by zero".to_string());
    }

    Ok(DivideOutput {
        result: input.dividend / input.divisor,
    })
}
```

Errors are:
- Serialized as JSON: `{"error": "error message"}`
- Logged by the agent
- Returned to the caller

## Example: Weather Plugin

Here's a complete example that fetches weather data:

```rust
use corvid_plugin_sdk::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct WeatherInput {
    pub city: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WeatherOutput {
    pub city: String,
    pub temperature: f64,
    pub description: String,
}

#[corvid_plugin]
struct WeatherPlugin;

#[corvid_tool(name = "weather", description = "Get weather for a city")]
fn weather(ctx: Context, input: WeatherInput) -> Result<WeatherOutput, String> {
    // Make an HTTP request to weather API
    let url = format!(
        "https://api.openweathermap.org/data/2.5/weather?q={}&units=metric&appid=YOUR_API_KEY",
        input.city
    );

    let response = ctx.http_get(&url)?;
    let data: serde_json::Value = serde_json::from_str(&response)
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    let temperature = data["main"]["temp"]
        .as_f64()
        .ok_or_else(|| "Missing temperature".to_string())?;

    let description = data["weather"][0]["description"]
        .as_str()
        .ok_or_else(|| "Missing description".to_string())?
        .to_string();

    Ok(WeatherOutput {
        city: input.city,
        temperature,
        description,
    })
}
```

Build and test:

```bash
cargo build --lib --target wasm32-wasip1 --release

# Copy to agent
cp target/wasm32-wasip1/release/weather_plugin.wasm ~/.corvid/plugins/

# Invoke
can plugin invoke weather-plugin weather '{"city": "San Francisco"}'
```

## Example: Stateful Plugin

Plugins can store persistent data:

```rust
use corvid_plugin_sdk::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct CounterInput {}

#[derive(Debug, Serialize, Deserialize)]
pub struct CounterOutput {
    pub count: u32,
}

#[corvid_plugin]
struct CounterPlugin;

#[corvid_tool(name = "increment", description = "Increment a counter")]
fn increment(ctx: Context, _input: CounterInput) -> Result<CounterOutput, String> {
    // Get current count
    let count_str = ctx.get("counter")?.unwrap_or_else(|| "0".to_string());
    let mut count: u32 = count_str.parse()
        .map_err(|e| format!("Invalid counter: {}", e))?;

    // Increment
    count += 1;

    // Store updated count
    ctx.set("counter", &count.to_string())?;

    Ok(CounterOutput { count })
}
```

Storage is **plugin-isolated** — each plugin has its own key-value store.

## Testing Your Plugin

Write tests in `src/lib.rs` or a separate `tests/` directory:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_greet() {
        let input = GreetInput {
            name: "Alice".to_string(),
        };

        let output = greet(input).expect("greet failed");
        assert!(output.message.contains("Alice"));
    }
}
```

Run tests with:

```bash
cargo test
```

**Note:** Tests run in the host (not WASM), so they can't test `Context` functionality. For that, test manually by loading the plugin.

## Trust Tiers

When loading your plugin, specify a trust tier:

```bash
can plugin load my-plugin.wasm --tier trusted
```

Tiers affect resource limits:

| Tier | Memory | Timeout | Use |
|------|--------|---------|-----|
| `trusted` | 512 MiB | 60s | First-party plugins you control |
| `verified` | 128 MiB | 30s | Code-reviewed third-party |
| `untrusted` | 32 MiB | 10s | Unknown/unreviewed (default) |

Choose the least-permissive tier appropriate for your plugin.

## Troubleshooting

### "Plugin binary is not a valid WebAssembly module"

Ensure:
- You compiled with `--target wasm32-wasip1`
- You used `crate-type = ["cdylib"]` in Cargo.toml
- No compile errors exist

### "Tool 'my-tool' not found"

Check:
- The tool name in `#[corvid_tool(name = "...")]` matches what you're invoking
- The plugin is loaded: `can plugin list`
- The WASM built successfully

### Plugin timeouts

If a tool times out:
- Reduce computation or network requests
- The timeout varies by trust tier (10s-60s)
- Consider breaking work into smaller tools

### "Memory limit exceeded"

- Reduce the amount of data your tool processes at once
- Use streaming/pagination for large datasets
- The limit varies by trust tier (32-512 MiB)

## Performance Tips

1. **Keep tools fast** — Prefer simple operations
2. **Cache results** — Use `ctx.set()` to avoid redundant computation
3. **Batch requests** — Make fewer HTTP calls with more data per call
4. **Minimize allocations** — Pre-allocate buffers when possible

## Publishing Your Plugin

1. **Create a repository** on GitHub
2. **Add a README** with usage instructions
3. **Include examples** and tests
4. **Document the API** for your tools
5. **Tag releases** matching plugin versions
6. **Add to the [plugin registry](https://github.com/CorvidLabs/corvid-agent-nano-plugins)** (coming soon)

## API Reference

For detailed API documentation, see the generated rustdoc:

```bash
cd crates/corvid-plugin-sdk
cargo doc --open
```

This shows:
- All available types
- Full method signatures
- Examples for each API
- Trait definitions

## Next Steps

- Read the [Plugins guide](./plugins.md) for running and managing plugins
- Check [CONTRIBUTING.md](../../CONTRIBUTING.md) for contribution guidelines
- Review the [hello-world example](https://github.com/CorvidLabs/corvid-agent-nano/tree/main/plugins/hello-world) for a minimal plugin
- Join the [discussions](https://github.com/CorvidLabs/corvid-agent-nano/discussions) for help

## Security Considerations

Your plugin runs in a sandbox, but keep these in mind:

1. **Don't trust plugin input** — Always validate parameters
2. **Handle errors gracefully** — Return meaningful error messages
3. **Limit external calls** — HTTP requests should have timeout and size limits
4. **Be careful with storage** — Don't store secrets unencrypted
5. **Test thoroughly** — Security bugs in plugins can affect the agent

For more on security, see [Security Model](../architecture/security.md).
