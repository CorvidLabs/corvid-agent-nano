---
module: plugin-macros
version: 3
status: stable
files:
  - crates/corvid-plugin-macros/src/lib.rs
depends_on:
  - specs/plugin/plugin-sdk.spec.md
---

# Plugin Macros

## Purpose

Proc-macro crate (`proc-macro = true`) that generates WASM export glue for plugin authors. The `#[corvid_plugin]` attribute macro eliminates manual `extern "C"` boilerplate — plugin authors implement `CorvidPlugin` on a struct, apply the macro, and get correct WASM exports automatically. No `dyn Trait` fat pointers cross the WASM boundary; the macro generates pure `extern "C"` functions with pointer/length serialization.

## Public API

### Exported Functions (Macros)

| Function | Kind | Description |
|----------|------|-------------|
| `corvid_plugin` | Attribute macro | Applied to a struct that implements `CorvidPlugin`. Generates all WASM export functions |
| `corvid_tool` | Attribute macro | Applied to a struct to generate `PluginTool` trait implementation from annotations |

### Generated WASM Exports

When `#[corvid_plugin]` is applied to `struct MyPlugin`, the macro generates:

| Export Function | Signature | Description |
|----------------|-----------|-------------|
| **__corvid_abi_version** | `extern "C" fn() -> i32` | Returns `corvid_plugin_sdk::ABI_VERSION` cast to `i32` |
| **__corvid_manifest** | `extern "C" fn() -> i32` | Serializes manifest to MessagePack, returns ptr to length-prefixed buffer |
| **__corvid_invoke** | `extern "C" fn(tool_ptr: i32, tool_len: i32, input_ptr: i32, input_len: i32) -> i32` | Routes tool call by name, returns ptr to length-prefixed msgpack result |
| **__corvid_on_event** | `extern "C" fn(event_ptr: i32, event_len: i32) -> i32` | Deserializes `PluginEvent`, calls `on_event()`, returns 0 on success or -1 on error |
| **__corvid_shutdown** | `extern "C" fn()` | No-op shutdown hook — called before the WASM module is torn down |
| **__corvid_alloc** | `extern "C" fn(size: i32) -> i32` | WASM memory allocator for host→plugin data transfer |
| **__corvid_dealloc** | `extern "C" fn(ptr: i32, size: i32)` | WASM memory deallocator for cleanup |

### Return Value Encoding

Functions that return data use **length-prefix** encoding: the return value is an `i32` pointer into WASM linear memory pointing to `[4-byte LE u32 length][msgpack data]`. 0 means allocation failure or not-found.

`__corvid_on_event` is the exception: it returns a plain `i32` status code (0 = success, -1 = error/skip).

### Serialization Format

All data crossing the WASM boundary uses **MessagePack** (`rmp-serde`). The generated code:

1. Reads input from WASM linear memory at `(ptr, len)`
2. Deserializes MessagePack bytes into the appropriate Rust type
3. Calls the trait method
4. Serializes the result to MessagePack as `{"result": String}` or `{"error": String}`
5. Writes length-prefixed buffer to WASM linear memory and returns the ptr

### Payload Structures

| Function | Input | Output |
|----------|-------|--------|
| `__corvid_invoke` | tool name: raw UTF-8 bytes; input: msgpack `serde_json::Value` | msgpack `{"result": String}` or `{"error": String}` |
| `__corvid_on_event` | msgpack `PluginEvent` | `i32` status (0=ok, -1=error) |
| `__corvid_manifest` | none | msgpack `PluginManifest` |

### Instance Lifecycle

The macro creates a **fresh plugin instance on every `__corvid_invoke` call** via `Default::default()`. The WASM host creates a new `Store` per invocation, so WASM-level statics are reset. All persistent state must go through host KV functions (`host_kv_get` / `host_kv_set`).

## Invariants

1. The macro generates `#[unsafe(no_mangle)] pub extern "C"` functions only — no fat pointers, no vtables
2. Exactly one `#[corvid_plugin]` annotation per WASM module (one plugin per `.wasm`)
3. The annotated struct must implement both `CorvidPlugin` and `Default` — a fresh instance is created per invocation via `Default::default()`
4. All export names use `__corvid_` prefix to avoid collisions with user code
5. `__corvid_abi_version()` always returns `corvid_plugin_sdk::ABI_VERSION as i32` — never hardcoded
6. Tool routing in `__corvid_invoke` matches by tool name string — O(n) over tools list (acceptable; plugins have <20 tools)
7. The generated code depends on `corvid-plugin-sdk` and `rmp-serde` at compile time — the proc-macro crate itself only depends on `syn`, `quote`, `proc-macro2`
8. Memory management (`__corvid_alloc`/`__corvid_dealloc`) is always generated to support host→plugin data transfer
9. Generated exports are gated on `#[cfg(target_arch = "wasm32")]` — native test builds do not emit them

## Behavioral Examples

### Scenario: Basic plugin annotation

- **Given** a struct `AlgoOraclePlugin` implementing `CorvidPlugin` and `Default`
- **When** `#[corvid_plugin]` is applied to the struct
- **Then** seven `extern "C"` functions are generated (wasm32 only): `__corvid_abi_version`, `__corvid_manifest`, `__corvid_invoke`, `__corvid_on_event`, `__corvid_shutdown`, `__corvid_alloc`, `__corvid_dealloc`

### Scenario: Tool routing

- **Given** a plugin with tools `["set_threshold", "fetch_app_state"]`
- **When** `__corvid_invoke` is called with tool name bytes `"set_threshold"` and msgpack input
- **Then** the generated code finds the matching `PluginTool` by name, calls `execute()`, and returns a length-prefixed `{"result": ...}` buffer

### Scenario: Unknown tool name

- **Given** a plugin with tools `["set_threshold"]`
- **When** `__corvid_invoke` is called with tool name `"nonexistent"`
- **Then** returns ptr to length-prefixed msgpack `{"error": "unknown tool: nonexistent"}`

## Error Cases

| Condition | Behavior |
|-----------|----------|
| Applied to an enum or union | Compile error: `#[corvid_plugin] can only be applied to structs` |
| Struct does not implement `CorvidPlugin` | Standard Rust compile error (trait not implemented) |
| Struct does not implement `Default` | Standard Rust compile error (trait not implemented) |
| Deserialization failure in generated code | Returns length-prefixed msgpack `{"error": String}` |
| Panic in plugin method | WASM trap — caught at Wasmtime boundary in the host |

## Dependencies

### Consumes

| Module | What is used |
|--------|-------------|
| `corvid-plugin-sdk` | `ABI_VERSION`, `CorvidPlugin`, `PluginManifest`, `PluginError` types |
| `syn` | Rust syntax parsing for proc-macro |
| `quote` | Code generation for proc-macro |
| `proc-macro2` | Token stream manipulation |

### Consumed By

| Module | What is used |
|--------|-------------|
| All plugin crates | `#[corvid_plugin]` attribute macro |

## Configuration

None — this is a proc-macro crate with no runtime configuration.

## Change Log

| Date | Author | Change |
|------|--------|--------|
| 2026-03-28 | CorvidAgent | Initial spec from council synthesis (Issue #15) |
| 2026-04-06 | CorvidAgent | Updated to spec-sync v3.3.0 format — status: active → stable |
| 2026-03-28 | CorvidAgent | Promoted to active — updated export names (`__corvid_*`), signatures (`i64` packed returns), payload formats, added `#[corvid_tool]` macro, `__corvid_alloc`/`__corvid_dealloc`, removed unimplemented `catch_unwind` claim |
| 2026-04-18 | Jackdaw | v3 (spec-sync 4.x): corrected export names to match host ABI — `__corvid_tool_call` → `__corvid_invoke` (4-arg), `__corvid_handle_event` → `__corvid_on_event`, removed `__corvid_init`; changed return encoding from packed `i64` to length-prefixed `i32` ptr; documented fresh-instance-per-call invariant; added `wasm32`-only gating note |
