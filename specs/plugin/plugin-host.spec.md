---
module: plugin-host
version: 2
status: active
files:
  - crates/corvid-plugin-host/src/main.rs
  - crates/corvid-plugin-host/src/engine.rs
  - crates/corvid-plugin-host/src/loader.rs
  - crates/corvid-plugin-host/src/registry.rs
  - crates/corvid-plugin-host/src/executor.rs
  - crates/corvid-plugin-host/src/sandbox.rs
  - crates/corvid-plugin-host/src/discovery.rs
  - crates/corvid-plugin-host/src/host_functions/messaging.rs
  - crates/corvid-plugin-host/src/host_functions/storage.rs
  - crates/corvid-plugin-host/src/host_functions/algo.rs
  - crates/corvid-plugin-host/src/host_functions/http.rs
  - crates/corvid-plugin-host/src/host_functions/db.rs
  - crates/corvid-plugin-host/src/host_functions/fs.rs
  - crates/corvid-plugin-host/src/wasm_mem.rs
  - crates/corvid-plugin-host/src/invoke.rs
depends_on:
  - specs/plugin/plugin-sdk.spec.md
  - specs/plugin/plugin-macros.spec.md
---

# Plugin Host

## Purpose

The Rust sidecar binary that hosts WASM plugins for corvid-agent. Runs as a separate process communicating with the TypeScript server over a Unix domain socket. Manages the Wasmtime engine, plugin loading/validation/sandboxing, hot-reload under load, and host function dispatch. This binary is never published to crates.io — it is built and deployed alongside corvid-agent.

**Why sidecar over native addon:**
- Upgrade atomicity: server and plugin host have independent version lifecycles
- True hot-reload: kill/restart subprocess, impossible with dlopen
- Clean separation: TS server needs ~100 lines to integrate
- Panic isolation: plugin panics caught at Wasmtime boundary, never crash Bun

## Public API

### IPC Protocol (Unix Domain Socket)

| Plane | Format | Use |
|-------|--------|-----|
| Control plane | JSON | Manifest queries, plugin management, health checks |
| Data plane | MessagePack | Tool invocations, event dispatch, results |

### JSON-RPC Methods (Control Plane)

| Method | Parameters | Returns | Status | Description |
|--------|-----------|---------|--------|-------------|
| `plugin.list` | `{}` | `PluginManifest[]` | **Implemented** | List all loaded plugin manifests |
| `plugin.load` | `{ path: string, tier: string }` | `{ ok: bool, error?: string }` | **Implemented** | Load a WASM plugin from path |
| `plugin.unload` | `{ id: string }` | `{ ok: bool }` | **Implemented** | Gracefully unload a plugin (drain + shutdown) |
| `plugin.reload` | `{ id: string, path: string }` | `{ ok: bool, error?: string }` | **Implemented** | Hot-reload: drain → swap → activate |
| `plugin.tools` | `{ id?: string }` | `ToolInfo[]` | **Stub** (returns `[]`) | List tools — requires WASM tool schema extraction |
| `plugin.invoke` | `{ plugin_id: string, tool: string, input: Value }` | `{ result: Value } \| { error: string }` | **Implemented** | Invoke a plugin tool via `__corvid_invoke` WASM export |
| `plugin.event` | `{ event: PluginEvent }` | `{ ok: bool, dispatched: u32, errors: string[] }` | **Implemented** | Dispatch event to subscribing plugins via `__corvid_on_event` |
| `health.check` | `{}` | `{ plugins: StatusMap, uptime_ms: u64 }` | **Implemented** | Health status |

### Exported Structs

| Struct | File | Description |
|--------|------|-------------|
| `PluginSlot` | `registry.rs` | Holds a plugin instance with hot-reload drain pattern |
| `CallGuard` | `registry.rs` | RAII guard tracking active calls for drain synchronization |
| `LoadedPlugin` | `loader.rs` | Validated plugin instance with manifest and tier |
| `LoadError` | `loader.rs` | Error type for plugin loading failures |
| `SandboxLimits` | `sandbox.rs` | Per-tier memory, fuel, and timeout limits |
| `MemoryLimiter` | `sandbox.rs` | Memory constraint enforcer |
| `InvokeContext` | `invoke.rs` | Shared backends for plugin tool invocations and event dispatch |
| `StorageBackend` | `host_functions/storage.rs` | Pluggable key-value storage backend |
| `AlgoBackend` | `host_functions/algo.rs` | Pluggable Algorand state query backend |
| `AlgoQuery` | `host_functions/algo.rs` | Algorand state query request |
| `MessageDispatch` | `host_functions/messaging.rs` | Message delivery tracker |
| `MessagingBackend` | `host_functions/messaging.rs` | Pluggable agent message dispatch backend |
| `DbBackend` | `host_functions/db.rs` | Pluggable read-only database query backend |
| `FsBackend` | `host_functions/fs.rs` | Pluggable sandboxed filesystem read backend |
| `PluginRegistry` | `registry.rs` | Centralized registry managing all loaded plugins |
| `PluginState` | `registry.rs` | State enum for plugin lifecycle (ACTIVE, DRAINING, UNLOADED) |
| `ToolInfo` | `invoke.rs` | Tool metadata: name, description, input schema |
| `ListResponse` | `main.rs` | JSON-RPC list response wrapper |
| `ToolsResponse` | `invoke.rs` | Tool list response structure |
| `PluginToolEntry` | `invoke.rs` | Individual plugin tool entry in response |

### Exported Functions

| Function | File | Parameters | Returns | Description |
|----------|------|-----------|---------|-------------|
| `build_engine` | `engine.rs` | `cache_dir: &Path` | `Result<Engine>` | Build Wasmtime engine with AOT caching |
| `validate_manifest` | `loader.rs` | `m: &PluginManifest` | `Result<(), LoadError>` | Validate plugin manifest |
| `check_abi_version` | `loader.rs` | `version: u32` | `Result<()>` | Check ABI version compatibility |
| `parse_sig_file` | `loader.rs` | `path: &Path` | `Result<(Vec<u8>, Vec<u8>)>` | Parse Ed25519 signature file |
| `is_key_trusted` | `loader.rs` | `pubkey: &[u8]` | `bool` | Check if key is in trusted registry |
| `verify_signature` | `loader.rs` | `pubkey, sig, data` | `Result<()>` | Verify Ed25519 signature |
| `extract_manifest` | `loader.rs` | `wasm: &[u8]` | `Result<PluginManifest>` | Extract manifest from WASM module |
| `load_plugin` | `loader.rs` | `path: &Path`, `tier: TrustTier` | `Result<LoadedPlugin>` | Load and validate plugin |
| `list_plugins` | `invoke.rs` | `registry: &PluginRegistry` | `Vec<PluginManifest>` | List all loaded plugin manifests |
| `list_tools` | `invoke.rs` | `registry: &PluginRegistry`, `plugin_id?: String` | `Vec<ToolInfo>` | List available tools |
| `invoke_tool` | `invoke.rs` | `registry`, `plugin_id`, `tool`, `input` | async `Result<String>` | Execute plugin tool |
| `dispatch_event` | `executor.rs` | `registry`, `event` | async | Dispatch event to subscribing plugins |
| `dispatch_event_to_plugin` | `invoke.rs` | `plugin`, `event` | async `Result<()>` | Deliver event to single plugin |
| `for_tier` | `sandbox.rs` | `tier: TrustTier` | `SandboxLimits` | Get limits for trust tier |
| `is_ssrf_blocked` | `host_functions/http.rs` | `url: &str`, `allowlist: &[String]` | `bool` | Check SSRF blocklist |
| `validate_url` | `host_functions/http.rs` | `url: &str` | `Result<()>` | Validate HTTP URL format |
| `app_state` | `host_functions/algo.rs` | `query: &AlgoQuery` | async `Result<Value>` | Query Algorand app state |
| `send` | `host_functions/messaging.rs` | `msg`, `target_filter` | async `Result<()>` | Send agent message |
| `matches_target_filter` | `host_functions/messaging.rs` | `agent_id: &str`, `filter: &str` | `bool` | Test target filter match |
| `link` | `sandbox.rs` | `linker`, `backend`, `tier` | `Result<()>` | Link host functions to WASM |
| `read_bytes` | `wasm_mem.rs` | `store`, `ptr`, `len` | `Result<Vec<u8>>` | Read bytes from WASM memory |
| `read_str` | `wasm_mem.rs` | `store`, `ptr`, `len` | `Result<String>` | Read string from WASM memory |
| `write_response` | `wasm_mem.rs` | `store`, `data` | `Result<(i32, i32)>` | Write response to WASM memory |
| `set` | `host_functions/storage.rs` | `backend`, `key`, `value` | async `Result<()>` | Set KV pair in storage |

### Struct Methods

#### PluginRegistry

| Method | Parameters | Returns | Description |
|--------|-----------|---------|-------------|
| `register` | `id: String`, `slot: PluginSlot` | — | Register a loaded plugin |
| `get` | `id: &str` | `Option<Arc<PluginSlot>>` | Get plugin by ID |
| `reload` | `id: &str`, `plugin: LoadedPlugin` | `Result<()>` | Hot-reload a plugin |
| `list_manifests` | `()` | `Vec<PluginManifest>` | List all loaded plugin metadata |
| `health_status` | `()` | `StatusMap` | Get health status of all plugins |
| `len` | `()` | `usize` | Count of loaded plugins |
| `is_empty` | `()` | `bool` | Check if registry is empty |
| `dispatch_event_counted` | `event: &PluginEvent` | `(u32, Vec<String>)` | Dispatch event, return count and errors |

#### PluginSlot

| Method | Parameters | Returns | Description |
|--------|-----------|---------|-------------|
| `new` | `plugin: LoadedPlugin` | `Self` | Create a new slot for a plugin |
| `is_active` | `()` | `bool` | Check if plugin is active |
| `is_draining` | `()` | `bool` | Check if plugin is draining |
| `try_acquire` | `()` | `Option<CallGuard>` | Acquire a call guard (fails if draining) |
| `drain_and_reload` | `new_plugin: LoadedPlugin` | async `Result<()>` | Hot-reload: wait for calls, swap, resume |
| `unload` | `()` | async | Gracefully unload the plugin |
| `state_str` | `()` | `&str` | Get human-readable state string |

## Modules

### main.rs — Socket Server

Binds a Unix domain socket, accepts JSON-RPC requests, dispatches to the appropriate handler. Tokio async runtime.

Socket path: `{data_dir}/plugins.sock` (default `~/.corvid/plugins.sock`)

### engine.rs — Wasmtime Engine + AOT Cache

```rust
fn build_engine(cache_dir: &Path) -> Result<Engine>
```

Configures Wasmtime with AOT compilation cache. Cache directory defaults to `~/.corvid/cache/plugins/<agent-id>/`. Keyed by `(wasm_hash + compiler_version + cpu_features)`. First load: ~150ms per plugin. Cached: ~5ms per plugin.

### loader.rs — Plugin Loading Pipeline

Four-step load sequence, all before `init()`:

1. **ABI check** — Extract `__corvid_abi_version()`, verify within `[ABI_MIN_COMPATIBLE, ABI_VERSION]`
2. **Signature verification** — Ed25519 on WASM binary (Trusted tier only). Reads detached `.sig` file (hex pubkey + hex signature), verifies against `{data_dir}/trusted-keys/` registry
3. **Manifest extraction + validation** — ID regex, semver, min_host_version, capability audit
4. **Instantiation** — Create Wasmtime instance with tier-appropriate limits

```rust
fn validate_manifest(m: &PluginManifest) -> Result<(), LoadError>
fn load_plugin(path: &Path, tier: TrustTier) -> Result<LoadedPlugin>
```

### registry.rs — Plugin Registry + Hot-Reload

The `PluginSlot` drain pattern enables hot-reload under load without dropping in-flight requests.

```rust
pub struct PluginSlot {
    inner:        Arc<RwLock<Box<dyn CorvidPlugin>>>,
    active_calls: Arc<AtomicUsize>,
    state:        Arc<AtomicU8>,  // ACTIVE=0, DRAINING=1, UNLOADED=2
}
```

**Hot-reload sequence:**
1. Set state to `DRAINING` — new calls return `PluginError::Unavailable`
2. Wait up to 30s for `active_calls` to reach 0
3. Call `shutdown()` on old instance
4. Swap in new instance
5. Set state to `ACTIVE`
6. `scopeguard` ensures state resets to `ACTIVE` even if `init()` fails

### executor.rs — Event Dispatch (Legacy)

Routes `PluginEvent` to plugins whose `event_filter` matches the event kind. Respects `PluginSlot` state (skips draining/unloaded plugins). Legacy dispatcher that logs events without WASM execution — superseded by `invoke.rs` for real WASM event dispatch.

### invoke.rs — Tool Invocation & Event Dispatch

Creates per-call WASM `Store` instances with all host functions linked, then executes plugin exports:

- `invoke_tool()` — calls `__corvid_invoke(tool_ptr, tool_len, input_ptr, input_len) -> ptr` to execute a plugin tool. Input/output are msgpack-serialized JSON values.
- `dispatch_event_to_plugin()` — calls `__corvid_on_event(event_ptr, event_len) -> i32` to deliver events.
- `InvokeContext` — holds shared backends (`StorageBackend`, `AlgoBackend`, `MessagingBackend`) passed to each invocation.

Each invocation gets a fresh `Store` with fuel budget and memory limits — no state leaks between calls.

### sandbox.rs — Security Sandboxing

Enforces per-tier resource limits and capability-gated host function linking.

#### Per-Tier Limits

| Constraint | Trusted | Verified | Untrusted |
|------------|---------|----------|-----------|
| Memory | 128 MB | 32 MB | 4 MB |
| Fuel per call | 1B instructions | 100M | 10M |
| Wall-clock timeout | 30s | 5s | 1s |
| Network | Declared allowlist | Read-only, allowlist | None |
| Filesystem | None | None | None |
| DB read | Yes | Yes | No |
| Agent messaging | Yes (declared targets) | No | No |
| Algorand read | Yes | Yes | Block explorer only |
| Cross-plugin reads | Never | Never | Never |

#### WASM Host Function Gating

Host functions are linked at instantiation time, not checked at call time. Capabilities not granted to a plugin result in the host function not being linked — calling it from WASM triggers a trap.

```rust
fn link_host_functions(linker: &mut Linker<State>, caps: &[Capability])
```

**Critical:** Do NOT use `wasmtime_wasi::add_to_linker` for untrusted plugins. WASI P1 includes filesystem, clocks, random, and process arguments by default — this bypasses the capability model. Start with an **empty WASI context** and link only what capabilities grant.

#### SSRF Mitigation

| Concern | Mitigation |
|---------|------------|
| RFC1918 SSRF | Block 10.x, 172.16-31.x, 192.168.x.x |
| Localhost SSRF | Block 127.0.0.0/8, `::1` |
| Cloud metadata | Block `169.254.169.254`, `fd00::/8` |
| `file://` scheme | Reject non-http(s) schemes |
| Infinite loops | Wasmtime fuel: `store.set_fuel(tier.fuel_per_call)` |
| Memory bombs | `store.limiter(MemoryLimiter::new(tier.memory_limit))` |
| Wall-clock timeout | `tokio::time::timeout(tier.timeout, ...)` |
| SQL injection via db:read | Parse SQL AST, reject non-SELECT |
| Path traversal via fs | `O_NOFOLLOW` + `realpath` + prefix check |
| Symlink attacks | `O_NOFOLLOW` on all file opens |
| Cross-plugin data leaks | Per-plugin KV namespace, no cross-namespace reads by construction |

### discovery.rs — Plugin Discovery API

Handles `plugin.list` and `plugin.tools` JSON-RPC methods. Returns manifest and tool info for the TypeScript bridge to register into the MCP/skill registry.

### wasm_mem.rs — WASM Memory Access Helpers

Safe read/write operations across the WASM boundary. Used by all host functions.

- `read_bytes(caller, ptr, len)` — read bytes from plugin linear memory
- `read_str(caller, ptr, len)` — read UTF-8 string from plugin linear memory
- `write_response(caller, data)` — allocate via `__corvid_alloc`, write length-prefixed response

### host_functions/ — Capability Implementations

Host function linking is implemented — capabilities are gated at instantiation time. All four host function modules are fully implemented with WASM memory access and pluggable backends.

| File | Capability | Status | Description |
|------|-----------|--------|-------------|
| `http.rs` | `Network` | **Implemented** | Allowlisted outbound HTTP with SSRF mitigation via `ureq` |
| `storage.rs` | `Storage` | **Implemented** | Scoped key-value store per plugin namespace (in-memory) |
| `algo.rs` | `AlgoRead` | **Implemented** | Read Algorand application state via pluggable `AlgoBackend` (trait-based for testability) |
| `messaging.rs` | `AgentMessage` | **Implemented** | Send messages to agents via pluggable `MessagingBackend` with `target_filter` enforcement |

## Invariants

1. Plugin loading always follows the 4-step sequence: ABI check → signature → manifest → instantiation
2. Unknown capabilities in a manifest should cause a **hard load failure** — currently silently ignored (logged at debug level); enforcement planned
3. Host functions are linked at WASM instantiation time, not checked per-call
4. `PluginSlot.drain_and_reload()` waits up to 30s for in-flight calls before swapping
5. `scopeguard` on drain ensures state resets to `ACTIVE` even on `init()` failure
6. Default WASI linker (`wasmtime_wasi::add_to_linker`) is NEVER used for untrusted plugins
7. Each plugin gets its own Wasmtime `Store` — no shared mutable state between plugins
8. Per-plugin KV namespace isolation is enforced by construction (namespace prefix on all keys)
9. AOT cache is keyed by `(wasm_hash, compiler_version, cpu_features)` — no stale cache hits
10. Ed25519 signature verification happens BEFORE manifest extraction for Trusted tier — verifies detached `.sig` file and checks public key against `{data_dir}/trusted-keys/` registry
11. SQL queries from `DbRead` capability will be parsed and rejected if not SELECT-only — **not yet implemented** (no `db.rs` host function)
12. Native plugin loading (dlopen) is planned for `--features dev-mode` — **not yet implemented**
13. Panics in plugins are caught at the Wasmtime boundary — never propagate to the host process
14. The socket path is `{data_dir}/plugins.sock` — configurable via `--data-dir`

## Behavioral Examples

### Scenario: Cold start with AOT cache miss

- **Given** 10 plugins installed, no AOT cache
- **When** the host starts
- **Then** each plugin takes ~150ms to compile, total ~1.5s. AOT artifacts are cached for next start

### Scenario: Warm start with AOT cache hit

- **Given** 10 plugins installed, AOT cache populated
- **When** the host starts
- **Then** each plugin takes ~5ms to load, total ~50ms

### Scenario: Hot-reload a plugin

- **Given** plugin "algo-oracle" is active with 3 in-flight calls
- **When** `plugin.reload` is called with a new WASM binary
- **Then** new calls to "algo-oracle" return `Unavailable`; 3 in-flight calls complete; old instance shuts down; new instance loads and activates

### Scenario: Untrusted plugin attempts network access

- **Given** an untrusted plugin (no `Network` capability)
- **When** the plugin calls `host_http_get` from WASM
- **Then** WASM trap — function was never linked, call fails at the import level

### Scenario: Plugin attempts localhost SSRF

- **Given** a verified plugin with `Network { allowlist: ["api.example.com"] }`
- **When** the plugin calls `host_http_get("http://127.0.0.1/admin")`
- **Then** request blocked — URL fails SSRF validation (127.0.0.0/8 blocked)

## Error Cases

| Condition | Behavior |
|-----------|----------|
| ABI version mismatch | `LoadError`: "plugin ABI {n} incompatible with host [{min}, {max}]" |
| Ed25519 signature invalid (Trusted) | `LoadError`: signature verification failed |
| Invalid manifest ID | `LoadError`: ID does not match `^[a-z][a-z0-9-]{0,49}$` |
| Manifest requires newer host | `LoadError`: min_host_version > current |
| Unknown capability | `LoadError`: hard reject |
| Duplicate tool name | `LoadError`: manifest validation |
| Plugin `init()` fails | Plugin not loaded, error returned via JSON-RPC |
| Fuel exhausted during execution | WASM trap → `PluginError::Timeout` |
| Memory limit exceeded | WASM trap → `PluginError::Exec` |
| Wall-clock timeout | `tokio::time::timeout` → `PluginError::Timeout` |
| Drain timeout (30s) | Force swap — in-flight calls may get interrupted |
| Socket path already in use | Exit with error — another host instance is running |

## Dependencies

### Consumes

| Module | What is used |
|--------|-------------|
| `corvid-plugin-sdk` | All types: `CorvidPlugin`, `PluginManifest`, `Capability`, `TrustTier`, etc. |
| `wasmtime` | WASM runtime engine with component model + cache features |
| `tokio` | Async runtime for socket server and timeouts |
| `rmp-serde` | MessagePack serialization (data plane) |
| `serde_json` | JSON serialization (control plane) |
| `scopeguard` | Cleanup guard for drain-and-reload |
| `ed25519-dalek` | Signature verification for Trusted tier |
| `ureq` | Synchronous HTTP client for plugin outbound requests |

### Consumed By

| Module | What is used |
|--------|-------------|
| `server/plugins/rust-bridge.ts` | Unix socket client (TypeScript side) |
| `corvid-plugin-cli` | Sends reload signals via socket |

## Configuration

| Env Var / Flag | Default | Description |
|----------------|---------|-------------|
| `--data-dir` | `~/.corvid` | Base directory for socket, cache, plugin storage |
| `--socket-path` | `{data_dir}/plugins.sock` | Unix domain socket path |
| `--cache-dir` | `{data_dir}/cache/plugins/{agent-id}/` | Wasmtime AOT cache directory |
| `--log-level` | `info` | Tracing subscriber log level (`RUST_LOG` env filter) |
| `--agent-id` | (required) | Agent identity for cache isolation and context |

## WASM Target Notes

**v1: `wasm32-wasip1`** — stable, Wasmtime 22 supports it well.

**v1.1: WIT Component Model (`wasm32-wasip2`)** — define `.wit` interfaces now, migrate bindings when toolchain stabilizes. Eliminates manual ptr/len serialization.

## Change Log

| Date | Author | Change |
|------|--------|--------|
| 2026-03-28 | CorvidAgent | Initial spec from council synthesis (Issue #15) |
| 2026-03-28 | CorvidAgent | Promoted to active — added implementation status markers to RPC methods, host functions, and invariants. Documented stubs (tool discovery, invoke, event dispatch, host function bodies, Ed25519 verification) |
| 2026-03-28 | CorvidAgent | Implemented Ed25519 signature verification — detached `.sig` file format (hex pubkey + hex sig), trusted key registry at `{data_dir}/trusted-keys/*.pub` |
| 2026-03-28 | CorvidAgent | Phase A data plane: WASM memory access layer (`wasm_mem.rs`), real `host_kv_get`/`host_kv_set` with namespace isolation, real `host_http_get`/`host_http_post` via `ureq` with SSRF+allowlist validation |
| 2026-03-28 | CorvidAgent | Phase B data plane: `host_algo_state` with pluggable `AlgoBackend`, `host_send_message` with `MessagingBackend` + target_filter enforcement, `plugin.invoke` RPC via `__corvid_invoke` WASM export, `plugin.event` RPC via `__corvid_on_event`, `invoke.rs` execution module |
