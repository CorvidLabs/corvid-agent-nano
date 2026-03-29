# plugin

Manage WASM plugins.

```bash
can plugin <SUBCOMMAND>
```

Requires the agent to be running (`can run`) -- plugins are hosted by the sidecar process.

## Subcommands

### list

List loaded plugins.

```bash
can plugin list
```

### invoke

Invoke a plugin tool.

```bash
can plugin invoke <PLUGIN_ID> <TOOL> [INPUT_JSON]
```

**Example:**
```bash
can plugin invoke hello-world hello '{"name": "Leif"}'
```

### load

Load a plugin from a WASM file.

```bash
can plugin load <PATH> [--tier <TIER>]
```

Trust tiers: `trusted`, `verified`, `untrusted` (default).

### unload

Unload a plugin by ID.

```bash
can plugin unload <PLUGIN_ID>
```

### health

Check plugin host health.

```bash
can plugin health
```

## Plugin system

Plugins are WebAssembly modules that extend agent capabilities. They run in a sandboxed environment with resource limits based on their trust tier:

| Tier | Memory | Execution time |
|------|--------|---------------|
| Trusted | 512 MiB | 60s |
| Verified | 128 MiB | 30s |
| Untrusted | 32 MiB | 10s |

Place `.wasm` files in `<data-dir>/plugins/` and they will be loaded automatically when the agent starts.

See [Plugins guide](../guides/plugins.md) for writing custom plugins.
