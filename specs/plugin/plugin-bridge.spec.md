---
module: plugin-bridge
version: 2
status: stable
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

### Exported Interfaces

| Interface | File | Description |
|-----------|------|-------------|
| `PluginManifest` | `rust-bridge.ts` | Plugin metadata: id, version, author, description, capabilities, trust tier, and available tools |
| `ToolInfo` | `rust-bridge.ts` | Tool definition: name, description, and JSON Schema v7 input schema |
| `HealthStatus` | `rust-bridge.ts` | Plugin host health status: connected flag, per-plugin state, and uptime |
| `PluginEvent` | `rust-bridge.ts` | Event dispatched to plugins: type, timestamp, and payload |

### Exported Classes

| Class | File | Description |
|-------|------|-------------|
| `PluginBridge` | `rust-bridge.ts` | Unix socket client — JSON-RPC dispatch, tool invocation, event forwarding |

### Exported Functions

| Function | File | Parameters | Returns | Description |
|----------|------|-----------|---------|-------------|
| `registerPluginRoutes` | `plugins.ts` | `(app: BunServe, bridge: PluginBridge)` | `void` | Register `/api/plugins` and tool invocation REST endpoints |

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
| `refreshTools` | `()` | `Promise<void>` | Refresh tool registry from host — fetches manifests and registers all tools |
| `connected` | (getter) | `boolean` | Whether the bridge is currently connected to the plugin host |

### Exported Functions

| Function | File | Parameters | Returns | Description |
|----------|------|-----------|---------|-------------|
| `registerPluginRoutes` | `plugins.ts` | `(router: Router, bridge: PluginBridge)` | `void` | Register `/api/plugins` REST endpoints with the server router |

### Auto-Registration

The bridge auto-registers plugin tools on connect by calling `refreshTools()`:

1. Fetches all plugin manifests from the host
2. Stores trust tier info for each plugin (used to determine invocation timeout)
3. Fetches all tools and registers them into the corvid-agent tool registry
4. Tool names are namespaced as `plugin:<pluginId>:<toolName>` to avoid collisions

```typescript
// After successful socket connection:
this.refreshTools().catch((err) =>
  console.warn("[plugin-bridge] tool refresh failed:", err.message),
);

// Tool registration for each plugin tool:
toolRegistry.register({
  name: `plugin:${pluginId}:${toolName}`,
  description: tool.description,
  inputSchema: tool.input_schema,  // JSON Schema v7 passthrough
  execute: (input) => bridge.invoke(pluginId, toolName, input),
});
```

### REST Endpoints

| Route | Method | Parameters | Response | Status | Description |
|-------|--------|-----------|----------|--------|-------------|
| `/api/plugins` | GET | — | `{ plugins: PluginListItem[] }` | 200, 503, 502 | List all plugins with their tools |
| `/api/plugins/:id/invoke/:tool` | POST | `id` (plugin id), `tool` (tool name), body (tool input) | `{ result: string }` | 200, 400, 503, 500 | Invoke a specific plugin tool |

**Response status codes:**
- `200` — Success
- `400` — Missing plugin id or tool name
- `500` — Tool invocation failed (with optional `{ error, retryable }` fields)
- `503` — Plugin host not connected or plugin draining (retryable)
- `502` — Plugin host error (bad gateway)

### TypeScript Types

```typescript
export interface PluginManifest {
  id: string;
  version: string;
  author: string;
  description: string;
  capabilities: string[];
  trust_tier: "trusted" | "verified" | "untrusted";
  tools: ToolInfo[];
}

export interface ToolInfo {
  name: string;
  description: string;
  input_schema: Record<string, unknown>;  // JSON Schema v7
}

export interface HealthStatus {
  connected: boolean;
  plugins: Record<string, "active" | "draining" | "unloaded">;
  uptimeMs: number;
}

export interface PluginEvent {
  type: string;
  pluginId?: string;
  payload: unknown;
}
```

## Invariants

1. The bridge is the **only** TS code that talks to the plugin host — no other server module opens the socket
2. Tool names are always namespaced: `plugin:<pluginId>:<toolName>`
3. Auto-registration is triggered by calling `refreshTools()` on socket connect — zero per-plugin TypeScript wiring required
4. The bridge reconnects automatically if the socket drops (with exponential backoff, max 30s configured via constructor option)
5. MessagePack is used for tool invocation payloads (data plane); newline-delimited JSON for JSON-RPC (control plane)
6. The bridge does not validate plugin manifests — that's the host's responsibility
7. If the plugin host is not running, the bridge logs a warning and queues no requests — tools simply aren't registered, endpoints return 503
8. Tool `input_schema` from plugins is passed through verbatim to the registry — no transformation
9. Invocation timeout depends on plugin trust tier: trusted=30s, verified=5s, untrusted=1s
10. All pending RPC requests are rejected if socket closes or disconnect is called

## Behavioral Examples

### Scenario: Bridge connects to plugin host with 3 plugins loaded

- **Given** the plugin host is running with algo-oracle, code-snoop, memory-graph already loaded
- **When** the bridge successfully connects to the Unix socket
- **Then** `refreshTools()` fetches all manifests, stores trust tiers, and registers all tools: `plugin:corvid-algo-oracle:set_threshold`, `plugin:corvid-algo-oracle:fetch_app_state`, `plugin:corvid-code-snoop:lint_diff`, `plugin:corvid-memory-graph:find_related_memories`

### Scenario: Agent invokes a plugin tool

- **Given** a tool `plugin:corvid-algo-oracle:set_threshold` with trust_tier="verified" is registered
- **When** `invoke("corvid-algo-oracle", "set_threshold", input)` is called
- **Then** the bridge: (1) looks up timeout for "verified" tier (5s), (2) encodes input as MessagePack, (3) sends `plugin.invoke` RPC with base64-encoded payload, (4) awaits result with 5s timeout, (5) returns result as string or throws error

### Scenario: Tool refresh is called manually

- **Given** a user calls `refreshTools()` on the bridge
- **When** the method completes
- **Then** all previously registered tools are unregistered, fresh manifests fetched, and new tool set registered

### Scenario: Plugin host not running

- **Given** the corvid-agent server starts but the plugin host sidecar is not running
- **When** the bridge tries to connect with `connect(socketPath)`
- **Then** logs a warning, connection rejected, and `scheduleReconnect()` initiates exponential backoff; server operates normally without plugins, endpoints return 503

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
| `node:net` | `connect()` for Unix domain socket connection |
| `@msgpack/msgpack` | `encode()` and `decode()` for data plane MessagePack serialization |
| `corvid-plugin-host` | Unix socket server providing JSON-RPC interface (Rust sidecar) |
| `ToolRegistry` (injected) | `register()` and `unregister()` for tool lifecycle management |

### Consumed By

| Module | What is used |
|--------|-------------|
| `server/routes/plugins.ts` | `PluginBridge` class and all exported interfaces (`PluginManifest`, `ToolInfo`, `HealthStatus`, `PluginEvent`); `registerPluginRoutes()` function |
| Server initialization | Bridge instance passed to route registration and server lifecycle handlers |
| Agent tool invocation pipeline | Auto-registered plugin tools callable like any built-in tool |
| REST API clients (dashboard, CLI) | `/api/plugins` endpoints for plugin discovery and tool invocation |

## Configuration

### Constructor Options

```typescript
interface PluginBridgeOptions {
  reconnectMax?: number;    // Max reconnect backoff in milliseconds (default: 30000)
  toolRegistry?: ToolRegistry;  // Tool registry for auto-registration (default: null/no auto-register)
}
```

| Option | Default | Description |
|--------|---------|-------------|
| `reconnectMax` | `30000` | Max exponential backoff for reconnection attempts (milliseconds) |
| `toolRegistry` | `null` | Optional tool registry instance; if provided, tools are auto-registered on connect |

### Runtime Settings

- **Socket path** — Passed as parameter to `connect(socketPath: string)`
- **General RPC timeout** — 10s (hardcoded for control plane RPCs)
- **Invocation timeouts** — Determined by plugin trust tier: trusted=30s, verified=5s, untrusted=1s

## Change Log

| Date | Author | Change |
|------|--------|--------|
| 2026-04-06 | CorvidAgent | Updated to spec-sync v3.3.0 format — status: active → stable |
| 2026-03-31 | Magpie | Updated spec to match actual implementation: added all exported interfaces/functions, clarified `refreshTools()` auto-registration, fixed Configuration to use constructor options, updated field names to snake_case, expanded REST endpoint documentation |
| 2026-03-28 | CorvidAgent | Initial spec from council synthesis (Issue #15) |
