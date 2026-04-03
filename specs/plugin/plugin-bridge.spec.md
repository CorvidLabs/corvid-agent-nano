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

### Exported Types

| Type | File | Description |
|------|------|-------------|
| `PluginManifest` | `rust-bridge.ts` | Plugin metadata: ID, version, author, capabilities, trust tier, and tools |
| `ToolInfo` | `rust-bridge.ts` | Tool definition: name, description, and JSON Schema input |
| `HealthStatus` | `rust-bridge.ts` | Plugin host health: connection state, per-plugin status, uptime |
| `PluginEvent` | `rust-bridge.ts` | Event for plugin dispatch: type, optional plugin ID, and payload |

### Exported Functions

| Function | File | Description |
|----------|------|-------------|
| `registerPluginRoutes` | `plugins.ts` | Registers `/api/plugins` REST endpoints with the server router |

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

On successful connection, the bridge automatically calls `refreshTools()` to query all available plugin tools and registers them into the corvid-agent tool registry:

```typescript
async refreshTools(): Promise<void> {
  // Fetch manifests to get trust tier info
  const manifests = await this.listManifests();
  for (const m of manifests) {
    this.pluginTiers.set(m.id, m.trust_tier);
  }

  // Fetch all tools and register them
  const resp = await this.rpc("plugin.tools", {});
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

Tool names are namespaced as `plugin:<pluginId>:<toolName>` to avoid collisions with built-in tools. This ensures tool availability is synchronized with plugin host state on every connect.

### REST Endpoint

| Route | Method | Response | Description |
|-------|--------|----------|-------------|
| `/api/plugins` | GET | `PluginManifest[]` with tools | List all plugins and their tools |
| `/api/plugins/:id/invoke/:tool` | POST | `{ result: string }` | Invoke a specific plugin tool |

### JSON-RPC Request/Response Formats

#### `plugin.invoke` Request (Control Plane)

```typescript
{
  method: "plugin.invoke",
  params: {
    pluginId: string;
    tool: string;
    input: string;  // Base64-encoded MessagePack of {pluginId, tool, input}
  },
  id: number;
}
```

The `input` parameter contains the base64-encoded MessagePack serialization of the original input structure. The host decodes and processes it.

#### `plugin.tools` Response (Control Plane)

```typescript
{
  result: {
    tools: Array<{
      plugin_id: string;  // Plugin identifier
      tool: {
        name: string;
        description: string;
        input_schema: Record<string, unknown>;  // JSON Schema v7
      };
    }>;
  }
}
```

#### `plugin.invoke` Response (Control Plane)

```typescript
{
  result: {
    result?: string;     // Output from plugin
    error?: string;      // Error message if invocation failed
    unavailable?: boolean; // True if plugin is draining (503)
  }
}
```

#### `health.check` Response

```typescript
{
  result: {
    plugins: Record<string, 'active' | 'draining' | 'unloaded'>;
    uptime_ms: number;
  }
}
```

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
3. Auto-registration happens on successful connection via `refreshTools()` — zero per-plugin TypeScript wiring required
4. The bridge reconnects automatically if the socket drops (with exponential backoff, max 30s)
5. Tool invocation uses MessagePack for the payload structure, then base64-encodes it before sending as a JSON-RPC parameter (data plane); JSON for all control plane requests
6. The bridge does not validate plugin manifests — that's the host's responsibility
7. If the plugin host is not running, the bridge logs a warning and queues no requests — tools simply aren't registered
8. Tool `input_schema` from plugins is passed through verbatim to the registry — no transformation

## Behavioral Examples

### Scenario: Bridge connects to running plugin host

- **Given** the plugin host is running with algo-oracle, code-snoop, memory-graph loaded
- **When** the bridge successfully connects to the Unix socket
- **Then** it calls `refreshTools()`, fetches all tools via `plugin.tools` RPC, and registers: `plugin:corvid-algo-oracle:set_threshold`, `plugin:corvid-algo-oracle:fetch_app_state`, `plugin:corvid-code-snoop:lint_diff`, `plugin:corvid-memory-graph:find_related_memories`

### Scenario: Agent invokes a plugin tool

- **Given** an agent calls tool `plugin:corvid-algo-oracle:set_threshold` with input `{value: 50}`
- **When** the bridge receives the invocation
- **Then** it:
  1. Encodes `{pluginId, tool, input}` as MessagePack
  2. Base64-encodes the MessagePack bytes
  3. Sends JSON-RPC `plugin.invoke` with base64 string as parameter
  4. Awaits result from host and returns it to agent

### Scenario: Socket drops and reconnects

- **Given** the plugin host crashes and restarts
- **When** the socket connection drops
- **Then** the bridge:
  1. Unregisters all tools
  2. Schedules reconnect with exponential backoff (500ms → 30s max)
  3. On reconnect, calls `refreshTools()` to re-register tools

### Scenario: Plugin host not running

- **Given** the corvid-agent server starts but the plugin host sidecar is not running
- **When** the bridge tries to connect
- **Then** logs a warning, no plugin tools are registered, server operates normally without plugins

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
