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

### Auto-Registration

When the bridge connects, it automatically fetches all plugin manifests and tools, then registers each into the corvid-agent tool registry via the `refreshTools()` method:

```typescript
// Called automatically on connect
async refreshTools(): Promise<void> {
  const manifests = await this.listManifests();
  const resp = await this.rpc('plugin.tools', {});

  for (const entry of resp.tools) {
    toolRegistry.register({
      name: `plugin:${entry.plugin_id}:${entry.tool.name}`,
      description: entry.tool.description,
      inputSchema: entry.tool.input_schema,  // JSON Schema v7 passthrough
      execute: (input) => bridge.invoke(entry.plugin_id, entry.tool.name, input),
    });
  }
}
```

Tool names are namespaced as `plugin:<pluginId>:<toolName>` to avoid collisions with built-in tools.

### REST Endpoints

| Route | Method | Response | Status | Description |
|-------|--------|----------|--------|-------------|
| `/api/plugins` | GET | `{ plugins: PluginManifest[] }` | 200 | List all plugins with tools |
| `/api/plugins` | GET | `{ error: string, plugins: [] }` | 503 | Plugin host not connected |
| `/api/plugins` | GET | `{ error: string, plugins: [] }` | 502 | Plugin host communication error |
| `/api/plugins/:id/invoke/:tool` | POST | `{ result: string }` | 200 | Tool invocation succeeded |
| `/api/plugins/:id/invoke/:tool` | POST | `{ error: string }` | 503 | Plugin draining (retryable) |
| `/api/plugins/:id/invoke/:tool` | POST | `{ error: string }` | 500 | Tool invocation error |
| `/api/plugins/:id/invoke/:tool` | POST | `{ error: string }` | 503 | Plugin host not connected |
| `/api/plugins/:id/invoke/:tool` | POST | `{ error: string }` | 400 | Missing plugin id or tool name |

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
3. Auto-registration happens on connect via `refreshTools()` — zero per-plugin TypeScript wiring required
4. The bridge reconnects automatically if the socket drops (with exponential backoff, max 30s)
5. MessagePack is used for tool invocation payloads (data plane); JSON-RPC for everything else (control plane)
6. The bridge does not validate plugin manifests — that's the host's responsibility
7. If the plugin host is not running, the bridge logs a warning and queues no requests — tools simply aren't registered
8. Tool `input_schema` from plugins is passed through verbatim to the registry — no transformation

## Behavioral Examples

### Scenario: Bridge connects and auto-registers tools

- **Given** the bridge calls `connect(socketPath)`
- **When** the connection succeeds
- **Then** `refreshTools()` fetches all manifests and tool lists via RPC, then registers each tool with the registry as `plugin:<pluginId>:<toolName>`

### Scenario: Agent invokes a plugin tool

- **Given** an agent invokes tool `plugin:corvid-algo-oracle:set_threshold` with input `{ value: 100 }`
- **When** the bridge receives the invocation with timeout based on trust tier
- **Then** it sends a MessagePack-encoded `plugin.invoke` RPC to the host, awaits the result, returns it to the agent

### Scenario: Manual tool refresh

- **Given** a manual call to `bridge.refreshTools()` (e.g., after a plugin is manually reloaded on the host)
- **When** the refresh completes
- **Then** old tools are unregistered and the latest tool set from the host is re-registered

### Scenario: Plugin host not running

- **Given** the corvid-agent server starts but the plugin host sidecar is not running
- **When** the bridge tries to connect
- **Then** logs a warning "connection refused", schedules reconnect with exponential backoff, server operates normally without plugin tools

## Error Cases

| Condition | Behavior |
|-----------|----------|
| Socket connection refused | Warning logged, retry with exponential backoff |
| Socket drops mid-operation | Pending invocations rejected, reconnect initiated |
| Plugin tool invocation timeout | Returns error to caller after 30s (Trusted) / 5s (Verified) / 1s (Untrusted) |
| Plugin returns `Unavailable` (draining) | Bridge returns 503 to caller, retryable |
| Invalid MessagePack response | Error logged, invocation fails |

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
