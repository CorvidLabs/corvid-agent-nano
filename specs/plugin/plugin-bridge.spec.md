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
| `refreshTools` | `()` | `Promise<void>` | Refresh tool registry from host (called automatically on connect) |

### PluginBridge Properties

| Property | Type | Description |
|----------|------|-------------|
| `connected` | `boolean` | Whether the bridge is currently connected to the plugin host |

### Auto-Registration

On connect, the bridge automatically calls `refreshTools()` to fetch all plugin manifests and tools, then registers them into the corvid-agent tool registry:

```typescript
for (const entry of tools) {
  const toolName = `plugin:${entry.plugin_id}:${entry.tool.name}`;
  toolRegistry.register({
    name: toolName,
    description: entry.tool.description,
    inputSchema: entry.tool.input_schema,  // JSON Schema v7 passthrough
    execute: (input) => bridge.invoke(entry.plugin_id, entry.tool.name, input),
  });
}
```

Tool names are namespaced as `plugin:<pluginId>:<toolName>` to avoid collisions with built-in tools. Manual calls to `refreshTools()` will unregister stale tools and re-register the current set.

### REST Endpoints

| Route | Method | Response | Description |
|-------|--------|----------|-------------|
| `/api/plugins` | GET | `{ plugins: PluginManifest[] }` with tools, or `{ error: string, plugins: [] }` (503/502) | List all plugins and their tools |
| `/api/plugins/:id/invoke/:tool` | POST | `{ result: string }` or `{ error: string, retryable?: boolean }` (400/503/500) | Invoke a specific plugin tool with JSON request body |

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

interface PluginEvent {
  type: string;
  pluginId?: string;
  payload: unknown;
}
```

## Invariants

1. The bridge is the **only** TS code that talks to the plugin host — no other server module opens the socket
2. Tool names are always namespaced: `plugin:<pluginId>:<toolName>`
3. Auto-registration via `refreshTools()` is called automatically on connect — no per-plugin TypeScript wiring required
4. `refreshTools()` unregisters stale tools before registering new ones (supports hot-reload)
5. The bridge reconnects automatically if the socket drops (with exponential backoff, max 30s)
6. MessagePack is used for tool invocation payloads (data plane); JSON-RPC for everything else (control plane)
7. The bridge does not validate plugin manifests — that's the host's responsibility
8. If the plugin host is not running, the bridge logs a warning and queues no requests — tools simply aren't registered
9. Tool invocation timeouts vary by trust tier: 30s (trusted), 5s (verified), 1s (untrusted)
10. Tool `input_schema` from plugins is passed through verbatim to the registry — no transformation

## Behavioral Examples

### Scenario: Plugin host starts with 3 plugins

- **Given** the plugin host starts and loads algo-oracle, code-snoop, memory-graph
- **When** the bridge connects and calls `refreshTools()`
- **Then** the bridge queries the host for manifests and tools, registering: `plugin:corvid-algo-oracle:set_threshold`, `plugin:corvid-algo-oracle:fetch_app_state`, `plugin:corvid-code-snoop:lint_diff`, `plugin:corvid-memory-graph:find_related_memories`

### Scenario: Agent invokes a plugin tool

- **Given** an agent calls tool `plugin:corvid-algo-oracle:set_threshold` with input `{ value: 0.5 }`
- **When** the bridge's `invoke()` method is called via tool registry
- **Then** it encodes input as MessagePack, sends `plugin.invoke` RPC to the host, applies timeout per trust tier, and returns the result string

### Scenario: Plugin hot-reloaded

- **Given** algo-oracle is reloaded with a new version that adds a new tool
- **When** `refreshTools()` is called (manually or on reconnect)
- **Then** the bridge unregisters all old tools for all plugins and registers the current set from the host

### Scenario: Plugin host not running

- **Given** the corvid-agent server starts but the plugin host sidecar is not running
- **When** the bridge tries to connect to the socket path
- **Then** logs warning, schedules exponential backoff retry, server operates normally without plugins; tools are registered once host is available

## Error Cases

| Condition | Behavior |
|-----------|----------|
| Socket connection refused | Warning logged: `[plugin-bridge] connection refused: {socketPath}`, retry with exponential backoff |
| Socket drops mid-operation | All pending invocations rejected with "socket closed", reconnect initiated, tools unregistered |
| Plugin tool invocation timeout | RPC timeout error after 30s (Trusted) / 5s (Verified) / 1s (Untrusted) |
| Plugin returns `unavailable: true` (draining) | Bridge throws error with `status: 503, retryable: true` to caller |
| Plugin returns error | Bridge throws error message from plugin |
| Invalid JSON-RPC response | Malformed lines silently ignored during parsing |
| Plugin host disconnected before response | Pending RPC rejected with "not connected to plugin host" |

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
