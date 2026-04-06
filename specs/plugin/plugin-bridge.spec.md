---
module: plugin-bridge
version: 1
status: active
files:
  - server/plugins/rust-bridge.ts
  - server/routes/plugins.ts
depends_on:
  - specs/plugin/plugin-host.spec.md
---

# Plugin Bridge (TypeScript)

## Purpose

The TypeScript integration layer (~100 lines) that connects the corvid-agent Bun server to the Rust plugin host sidecar. Communicates over Unix domain socket using JSON-RPC (control) and MessagePack (data). Auto-registers plugin tools into the existing MCP/skill registry when plugins load. Provides the `/api/plugins` REST endpoint for the Angular dashboard.

This is the **only TypeScript code** needed to integrate the entire Rust plugin system.

## Public API

### Exported Classes

| Class | File | Description |
|-------|------|-------------|
| `PluginBridge` | `rust-bridge.ts` | Unix socket client — JSON-RPC dispatch, tool invocation, event forwarding |

### PluginBridge Methods

| Method | Parameters | Returns | Description |
|--------|-----------|---------|-------------|
| `connect` | `(socketPath: string)` | `Promise<void>` | Connect to the plugin host Unix socket |
| `disconnect` | `()` | `Promise<void>` | Gracefully close the socket connection |
| `listManifests` | `()` | `Promise<PluginManifest[]>` | List all loaded plugin manifests |
| `listTools` | `(pluginId?: string)` | `Promise<ToolInfo[]>` | List tools (all or filtered by plugin) |
| `invoke` | `(pluginId: string, tool: string, input: unknown)` | `Promise<string>` | Invoke a plugin tool via MessagePack data plane |
| `dispatchEvent` | `(event: PluginEvent)` | `Promise<void>` | Forward an event to subscribing plugins |
| `healthCheck` | `()` | `Promise<HealthStatus>` | Check plugin host health |
| `refreshTools` | `()` | `Promise<void>` | Refresh tool registry from host (called on connect and manually) |
| `connected` | (getter) | `boolean` | Whether the bridge is currently connected |

### Auto-Registration

When the plugin host emits tool updates, the bridge auto-registers each tool into the corvid-agent tool registry via `refreshTools()`:

```typescript
async refreshTools(): Promise<void> {
  // Unregister stale tools
  this.unregisterAllTools();

  // Fetch manifests to get tier info
  const manifests = await this.listManifests();
  for (const m of manifests) {
    this.pluginTiers.set(m.id, m.trust_tier);
  }

  // Fetch all tools and register them
  const resp = await this.rpc('plugin.tools', {});
  const data = resp as { tools?: Array<{ plugin_id: string; tool: ToolInfo }> };

  for (const entry of data.tools ?? []) {
    const toolName = `plugin:${entry.plugin_id}:${entry.tool.name}`;
    this.toolRegistry.register({
      name: toolName,
      description: entry.tool.description,
      inputSchema: entry.tool.input_schema,  // JSON Schema v7 passthrough
      execute: (input) => this.invoke(entry.plugin_id, entry.tool.name, input),
    });
  }
}
```

Tool names are namespaced as `plugin:<pluginId>:<toolName>` to avoid collisions with built-in tools.

### REST Endpoints

| Route | Method | Request | Response | Description |
|-------|--------|---------|----------|-------------|
| `/api/plugins` | GET | - | `{ plugins: (PluginManifest & { tools: ToolInfo[] })[] }` | List all plugins and their tools |
| `/api/plugins/:id/invoke/:tool` | POST | `unknown` (any JSON) | `{ result: string }` or `{ error: string, retryable: boolean }` | Invoke a specific plugin tool |

On connection failure (plugin host not running):
- GET `/api/plugins` returns 503 with `{ error: "plugin host not connected", plugins: [] }`
- POST `/api/plugins/:id/invoke/:tool` returns 503 with `{ error: "plugin host not connected" }`

### TypeScript Types

```typescript
interface PluginManifest {
  id: string;
  version: string;
  author: string;
  description: string;
  capabilities: string[];
  trust_tier: 'trusted' | 'verified' | 'untrusted';
  tools: ToolInfo[];
}

interface ToolInfo {
  name: string;
  description: string;
  input_schema: Record<string, unknown>;  // JSON Schema v7
}

interface HealthStatus {
  connected: boolean;
  plugins: Record<string, 'active' | 'draining' | 'unloaded'>;
  uptimeMs: number;
}
```

## Invariants

1. The bridge is the **only** TS code that talks to the plugin host — no other server module opens the socket
2. Tool names are always namespaced: `plugin:<pluginId>:<toolName>`
3. Auto-registration happens via `refreshTools()` — called on `connect()` and can be called manually
4. The bridge reconnects automatically if the socket drops (with exponential backoff, max 30s configurable)
5. MessagePack is used for tool invocation payloads (data plane); JSON-RPC for everything else (control plane)
6. The bridge does not validate plugin manifests — that's the host's responsibility
7. If the plugin host is not running, the bridge logs a warning and queues no requests — tools simply aren't registered
8. Tool `input_schema` from plugins is passed through verbatim to the registry — no transformation
9. Plugin trust tiers (`trust_tier`) determine invocation timeouts: `trusted` (30s), `verified` (5s), `untrusted` (1s)
10. Tool registry passed via constructor is optional — bridge operates without auto-registration if none provided

## Behavioral Examples

### Scenario: Plugin host starts with 3 plugins

- **Given** the plugin host starts and loads algo-oracle, code-snoop, memory-graph
- **When** the bridge calls `refreshTools()` (on connect and can be called manually)
- **Then** the bridge registers all tools: `plugin:corvid-algo-oracle:set_threshold`, `plugin:corvid-algo-oracle:fetch_app_state`, `plugin:corvid-code-snoop:lint_diff`, `plugin:corvid-memory-graph:find_related_memories`

### Scenario: Agent invokes a plugin tool

- **Given** an agent calls tool `plugin:corvid-algo-oracle:set_threshold`
- **When** the bridge receives the invocation
- **Then** it sends a MessagePack-encoded `plugin.invoke` to the host, awaits the result, returns it to the agent

### Scenario: Plugin hot-reloaded

- **Given** algo-oracle is reloaded with a new version that adds a new tool
- **When** the host emits `plugin.tools_registered` with the updated tool list
- **Then** the bridge unregisters old tools for that plugin and registers the new set

### Scenario: Plugin host not running

- **Given** the corvid-agent server starts but the plugin host sidecar is not running
- **When** the bridge tries to connect
- **Then** logs a warning, no plugin tools are registered, server operates normally without plugins

## Error Cases

| Condition | Behavior |
|-----------|----------|
| Socket connection refused | Warning logged, retry with exponential backoff (default max 30s) |
| Socket drops mid-operation | Pending RPC calls rejected with "bridge disconnected", reconnect initiated automatically |
| Plugin tool invocation timeout | Returns error to caller after timeout per trust tier: 30s (trusted) / 5s (verified) / 1s (untrusted) |
| Plugin returns `unavailable: true` (draining) | Bridge returns 503 to caller with `{ error, retryable: true }` |
| Plugin returns error | Bridge returns error message to caller with appropriate HTTP status |
| Not connected to plugin host | Bridge rejects all RPC calls with "not connected to plugin host" |
| Invalid JSON-RPC response | Error logged, invocation fails with appropriate error message |

## Dependencies

### Consumes

| Module | What is used |
|--------|-------------|
| `@msgpack/msgpack` | MessagePack encode/decode for data plane |
| `corvid-plugin-host` | Unix socket server (Rust side) |
| `server/tools/registry.ts` | Tool registration for auto-discovery |

### Consumed By

| Module | What is used |
|--------|-------------|
| `server/routes/plugins.ts` | `PluginBridge` for REST endpoints |
| `server/tools/registry.ts` | Auto-registered plugin tools |
| Agent tool invocation pipeline | Plugin tools callable like any built-in tool |

## Configuration

| Env Var / Flag | Default | Description |
|----------------|---------|-------------|
| `PLUGIN_SOCKET_PATH` | `~/.corvid/plugins.sock` | Path to plugin host Unix socket |
| `PLUGIN_RECONNECT_MAX` | `30000` | Max reconnect backoff in milliseconds |

## Change Log

| Date | Author | Change |
|------|--------|--------|
| 2026-03-28 | CorvidAgent | Initial spec from council synthesis (Issue #15) |
