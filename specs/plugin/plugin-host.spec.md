---
module: plugin-host
version: 1
status: active
files:
  - corvid-plugin-host/src/main.rs
  - corvid-plugin-host/src/engine.rs
  - corvid-plugin-host/src/loader.rs
  - corvid-plugin-host/src/registry.rs
  - corvid-plugin-host/src/executor.rs
  - corvid-plugin-host/src/sandbox.rs
  - corvid-plugin-host/src/discovery.rs
  - corvid-plugin-host/src/host_functions/messaging.rs
  - corvid-plugin-host/src/host_functions/storage.rs
  - corvid-plugin-host/src/host_functions/algo.rs
  - corvid-plugin-host/src/host_functions/http.rs
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
| `plugin.invoke` | `{ plugin_id: string, tool: string, input: Value }` | `{ result: string } \| { error: PluginError }` | **Planned** | Invoke a plugin tool — requires data plane |
| `plugin.event` | `{ event: PluginEvent }` | `{ ok: bool }` | **Planned** | Dispatch event to subscribing plugins |
| `health.check` | `{}` | `{ plugins: StatusMap, uptime_ms: u64 }` | **Implemented** | Health status |

### Exported Structs

| Struct | File | Description |
|--------|------|-------------|
| `PluginSlot` | `registry.rs` | Holds a plugin instance with hot-reload drain pattern |
| `CallGuard` | `registry.rs` | RAII guard tracking active calls for drain synchronization |
| `LoadedPlugin` | `loader.rs` | Validated plugin instance with manifest and tier |
| `SandboxLimits` | `sandbox.rs` | Per-tier memory, fuel, and timeout limits |

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
2. **Signature verification** — Ed25519 on WASM binary (Trusted tier only). **Note:** Currently a placeholder that logs a warning and accepts all Trusted plugins
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

### executor.rs — Event Dispatch

Routes `PluginEvent` to plugins whose `event_filter` matches the event kind. Respects `PluginSlot` state (skips draining/unloaded plugins). **Note:** Currently logs dispatch events but does not execute WASM event handlers — full WASM event calling is planned for the data plane integration.

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

### host_functions/ — Capability Implementations

Host function linking is implemented — capabilities are gated at instantiation time. The SSRF validation logic and storage backend architecture are in place. **Note:** The actual host function bodies are currently stubs returning `0`. Full implementations are planned for the data plane integration phase.

| File | Capability | Status | Description |
|------|-----------|--------|-------------|
| `http.rs` | `Network` | **Stub** (URL validation implemented, request execution stubbed) | Allowlisted outbound HTTP with SSRF mitigation |
| `storage.rs` | `Storage` | **Stub** (StorageBackend trait defined, KV ops stubbed) | Scoped key-value store per plugin namespace |
| `algo.rs` | `AlgoRead` | **Stub** | Read Algorand application state, account info |
| `messaging.rs` | `AgentMessage` | **Stub** | Send messages to agents matching `target_filter` |

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
10. Ed25519 signature verification happens BEFORE manifest extraction for Trusted tier — **currently a placeholder** accepting all Trusted plugins with a warning
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
