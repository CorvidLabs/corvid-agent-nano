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

### Exported Types (rust-bridge.ts)

| Type | Description |
|------|-------------|
| `PluginManifest` | Metadata for a loaded plugin: id, version, author, description, capabilities, trust_tier, and tools |
| `ToolInfo` | Tool description: name, description, and JSON Schema v7 input_schema |
| `HealthStatus` | Plugin host health: connected status, per-plugin state, and uptime in milliseconds |
| `PluginEvent` | Event to dispatch to plugins: type, optional pluginId, and payload (generic) |

### Exported Classes (rust-bridge.ts)

| Class | Description |
|-------|-------------|
| `PluginBridge` | Unix socket client for the Rust plugin host sidecar — JSON-RPC dispatch, tool invocation, event forwarding, health checks, and auto-registration |

### PluginBridge Public Methods

| Method | Parameters | Returns | Description |
|--------|-----------|---------|-------------|
| `constructor` | `(opts?: { reconnectMax?: number; toolRegistry?: ToolRegistry })` | — | Initialize bridge with optional reconnect max (ms) and tool registry for auto-registration |
| `connect` | `(socketPath: string)` | `Promise<void>` | Connect to the plugin host Unix socket; rejects if connection fails, retries with exponential backoff |
| `disconnect` | `()` | `Promise<void>` | Gracefully close the socket and clean up all resources (timers, tools, pending requests) |
| `listManifests` | `()` | `Promise<PluginManifest[]>` | RPC `plugin.list` — list all loaded plugin manifests |
| `listTools` | `(pluginId?: string)` | `Promise<ToolInfo[]>` | RPC `plugin.tools` — list all tools, optionally filtered by plugin ID |
| `invoke` | `(pluginId: string, tool: string, input: unknown)` | `Promise<string>` | RPC `plugin.invoke` — invoke a tool with MessagePack-encoded input; respects trust tier timeouts; throws if plugin is draining (retryable) |
| `dispatchEvent` | `(event: PluginEvent)` | `Promise<void>` | RPC `plugin.dispatch` — forward an event to subscribing plugins |
| `healthCheck` | `()` | `Promise<HealthStatus>` | RPC `health.check` — check plugin host connectivity and per-plugin status; never throws |
| `connected` | — | `boolean` (getter) | Whether the bridge is currently connected to the plugin host socket |
| `refreshTools` | `()` | `Promise<void>` | Fetch manifests and all tools, then auto-register them into the tool registry (if configured); unregisters stale tools first |

### Auto-Registration

When a `PluginBridge` is constructed with a `toolRegistry` option, it automatically registers plugin tools on `connect()` by calling `refreshTools()`:

1. Fetches all plugin manifests to get trust tier information
2. Fetches all tools from all plugins
3. For each tool, registers into the registry with name `plugin:<pluginId>:<toolName>`
4. Tool execution delegates to `bridge.invoke(pluginId, tool.name, input)`
5. On reconnect or manual `refreshTools()` call, old tools are unregistered and new ones registered

No external event subscription is needed — the bridge handles discovery on connect and periodically via `refreshTools()`.

### REST Endpoints (server/routes/plugins.ts)

| Route | Method | Request | Response | Status | Description |
|-------|--------|---------|----------|--------|-------------|
| `/api/plugins` | GET | — | `{ plugins: PluginListItem[] }` | 200 or 503 | List all plugins with their tools; returns empty array if host not connected |
| `/api/plugins/:id/invoke/:tool` | POST | JSON body (passed to tool) | `{ result: string }` or `{ error: string, retryable: boolean }` | 200, 400, 500, or 503 | Invoke a specific tool; `:id` is pluginId, `:tool` is toolName |

Note: `PluginListItem` extends `PluginManifest` and includes the `tools` array.

### TypeScript Type Definitions

```typescript
interface PluginManifest {
  id: string;
  version: string;
  author: string;
  description: string;
  capabilities: string[];  // e.g., ['storage', 'http', 'algo']
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
  plugins: Record<string, 'active' | 'draining' | 'unloaded'>;  // plugin ID -> status
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
2. Tool names are always namespaced: `plugin:<pluginId>:<toolName>` to avoid collisions
3. Auto-registration is driven by `refreshTools()` (called on connect and manually) — if a tool registry is provided, tools are automatically registered when manifests/tools are fetched
4. The bridge reconnects automatically if the socket drops, with exponential backoff starting at 500ms and capping at `reconnectMax` (default 30s)
5. MessagePack is used for tool invocation payloads (binary serialization in base64 JSON-RPC params); JSON for all RPC control messages
6. RPC request IDs are incremented sequentially starting from 1; pending requests are tracked by ID and timeout individually
7. The bridge logs warnings to stderr but never throws on socket/connection errors — callers must check `.connected` or handle RPC errors
8. Tool `input_schema` from plugins is passed through verbatim to the registry — no validation or transformation
9. When the socket closes unexpectedly, all pending RPC requests are rejected with "socket closed" error, and reconnection is scheduled if not explicitly closed
10. Trust tier timeout enforcement: `trusted` (30s), `verified` (5s), `untrusted` (1s) — invoking a higher-tier plugin than available uses the untrusted timeout

## Behavioral Examples

### Scenario: Bridge connects to running plugin host

- **Given** a `PluginBridge` is constructed with a tool registry and `connect()` is called
- **When** the socket connects successfully
- **Then** `refreshTools()` is automatically triggered, which:
  1. Calls `plugin.list` RPC to fetch all plugin manifests and store trust tiers
  2. Calls `plugin.tools` RPC to fetch all tools
  3. For each tool, registers `plugin:<pluginId>:<toolName>` into the registry
  4. Resolve connect promise
- **And** tools are now callable through the registry

### Scenario: Agent invokes a plugin tool

- **Given** a tool `plugin:corvid-algo-oracle:set_threshold` is registered in the tool registry
- **When** the agent invokes it with input `{ value: 100 }`
- **Then** the bridge:
  1. Looks up the plugin's trust tier (e.g., 'verified')
  2. Selects the appropriate timeout (5 seconds for 'verified')
  3. Sends `plugin.invoke` RPC with MessagePack-encoded input (base64-serialized)
  4. Waits up to 5 seconds for response
  5. Returns the result string to the caller
- **If** response is `{ unavailable: true }`, throws with `status: 503` and `retryable: true`

### Scenario: Plugin host drops unexpectedly

- **Given** the bridge is connected and actively using tools
- **When** the socket closes without explicit `disconnect()` call
- **Then**:
  1. All pending RPC requests are rejected with "socket closed"
  2. All registered tools are unregistered from the registry
  3. Reconnection is automatically scheduled with exponential backoff (500ms initial)
  4. On reconnect, `refreshTools()` is called again to re-register tools

### Scenario: Manual refresh after plugin updates

- **Given** the bridge is connected and `refreshTools()` is called manually
- **When** the method completes
- **Then**:
  1. Stale tools from the previous registration are unregistered
  2. Fresh manifests and tools are fetched
  3. New tools are registered with updated schemas and handlers
- **Used for** hot-reload scenarios without disconnecting the bridge

## Error Cases

| Condition | Bridge Behavior | HTTP Response (if via REST) |
|-----------|-----------------|---------------------------|
| Socket connection refused on `connect()` | Logs warning, rejects promise, schedules exponential-backoff reconnect | N/A |
| Socket drops mid-RPC | Pending request rejected with "socket closed", socket state nulled, reconnect scheduled | N/A (depends on whether request already sent) |
| RPC timeout (method timeout expires) | Request removed from pending map, rejected with "RPC timeout: <method> (<ms>)ms" | 500 or 502 (method-dependent) |
| Plugin tool invocation timeout (trust tier) | Request rejected after trust-tier timeout (30s/5s/1s); tool gets error | 500 with timeout error message |
| Plugin returns `{ unavailable: true }` | Throws error with `status: 503` and `retryable: true` | 503 with retryable flag |
| Plugin returns error string | Throws Error with plugin's error message | 500 with error message |
| RPC parsing error (malformed JSON) | Error caught and ignored in `handleData()` — response dropped silently | N/A |
| Invoke before `connect()` | `invoke()` rejects with "not connected to plugin host" | 503 if via REST |
| Bridge not connected on GET `/api/plugins` | Returns 503 with "plugin host not connected" | 503 |

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
| 2026-04-03 | Magpie | Documentation sync: clarified Public API (all exported types and methods), corrected auto-registration mechanism (on-connect `refreshTools()` not event-driven), fixed TypeScript type names (snake_case in actual code), added error cases table with HTTP status codes, updated invariants with timeout details |
| 2026-03-28 | CorvidAgent | Initial spec from council synthesis (Issue #15) |
