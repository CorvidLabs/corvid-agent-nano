---
module: plugin-macros
version: 1
status: active
files:
  - crates/corvid-plugin-macros/src/lib.rs
depends_on:
  - specs/plugin/plugin-sdk.spec.md
---

# Plugin Macros

## Purpose

Proc-macro crate (`proc-macro = true`) that generates WASM export glue for plugin authors. The `#[corvid_plugin]` attribute macro eliminates manual `extern "C"` boilerplate — plugin authors implement `CorvidPlugin` on a struct, apply the macro, and get correct WASM exports automatically. No `dyn Trait` fat pointers cross the WASM boundary; the macro generates pure `extern "C"` functions with pointer/length serialization.

## Public API

### Exported Macros

| Macro | Kind | Description |
|-------|------|-------------|
| `#[corvid_plugin]` | Attribute macro | Applied to a struct that implements `CorvidPlugin`. Generates all WASM export functions |
| `#[corvid_tool]` | Attribute macro | Applied to a struct to generate `PluginTool` trait implementation from annotations |

### Generated WASM Exports

When `#[corvid_plugin]` is applied to `struct MyPlugin`, the macro generates:

| Export Function | Signature | Description |
|----------------|-----------|-------------|
| `__corvid_abi_version` | `extern "C" fn() -> u32` | Returns `corvid_plugin_sdk::ABI_VERSION` |
| `__corvid_manifest` | `extern "C" fn() -> i64` | Serializes manifest to MessagePack, returns packed `(ptr << 32 \| len)` |
| `__corvid_init` | `extern "C" fn(ptr: i32, len: i32) -> i64` | Deserializes init payload `{ agent_id, host_version }`, calls `init()`, returns packed result |
| `__corvid_tool_call` | `extern "C" fn(ptr: i32, len: i32) -> i64` | Deserializes `{ tool, input, session_id }`, routes to `PluginTool::execute()`, returns packed result |
| `__corvid_handle_event` | `extern "C" fn(ptr: i32, len: i32) -> i64` | Deserializes `PluginEvent`, calls `on_event()`, returns packed result |
| `__corvid_shutdown` | `extern "C" fn()` | Calls `shutdown()` on the plugin instance |
| `__corvid_alloc` | `extern "C" fn(len: i32) -> i32` | WASM memory allocator for host→plugin data transfer |
| `__corvid_dealloc` | `extern "C" fn(ptr: i32, len: i32)` | WASM memory deallocator for cleanup |

### Return Value Encoding

Functions that return data use **packed `i64`** encoding: `(pointer << 32) | length`. The upper 32 bits hold the pointer into WASM linear memory, and the lower 32 bits hold the byte length of the serialized data. This avoids multi-return or out-parameter patterns.

### Serialization Format

All data crossing the WASM boundary uses **MessagePack** (`rmp-serde`). The generated code:

1. Reads input from WASM linear memory at `(ptr, len)`
2. Deserializes MessagePack bytes into the appropriate Rust type
3. Calls the trait method
4. Serializes the result to MessagePack as `Result<String, String>`
5. Writes to WASM linear memory and returns the packed `i64` pointer/length

### Payload Structures

| Function | Input Payload | Output Payload |
|----------|--------------|----------------|
| `__corvid_init` | `{ agent_id: String, host_version: String }` | `Result<(), String>` |
| `__corvid_tool_call` | `{ tool: String, input: Value, session_id: String }` | `Result<String, String>` |
| `__corvid_handle_event` | `PluginEvent` (MessagePack) | `Result<(), String>` |

## Invariants

1. The macro generates `#[no_mangle] pub extern "C"` functions only — no fat pointers, no vtables
2. Exactly one `#[corvid_plugin]` annotation per WASM module (one plugin per `.wasm`)
3. The macro creates a module-level `static INSTANCE: Mutex<Option<T>>` for the plugin instance (initialized in `__corvid_init`)
4. All export names use `__corvid_` prefix to avoid collisions with user code
5. `__corvid_abi_version()` always returns `corvid_plugin_sdk::ABI_VERSION` — never hardcoded
6. Tool routing in `__corvid_tool_call` matches by tool name string — O(n) over tools list (acceptable; plugins have <20 tools)
7. The generated code depends on `corvid-plugin-sdk` and `rmp-serde` at compile time — the proc-macro crate itself only depends on `syn`, `quote`, `proc-macro2`
8. Memory management (`__corvid_alloc`/`__corvid_dealloc`) is always generated to support host→plugin data transfer

## Behavioral Examples

### Scenario: Basic plugin annotation

- **Given** a struct `AlgoOraclePlugin` implementing `CorvidPlugin`
- **When** `#[corvid_plugin]` is applied to the struct
- **Then** eight `extern "C"` functions are generated: `__corvid_abi_version`, `__corvid_manifest`, `__corvid_init`, `__corvid_tool_call`, `__corvid_handle_event`, `__corvid_shutdown`, `__corvid_alloc`, `__corvid_dealloc`

### Scenario: Tool routing

- **Given** a plugin with tools `["set_threshold", "fetch_app_state"]`
- **When** `__corvid_tool_call` is called with payload `{ tool: "set_threshold", input: {...}, session_id: "..." }`
- **Then** the generated code finds the matching `PluginTool` by name and calls its `execute()`

### Scenario: Unknown tool name

- **Given** a plugin with tools `["set_threshold"]`
- **When** `__corvid_tool_call` is called with `tool = "nonexistent"`
- **Then** returns packed `i64` pointing to MessagePack-serialized `Err("unknown tool: nonexistent")`

## Error Cases

| Condition | Behavior |
|-----------|----------|
| Applied to an enum or union | Compile error: `#[corvid_plugin] can only be applied to structs` |
| Struct does not implement `CorvidPlugin` | Standard Rust compile error (trait not implemented) |
| Deserialization failure in generated code | Returns serialized `Err(String)` via packed `i64` |
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
| 2026-03-28 | CorvidAgent | Promoted to active — updated export names (`__corvid_*`), signatures (`i64` packed returns), payload formats, added `#[corvid_tool]` macro, `__corvid_alloc`/`__corvid_dealloc`, removed unimplemented `catch_unwind` claim |
