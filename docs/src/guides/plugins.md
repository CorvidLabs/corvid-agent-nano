# Plugins (WASM)

Extend agent capabilities with WebAssembly plugins.

## Overview

The plugin system uses a sidecar architecture:
1. `can run` spawns the `corvid-plugin-host` binary as a child process
2. The plugin host loads `.wasm` files from the plugins directory
3. `can` communicates with the plugin host via JSON-RPC over a Unix socket

## Installing plugins

Place `.wasm` files in `<data-dir>/plugins/`:

```bash
cp my-plugin.wasm ./data/plugins/
can run  # plugins are loaded on startup
```

## Using plugins

```bash
# List loaded plugins
can plugin list

# Invoke a tool
can plugin invoke <plugin-id> <tool-name> '{"key": "value"}'

# Check health
can plugin health

# Load at runtime
can plugin load ./path/to/plugin.wasm --tier untrusted

# Unload
can plugin unload <plugin-id>
```

## Trust tiers

Plugins run in a sandboxed WebAssembly environment with resource limits:

| Tier | Memory | Timeout | Use case |
|------|--------|---------|----------|
| `trusted` | 512 MiB | 60s | First-party, fully audited plugins |
| `verified` | 128 MiB | 30s | Third-party, code-reviewed plugins |
| `untrusted` | 32 MiB | 10s | Unknown/unreviewed plugins (default) |

## Writing plugins

Plugins are built with the `corvid-plugin-sdk` crate:

```rust
use corvid_plugin_sdk::prelude::*;

#[corvid_plugin]
struct HelloPlugin;

#[corvid_tool(name = "hello", description = "Say hello")]
fn hello(input: HelloInput) -> HelloOutput {
    HelloOutput {
        message: format!("Hello, {}!", input.name),
    }
}
```

Build with:
```bash
cargo build --target wasm32-wasip1 --release
```

See the `plugins/hello-world/` example in the repository.

## Disabling plugins

```bash
can run --no-plugins
```
