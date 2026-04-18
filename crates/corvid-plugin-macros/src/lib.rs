//! Proc macros for corvid-agent plugin authoring.
//!
//! Provides `#[corvid_plugin]` which generates WASM export boilerplate:
//! - `__corvid_abi_version` — returns ABI version constant
//! - `__corvid_manifest` — returns ptr to length-prefixed msgpack manifest
//! - `__corvid_invoke` — routes tool invocations (4-arg: tool + input ptr/len pairs)
//! - `__corvid_on_event` — handles events (2-arg: event ptr/len)
//! - `__corvid_shutdown` — calls `shutdown()`
//! - `__corvid_alloc` / `__corvid_dealloc` — WASM memory management
//!
//! Also provides `#[corvid_tool]` for generating `PluginTool` impls.
//!
//! ## Host ABI contract
//!
//! The host calls `__corvid_invoke(tool_ptr, tool_len, input_ptr, input_len) -> i32`
//! where the tool name is raw UTF-8 bytes at (tool_ptr, tool_len) and the input
//! is msgpack-serialized JSON at (input_ptr, input_len). The return value is a
//! pointer to `[4-byte LE len][msgpack data]` in WASM linear memory.
//!
//! `__corvid_manifest()` returns a pointer to the same length-prefixed format.
//!
//! Host functions (`host_kv_get`, `host_http_post`, etc.) also return pointers
//! to length-prefixed buffers; 0 means not-found or error.

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemStruct};

/// Generates WASM export glue for a struct implementing `CorvidPlugin`.
///
/// The struct must also implement `Default` — the macro creates a fresh instance
/// on every `__corvid_invoke` call because the WASM host creates a new Store per
/// invocation. All persistent state must go through host KV functions.
///
/// # Usage
/// ```ignore
/// use corvid_plugin_sdk::prelude::*;
/// use corvid_plugin_macros::corvid_plugin;
///
/// #[corvid_plugin]
/// #[derive(Default)]
/// struct MyPlugin;
///
/// impl CorvidPlugin for MyPlugin {
///     fn manifest() -> PluginManifest { /* ... */ }
///     fn tools(&self) -> &[Box<dyn PluginTool>] { /* ... */ }
///     fn init(&mut self, _ctx: InitContext) -> Result<(), PluginError> { Ok(()) }
/// }
/// ```
#[proc_macro_attribute]
pub fn corvid_plugin(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemStruct);
    let struct_name = &input.ident;

    let expanded = quote! {
        #input

        // ── WASM exports (only compiled for wasm32 target) ──────────────

        #[cfg(target_arch = "wasm32")]
        mod __corvid_wasm_exports {
            use super::*;
            use corvid_plugin_sdk::{CorvidPlugin, ABI_VERSION};

            // ── Memory helpers ───────────────────────────────────────────

            /// Allocate `total` bytes and write `[4-byte LE len][data]`.
            /// Returns the ptr, or 0 on failure.
            fn write_response(data: &[u8]) -> i32 {
                let total_len = 4 + data.len();
                let ptr = __corvid_alloc(total_len as i32);
                if ptr == 0 {
                    return 0;
                }
                unsafe {
                    let dest = ptr as *mut u8;
                    let len_bytes = (data.len() as u32).to_le_bytes();
                    std::ptr::copy_nonoverlapping(len_bytes.as_ptr(), dest, 4);
                    std::ptr::copy_nonoverlapping(data.as_ptr(), dest.add(4), data.len());
                }
                ptr
            }

            /// Read raw bytes from WASM linear memory at (ptr, len).
            unsafe fn read_bytes(ptr: i32, len: i32) -> Vec<u8> {
                std::slice::from_raw_parts(ptr as *const u8, len as usize).to_vec()
            }

            // ── WASM exports ─────────────────────────────────────────────

            /// Returns the ABI version this plugin was compiled against.
            #[unsafe(no_mangle)]
            pub extern "C" fn __corvid_abi_version() -> i32 {
                ABI_VERSION as i32
            }

            /// Returns ptr to `[4-byte LE len][msgpack PluginManifest]`.
            #[unsafe(no_mangle)]
            pub extern "C" fn __corvid_manifest() -> i32 {
                let manifest = <#struct_name as CorvidPlugin>::manifest();
                let data = rmp_serde::to_vec(&manifest).expect("manifest serialization failed");
                write_response(&data)
            }

            /// Invoke a tool. Called by the host with separate ptr/len for tool name
            /// and input. Returns ptr to `[4-byte LE len][msgpack {"result":...}|{"error":...}]`.
            ///
            /// Creates a fresh plugin instance on every call — the WASM host creates
            /// a new Store per invocation, so statics are reset. Use KV storage for
            /// persistent state.
            #[unsafe(no_mangle)]
            pub extern "C" fn __corvid_invoke(
                tool_ptr: i32,
                tool_len: i32,
                input_ptr: i32,
                input_len: i32,
            ) -> i32 {
                let tool_bytes = unsafe { read_bytes(tool_ptr, tool_len) };
                let tool_name = match std::str::from_utf8(&tool_bytes) {
                    Ok(s) => s.to_string(),
                    Err(_) => {
                        let resp = rmp_serde::to_vec(&serde_json::json!({"error": "invalid UTF-8 tool name"}))
                            .unwrap_or_default();
                        return write_response(&resp);
                    }
                };

                let input_bytes = unsafe { read_bytes(input_ptr, input_len) };
                let input: serde_json::Value = match rmp_serde::from_slice(&input_bytes) {
                    Ok(v) => v,
                    Err(e) => {
                        let resp = rmp_serde::to_vec(&serde_json::json!({"error": format!("bad input: {e}")}))
                            .unwrap_or_default();
                        return write_response(&resp);
                    }
                };

                // Fresh instance each call — statics reset by the host's new Store.
                let plugin = <#struct_name as Default>::default();

                let manifest = <#struct_name as CorvidPlugin>::manifest();
                let ctx = corvid_plugin_sdk::ToolContext {
                    agent_id: String::new(),
                    session_id: String::new(),
                    capabilities: manifest.capabilities.clone(),
                };

                let result = plugin
                    .tools()
                    .iter()
                    .find(|t| t.name() == tool_name.as_str())
                    .map(|t| t.execute(input, &ctx))
                    .unwrap_or_else(|| {
                        Err(corvid_plugin_sdk::PluginError::BadInput(format!(
                            "unknown tool: {tool_name}"
                        )))
                    });

                let resp = match result {
                    Ok(s) => rmp_serde::to_vec(&serde_json::json!({"result": s})),
                    Err(e) => rmp_serde::to_vec(&serde_json::json!({"error": format!("{e}")})),
                };
                write_response(&resp.unwrap_or_default())
            }

            /// Handle an event. Returns 0 on success, -1 on error/skip.
            #[unsafe(no_mangle)]
            pub extern "C" fn __corvid_on_event(event_ptr: i32, event_len: i32) -> i32 {
                let data = unsafe { read_bytes(event_ptr, event_len) };
                let event: corvid_plugin_sdk::PluginEvent = match rmp_serde::from_slice(&data) {
                    Ok(e) => e,
                    Err(_) => return -1,
                };

                let mut plugin = <#struct_name as Default>::default();
                let manifest = <#struct_name as CorvidPlugin>::manifest();
                let ctx = corvid_plugin_sdk::ToolContext {
                    agent_id: String::new(),
                    session_id: String::new(),
                    capabilities: manifest.capabilities.clone(),
                };

                match plugin.on_event(event, &ctx) {
                    Ok(()) => 0,
                    Err(_) => -1,
                }
            }

            /// Shutdown hook — called before the WASM module is torn down.
            #[unsafe(no_mangle)]
            pub extern "C" fn __corvid_shutdown() {}

            /// WASM memory allocator — host calls this to reserve space for inputs.
            #[unsafe(no_mangle)]
            pub extern "C" fn __corvid_alloc(size: i32) -> i32 {
                let layout =
                    std::alloc::Layout::from_size_align(size as usize, 1).expect("invalid alloc layout");
                unsafe { std::alloc::alloc(layout) as i32 }
            }

            /// WASM memory deallocator.
            #[unsafe(no_mangle)]
            pub extern "C" fn __corvid_dealloc(ptr: i32, size: i32) {
                let layout =
                    std::alloc::Layout::from_size_align(size as usize, 1).expect("invalid dealloc layout");
                unsafe { std::alloc::dealloc(ptr as *mut u8, layout) }
            }
        }
    };

    TokenStream::from(expanded)
}

/// Derive macro for generating `PluginTool` boilerplate from a struct.
///
/// # Usage
/// ```ignore
/// #[corvid_tool(name = "hello", description = "Says hello")]
/// struct HelloTool;
///
/// impl HelloTool {
///     fn run(&self, input: serde_json::Value, ctx: &ToolContext) -> Result<String, PluginError> {
///         Ok("Hello!".into())
///     }
/// }
/// ```
#[proc_macro_attribute]
pub fn corvid_tool(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemStruct);
    let struct_name = &input.ident;

    // Parse key = "value" pairs from attribute
    let attr_str = attr.to_string();
    let name = extract_attr_value(&attr_str, "name")
        .unwrap_or_else(|| struct_name.to_string().to_lowercase());
    let description = extract_attr_value(&attr_str, "description").unwrap_or_default();

    let expanded = quote! {
        #input

        impl corvid_plugin_sdk::tool::PluginTool for #struct_name {
            fn name(&self) -> &str {
                #name
            }

            fn description(&self) -> &str {
                #description
            }

            fn input_schema(&self) -> serde_json::Value {
                self.schema()
            }

            fn execute(
                &self,
                input: serde_json::Value,
                ctx: &corvid_plugin_sdk::context::ToolContext,
            ) -> Result<String, corvid_plugin_sdk::error::PluginError> {
                self.run(input, ctx)
            }
        }
    };

    TokenStream::from(expanded)
}

/// Extract `key = "value"` from a comma-separated attribute string.
fn extract_attr_value(attrs: &str, key: &str) -> Option<String> {
    let search = key;
    for part in attrs.split(',') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix(search) {
            let rest = rest.trim().strip_prefix('=')?;
            let rest = rest.trim();
            // Strip surrounding quotes
            if rest.starts_with('"') && rest.ends_with('"') && rest.len() >= 2 {
                return Some(rest[1..rest.len() - 1].to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_attr_basic() {
        let attrs = r#"name = "hello", description = "Says hello""#;
        assert_eq!(extract_attr_value(attrs, "name"), Some("hello".into()));
        assert_eq!(
            extract_attr_value(attrs, "description"),
            Some("Says hello".into())
        );
    }

    #[test]
    fn extract_attr_missing() {
        let attrs = r#"name = "hello""#;
        assert_eq!(extract_attr_value(attrs, "missing"), None);
    }

    #[test]
    fn extract_attr_empty() {
        assert_eq!(extract_attr_value("", "name"), None);
    }

    #[test]
    fn extract_attr_no_quotes() {
        let attrs = r#"name = hello"#;
        assert_eq!(extract_attr_value(attrs, "name"), None);
    }

    #[test]
    fn extract_attr_whitespace() {
        let attrs = r#"  name  =  "test"  , description = "desc"  "#;
        assert_eq!(extract_attr_value(attrs, "name"), Some("test".into()));
        assert_eq!(
            extract_attr_value(attrs, "description"),
            Some("desc".into())
        );
    }
}
