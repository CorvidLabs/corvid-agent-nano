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

### Exported Structs

| Struct | File | Description |
|--------|------|-------------|
| `PluginBridge` | `rust-bridge.ts` | Unix socket client — JSON-RPC dispatch, tool invocation, event forwarding |
| `PluginManifest` | `plugins.ts` | Plugin manifest interface: id, version, author, capabilities, tools |
| `ToolInfo` | `plugins.ts` | Tool descriptor: name, description, input JSON Schema |
| `HealthStatus` | `rust-bridge.ts` | Plugin host health status: connected, plugin states, uptime |
| `PluginEvent` | `rust-bridge.ts` | Event forwarded to subscribing plugins |

### Exported Functions

| Function | Parameters | Returns | Description |
|----------|-----------|---------|-------------|
| `registerPluginRoutes` | `(router: Router, bridge: PluginBridge)` | `void` | Register REST endpoints for plugin listing and invocation |

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

When the plugin host emits `plugin.tools_registered`, the bridge auto-registers each tool into the corvid-agent tool registry:

```typescript
socket.on('plugin.tools_registered', ({ pluginId, tools }) => {
  for (const tool of tools) {
    toolRegistry.register({
      name: `plugin:${pluginId}:${tool.name}`,
      description: tool.description,
      inputSchema: tool.inputSchema,  // JSON Schema v7 passthrough
      execute: (input) => bridge.invoke(pluginId, tool.name, input),
    });
  }
});
```

Tool names are namespaced as `plugin:<pluginId>:<toolName>` to avoid collisions with built-in tools.

### REST Endpoint

| Route | Method | Response | Description |
|-------|--------|----------|-------------|
| `/api/plugins` | GET | `PluginManifest[]` with tools | List all plugins and their tools |
| `/api/plugins/:id/invoke/:tool` | POST | `{ result: string }` | Invoke a specific plugin tool |

### TypeScript Types

```typescript
interface PluginManifest {
  id: string;
  version: string;
  author: string;
  description: string;
  capabilities: Capability[];
  trustTier: 'trusted' | 'verified' | 'untrusted';
  tools: ToolInfo[];
}

interface ToolInfo {
  name: string;
  description: string;
  inputSchema: Record<string, unknown>;  // JSON Schema v7
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
3. Auto-registration happens on `plugin.tools_registered` events — zero per-plugin TypeScript wiring required
4. The bridge reconnects automatically if the socket drops (with exponential backoff, max 30s)
5. MessagePack is used for tool invocation payloads (data plane); JSON for everything else (control plane)
6. The bridge does not validate plugin manifests — that's the host's responsibility
7. If the plugin host is not running, the bridge logs a warning and queues no requests — tools simply aren't registered
8. Tool `inputSchema` from plugins is passed through verbatim to the registry — no transformation

## Behavioral Examples

### Scenario: Plugin host starts with 3 plugins

- **Given** the plugin host starts and loads algo-oracle, code-snoop, memory-graph
- **When** each plugin loads, the host emits `plugin.tools_registered` for each
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
