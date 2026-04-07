//! Hello World — minimal test plugin for corvid-agent plugin host.
//!
//! Exports the required ABI: __corvid_abi_version, __corvid_alloc,
//! __corvid_manifest, __corvid_invoke.
//!
//! Tools:
//!   - "hello": returns {"greeting": "Hello, <name>!"} for input {"name": "..."}
//!   - "echo":  returns the input unchanged

use corvid_plugin_sdk::manifest::{PluginManifest, ToolInfo, TrustTier};
use std::alloc::{alloc, Layout};

// ── ABI version ─────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn __corvid_abi_version() -> i32 {
    corvid_plugin_sdk::ABI_VERSION as i32
}

// ── Allocator ───────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn __corvid_alloc(size: i32) -> i32 {
    let layout = Layout::from_size_align(size as usize, 4).unwrap();
    unsafe { alloc(layout) as i32 }
}

// ── Manifest ────────────────────────────────────────────────────────

/// Write a length-prefixed buffer to WASM memory and return the pointer.
fn write_response(data: &[u8]) -> i32 {
    let total = 4 + data.len();
    let ptr = __corvid_alloc(total as i32);
    if ptr == 0 {
        return 0;
    }
    let buf = unsafe { std::slice::from_raw_parts_mut(ptr as *mut u8, total) };
    buf[..4].copy_from_slice(&(data.len() as u32).to_le_bytes());
    buf[4..].copy_from_slice(data);
    ptr
}

#[no_mangle]
pub extern "C" fn __corvid_manifest() -> i32 {
    let manifest = PluginManifest {
        id: "hello-world".into(),
        version: "0.1.0".into(),
        author: "corvid".into(),
        description: "Minimal test plugin — hello and echo tools".into(),
        capabilities: vec![],
        event_filter: vec![],
        trust_tier: TrustTier::Untrusted,
        min_host_version: "0.1.0".into(),
        tools: vec![
            ToolInfo {
                name: "hello".into(),
                description: "Returns a greeting for the given name".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Name to greet" }
                    }
                }),
            },
            ToolInfo {
                name: "echo".into(),
                description: "Returns the input unchanged".into(),
                input_schema: serde_json::json!({
                    "type": "object"
                }),
            },
        ],
        dependencies: vec![],
    };

    let bytes = rmp_serde::to_vec(&manifest).unwrap();
    write_response(&bytes)
}

// ── Tool invocation ─────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn __corvid_invoke(
    tool_ptr: i32,
    tool_len: i32,
    input_ptr: i32,
    input_len: i32,
) -> i32 {
    // Read tool name from WASM memory
    let tool_name = unsafe {
        let slice = std::slice::from_raw_parts(tool_ptr as *const u8, tool_len as usize);
        std::str::from_utf8(slice).unwrap_or("unknown")
    };

    // Read input from WASM memory (msgpack)
    let input_bytes =
        unsafe { std::slice::from_raw_parts(input_ptr as *const u8, input_len as usize) };

    let input: serde_json::Value = rmp_serde::from_slice(input_bytes).unwrap_or_default();

    let result = match tool_name {
        "hello" => {
            let name = input
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("World");
            serde_json::json!({
                "greeting": format!("Hello, {}!", name)
            })
        }
        "echo" => input,
        _ => serde_json::json!({
            "error": format!("unknown tool: {}", tool_name)
        }),
    };

    let result_bytes = rmp_serde::to_vec(&result).unwrap();
    write_response(&result_bytes)
}
