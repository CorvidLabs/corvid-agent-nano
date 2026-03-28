---
module: plugin-macros
version: 1
status: draft
files:
  - corvid-plugin-macros/src/lib.rs
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

### Generated WASM Exports

When `#[corvid_plugin]` is applied to `struct MyPlugin`, the macro generates:

| Export Function | Signature | Description |
|----------------|-----------|-------------|
| `corvid_plugin_abi_version` | `extern "C" fn() -> u32` | Returns `corvid_plugin_sdk::ABI_VERSION` |
| `plugin_manifest_ptr` | `extern "C" fn() -> u32` | Serializes manifest to WASM linear memory, returns pointer |
| `plugin_init` | `extern "C" fn(ctx_ptr: u32, ctx_len: u32) -> u32` | Deserializes `InitContext`, calls `init()`, serializes result |
| `plugin_execute_tool` | `extern "C" fn(name_ptr: u32, name_len: u32, input_ptr: u32, input_len: u32) -> u32` | Routes to correct `PluginTool::execute()`, serializes result |
| `plugin_on_event` | `extern "C" fn(event_ptr: u32, event_len: u32) -> u32` | Deserializes `PluginEvent`, calls `on_event()`, serializes result |
| `plugin_shutdown` | `extern "C" fn()` | Calls `shutdown()` on the plugin instance |

### Serialization Format

All data crossing the WASM boundary uses **MessagePack** (`rmp-serde`). The generated code:

1. Reads input from WASM linear memory at `(ptr, len)`
2. Deserializes MessagePack bytes into the appropriate Rust type
3. Calls the trait method
4. Serializes the result back to MessagePack
5. Writes to WASM linear memory and returns the pointer

## Invariants

1. The macro generates `#[no_mangle] pub extern "C"` functions only — no fat pointers, no vtables
2. Exactly one `#[corvid_plugin]` annotation per WASM module (one plugin per `.wasm`)
3. The macro creates a module-level `static` for the plugin instance (initialized in `plugin_init`)
4. All generated functions use `catch_unwind` in dev-mode to catch panics at the boundary
5. `corvid_plugin_abi_version()` always returns `corvid_plugin_sdk::ABI_VERSION` — never hardcoded
6. Tool routing in `plugin_execute_tool` matches by tool name string — O(n) over tools list (acceptable; plugins have <20 tools)
7. The macro must not add any runtime dependencies beyond `corvid-plugin-sdk` and `rmp-serde`

## Behavioral Examples

### Scenario: Basic plugin annotation

- **Given** a struct `AlgoOraclePlugin` implementing `CorvidPlugin`
- **When** `#[corvid_plugin]` is applied to the struct
- **Then** six `extern "C"` functions are generated: `corvid_plugin_abi_version`, `plugin_manifest_ptr`, `plugin_init`, `plugin_execute_tool`, `plugin_on_event`, `plugin_shutdown`

### Scenario: Tool routing

- **Given** a plugin with tools `["set_threshold", "fetch_app_state"]`
- **When** `plugin_execute_tool` is called with `name = "set_threshold"`
- **Then** the generated code finds the matching `PluginTool` by name and calls its `execute()`

### Scenario: Unknown tool name

- **Given** a plugin with tools `["set_threshold"]`
- **When** `plugin_execute_tool` is called with `name = "nonexistent"`
- **Then** returns serialized `PluginError::BadInput("unknown tool: nonexistent")`

## Error Cases

| Condition | Behavior |
|-----------|----------|
| Applied to an enum or union | Compile error: `#[corvid_plugin] can only be applied to structs` |
| Struct does not implement `CorvidPlugin` | Standard Rust compile error (trait not implemented) |
| Deserialization failure in generated code | Returns serialized `PluginError::BadInput` |
| Panic in plugin method (dev-mode) | Caught by `catch_unwind`, returns `PluginError::Exec` |
| Panic in plugin method (release) | WASM trap — caught at Wasmtime boundary in the host |

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
