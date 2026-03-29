---
module: plugin-sdk
version: 1
status: active
files:
  - crates/corvid-plugin-sdk/src/lib.rs
  - crates/corvid-plugin-sdk/src/manifest.rs
  - crates/corvid-plugin-sdk/src/capability.rs
  - crates/corvid-plugin-sdk/src/context.rs
  - crates/corvid-plugin-sdk/src/tool.rs
  - crates/corvid-plugin-sdk/src/error.rs
  - crates/corvid-plugin-sdk/src/host_api.rs
  - crates/corvid-plugin-sdk/src/service.rs
depends_on: []
---

# Plugin SDK

## Purpose

The stable public contract for corvid-agent plugin authors. Published to crates.io as `corvid-plugin-sdk`. Defines the `CorvidPlugin` trait, `PluginTool` trait, capability system, manifest format, error types, and host function signatures. Plugin authors depend on this crate (plus optionally `corvid-plugin-macros`) and nothing else — zero runtime dependencies (no tokio, no algosdk, no wasmtime).

This crate is the **semver stability boundary**. Breaking changes bump `ABI_VERSION`. Non-breaking additions use `#[non_exhaustive]` on enums.

## Public API

### Exported Modules

| Module | Description |
|--------|-------------|
| `capability` | Capability enum and Display impl |
| `context` | InitContext and ToolContext structs |
| `error` | PluginError, PluginEvent, and EventKind types |
| `host_api` | WASM host function imports (extern "C") |
| `manifest` | PluginManifest and TrustTier types |
| `service` | Host-provided service traits |
| `tool` | PluginTool trait |

### Exported Constants

| Constant | Type | Value | Description |
|----------|------|-------|-------------|
| `ABI_VERSION` | `u32` | `1` | Current ABI version. Bumped on breaking trait/type layout changes |
| `ABI_MIN_COMPATIBLE` | `u32` | `1` | Oldest ABI the host will accept. Maintains a 1-major window |

### Exported Traits

| Trait | Description |
|-------|-------------|
| `CorvidPlugin` | Core plugin interface — manifest, tools, init, event handling, shutdown |
| `PluginTool` | A discrete callable unit within a plugin — name, description, JSON Schema input, execute |
| `StorageService` | Host-provided scoped key-value storage |
| `HttpService` | Host-provided allowlisted outbound HTTP |
| `DbReadService` | Host-provided read-only database access |
| `FsReadService` | Host-provided sandboxed filesystem read |
| `AlgoReadService` | Host-provided Algorand chain read access |
| `MessagingService` | Host-provided agent message bus |

### Exported Structs

| Struct | Description |
|--------|-------------|
| `PluginManifest` | Static metadata: id, version, author, capabilities, event filter, trust tier, min host version |
| `InitContext` | Passed to `init()` — agent ID, host version, capability-gated service handles |
| `ToolContext` | Passed to tool execution and event handling — agent ID, session ID, granted capabilities |

### Exported Enums

| Enum | Description |
|------|-------------|
| `Capability` | `#[non_exhaustive]` — Network, Storage, AlgoRead, DbRead, FsProjectDir, AgentMessage |
| `TrustTier` | Trusted, Verified, Untrusted |
| `PluginEvent` | AgentMessage, AlgoTransaction, ScheduledTick, HttpWebhook |
| `EventKind` | Discriminant-only version of `PluginEvent` for subscription filtering |
| `PluginError` | Init, Exec, MissingCapability, BadInput, Timeout, Unavailable |

### CorvidPlugin Trait

| Method | Parameters | Returns | Description |
|--------|-----------|---------|-------------|
| `manifest` | `()` (where Self: Sized) | `PluginManifest` | Static metadata — called at load time before instantiation |
| `tools` | `(&self)` | `&[Box<dyn PluginTool>]` | Tools this plugin exposes. Called after manifest validation |
| `init` | `(&mut self, ctx: InitContext)` | `Result<(), PluginError>` | Called once after instantiation with capability-gated context |
| `on_event` | `(&mut self, event: PluginEvent, ctx: &ToolContext)` | `Result<(), PluginError>` | Handle events matching declared `event_filter`. Default no-op |
| `shutdown` | `(&mut self)` | `()` | Called before unload. Errors logged and ignored — must not panic |

### PluginTool Trait

| Method | Parameters | Returns | Description |
|--------|-----------|---------|-------------|
| `name` | `(&self)` | `&str` | Unique tool name within the plugin |
| `description` | `(&self)` | `&str` | Human-readable description |
| `input_schema` | `(&self)` | `serde_json::Value` | JSON Schema v7 — directly usable by TS registry and MCP bridge |
| `execute` | `(&self, input: serde_json::Value, ctx: &ToolContext)` | `Result<String, PluginError>` | Sync execution — host wraps in blocking thread pool |

### PluginManifest Fields

| Field | Type | Description |
|-------|------|-------------|
| `id` | `String` | Plugin ID. Regex: `^[a-z][a-z0-9-]{0,49}$` |
| `version` | `String` | Semver version string |
| `author` | `String` | Author name or organization |
| `description` | `String` | Human-readable description |
| `capabilities` | `Vec<Capability>` | Required capabilities — host rejects unknown capabilities |
| `event_filter` | `Vec<EventKind>` | Events this plugin subscribes to |
| `trust_tier` | `TrustTier` | Declared tier (hint — host assigns actual tier) |
| `min_host_version` | `String` | Minimum compatible host version (semver) |

### Capability Variants

| Variant | Fields | Description |
|---------|--------|-------------|
| `Network` | `allowlist: Vec<String>` | Outbound HTTP to allowlisted domains |
| `Storage` | `namespace: String` | Scoped key-value storage |
| `AlgoRead` | — | Read-only Algorand chain access |
| `DbRead` | — | Read-only database access (SELECT only) |
| `FsProjectDir` | — | Read-only filesystem access within project dir |
| `AgentMessage` | `target_filter: String` | Send messages to matching agents ("broadcast" or specific) |

### InitContext Fields

| Field | Type | Description |
|-------|------|-------------|
| `agent_id` | `String` | Current agent's identity |
| `host_version` | `String` | Plugin host version |
| `storage` | `Option<Arc<dyn StorageService>>` | Present only when `Storage` capability granted |
| `http` | `Option<Arc<dyn HttpService>>` | Present only when `Network` capability granted |
| `db` | `Option<Arc<dyn DbReadService>>` | Present only when `DbRead` capability granted |
| `fs` | `Option<Arc<dyn FsReadService>>` | Present only when `FsProjectDir` capability granted |
| `algo` | `Option<Arc<dyn AlgoReadService>>` | Present only when `AlgoRead` capability granted |
| `messaging` | `Option<Arc<dyn MessagingService>>` | Present only when `AgentMessage` capability granted |

### PluginEvent Variants

| Variant | Fields | Description |
|---------|--------|-------------|
| `AgentMessage` | `from: String, content: serde_json::Value` | Incoming agent message |
| `AlgoTransaction` | `txid: String` | Relevant on-chain transaction |
| `ScheduledTick` | `interval_ms: u64, counter: u64` | Periodic timer tick |
| `HttpWebhook` | `path: String, body: serde_json::Value` | Incoming webhook request |

### PluginError Variants

| Variant | Description |
|---------|-------------|
| `Init(String)` | Plugin initialization failed |
| `Exec(String)` | Tool execution failed |
| `MissingCapability(Capability)` | Required capability not granted |
| `BadInput(String)` | Invalid input to tool |
| `Timeout` | Execution exceeded wall-clock limit |
| `Unavailable` | Plugin is draining (hot-reload in progress) |

### Exported Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `kind` | `(&self) -> EventKind` | Get the discriminant kind of a PluginEvent |
| `host_kv_get` | `extern "C" fn(key_ptr: i32, key_len: i32) -> i32` | Scoped KV read |
| `host_kv_set` | `extern "C" fn(key_ptr: i32, key_len: i32, val_ptr: i32, val_len: i32) -> i32` | Scoped KV write |
| `host_http_get` | `extern "C" fn(url_ptr: i32, url_len: i32) -> i32` | Allowlisted HTTP GET |
| `host_http_post` | `extern "C" fn(url_ptr: i32, url_len: i32, body_ptr: i32, body_len: i32) -> i32` | Allowlisted HTTP POST |
| `host_db_query` | `extern "C" fn(sql_ptr: i32, sql_len: i32) -> i32` | Read-only SQL query |
| `host_fs_read` | `extern "C" fn(path_ptr: i32, path_len: i32) -> i32` | Sandboxed file read |
| `host_algo_state` | `extern "C" fn(app_id: i64, key_ptr: i32, key_len: i32) -> i32` | Algorand app state read |
| `host_send_message` | `extern "C" fn(target_ptr: i32, target_len: i32, msg_ptr: i32, msg_len: i32) -> i32` | Agent message send |

All host functions use MessagePack serialization across the WASM boundary. Return values are pointers to MessagePack-encoded response buffers in WASM linear memory.

## Invariants

1. `CorvidPlugin` requires `Send + Sync + 'static`
2. `PluginManifest.id` must match regex `^[a-z][a-z0-9-]{0,49}$`
3. `PluginManifest.version` must be valid semver
4. `Capability` is `#[non_exhaustive]` — unknown capabilities cause **hard load failure**, not silent drop
5. Plugin tools must have unique names within a single plugin
6. `on_event` has a default no-op implementation — plugins only override if they subscribe to events
7. `shutdown()` must not panic — errors are logged and ignored
8. `InitContext` service handles are `None` for capabilities not granted to the plugin
9. This crate has **zero runtime dependencies** — no tokio, no algosdk, no wasmtime
10. All serialization across the WASM boundary uses MessagePack (`rmp-serde`)
11. ABI version is a `u32`, not semver — non-breaking additions keep `ABI_VERSION` unchanged
12. `ABI_MIN_COMPATIBLE` only moves forward on genuine breaks, maintains a 1-major window

## Behavioral Examples

### Scenario: Plugin declares unknown capability

- **Given** a plugin manifest declares `Capability::FutureFeature` (unknown to the current host)
- **When** the host loads the plugin
- **Then** the load fails with a hard error (not silently dropped)

### Scenario: Plugin tool execution

- **Given** a loaded plugin with tool "fetch_price"
- **When** the bridge invokes `execute` with `{"pair": "ALGO/USD"}`
- **Then** the tool returns a JSON string result or a `PluginError`

### Scenario: Service handle availability

- **Given** a plugin with capabilities `[AlgoRead, Storage]` but NOT `DbRead`
- **When** `init()` is called with `InitContext`
- **Then** `ctx.algo` is `Some(...)`, `ctx.storage` is `Some(...)`, `ctx.db` is `None`

## Error Cases

| Condition | Behavior |
|-----------|----------|
| Invalid manifest ID format | Host rejects at load time with `LoadError` |
| Invalid semver version | Host rejects at load time with `LoadError` |
| Unknown capability variant | Host rejects at load time — hard failure |
| ABI version out of range | Host rejects: `"plugin ABI {n} incompatible with host [{min}, {max}]"` |
| Tool name collision within plugin | Host rejects at manifest validation |
| `init()` returns Err | Plugin not loaded, error logged |
| `shutdown()` panics | Caught by host, error logged, plugin unloaded anyway |

## Dependencies

### Consumes

| Module | What is used |
|--------|-------------|
| `serde` | Derive macros for Serialize/Deserialize |
| `serde_json` | JSON Schema support for `PluginTool::input_schema` |
| `thiserror` | `PluginError` derive |
| `rmp-serde` | MessagePack serialization (WASM boundary) |

### Consumed By

| Module | What is used |
|--------|-------------|
| `corvid-plugin-macros` | `CorvidPlugin`, `PluginManifest`, `ABI_VERSION` |
| `corvid-plugin-host` | All exported types |
| All plugin crates | `CorvidPlugin`, `PluginTool`, `Capability`, `PluginManifest` |

## Configuration

None — this is a library crate with no runtime configuration.

## Change Log

| Date | Author | Change |
|------|--------|--------|
| 2026-03-28 | CorvidAgent | Initial spec from council synthesis (Issue #15) |
| 2026-03-28 | CorvidAgent | Promoted to active — field types updated to match implementation (String/Vec vs static refs) |
