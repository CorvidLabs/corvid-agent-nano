//! Proc macros for corvid-agent plugin authoring.
//!
//! Provides `#[corvid_plugin]` which generates WASM export boilerplate:
//! - `__corvid_abi_version` — returns ABI version constant
//! - `__corvid_manifest` — returns msgpack-serialized manifest
//! - `__corvid_init` — deserializes InitContext, calls `init()`
//! - `__corvid_tool_call` — routes tool invocations
//! - `__corvid_handle_event` — deserializes event, calls `on_event()`
//! - `__corvid_shutdown` — calls `shutdown()`
//! - `__corvid_alloc` / `__corvid_dealloc` — WASM memory management
//!
//! Also provides `#[corvid_tool]` for generating `PluginTool` impls.

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemStruct};

/// Generates WASM export glue for a struct implementing `CorvidPlugin`.
///
/// # Usage
/// ```ignore
/// use corvid_plugin_sdk::prelude::*;
/// use corvid_plugin_macros::corvid_plugin;
///
/// #[corvid_plugin]
/// struct MyPlugin { /* ... */ }
///
/// impl CorvidPlugin for MyPlugin {
///     fn manifest() -> PluginManifest { /* ... */ }
///     fn tools(&self) -> &[Box<dyn PluginTool>] { /* ... */ }
///     fn init(&mut self, ctx: InitContext) -> Result<(), PluginError> { /* ... */ }
/// }
/// ```
///
/// This generates all necessary `extern "C"` functions for the WASM host
/// to load and interact with the plugin, plus memory management helpers.
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
            use std::sync::Mutex;
            use corvid_plugin_sdk::{CorvidPlugin, PluginManifest, ABI_VERSION};

            static INSTANCE: Mutex<Option<#struct_name>> = Mutex::new(None);

            /// Helper: write bytes to WASM linear memory, return ptr|len packed as i64.
            fn write_response(data: &[u8]) -> i64 {
                let len = data.len() as i32;
                let ptr = __corvid_alloc(len);
                unsafe {
                    let dest = ptr as *mut u8;
                    std::ptr::copy_nonoverlapping(data.as_ptr(), dest, data.len());
                }
                ((ptr as i64) << 32) | (len as i64 & 0xFFFF_FFFF)
            }

            /// Helper: read bytes from WASM linear memory.
            unsafe fn read_input(ptr: i32, len: i32) -> Vec<u8> {
                let slice = std::slice::from_raw_parts(ptr as *const u8, len as usize);
                slice.to_vec()
            }

            /// Returns the ABI version this plugin was compiled against.
            #[unsafe(no_mangle)]
            pub extern "C" fn __corvid_abi_version() -> u32 {
                ABI_VERSION
            }

            /// Returns msgpack-serialized PluginManifest. Result is ptr|len packed as i64.
            #[unsafe(no_mangle)]
            pub extern "C" fn __corvid_manifest() -> i64 {
                let manifest = <#struct_name as CorvidPlugin>::manifest();
                let data = rmp_serde::to_vec(&manifest).expect("manifest serialization failed");
                write_response(&data)
            }

            /// Instantiate the plugin and call init(). Input is msgpack InitContext stub.
            /// Returns 0 on success, negative on error.
            #[unsafe(no_mangle)]
            pub extern "C" fn __corvid_init(ptr: i32, len: i32) -> i64 {
                let data = unsafe { read_input(ptr, len) };

                // Deserialize the lightweight init payload (agent_id, host_version).
                // Service handles are injected by the host via separate function calls.
                #[derive(serde::Deserialize)]
                struct InitPayload {
                    agent_id: String,
                    host_version: String,
                }

                let payload: InitPayload = match rmp_serde::from_slice(&data) {
                    Ok(p) => p,
                    Err(e) => {
                        let msg = format!("init payload deserialize error: {e}");
                        let data = rmp_serde::to_vec(&msg).unwrap_or_default();
                        return -write_response(&data);
                    }
                };

                let ctx = corvid_plugin_sdk::InitContext {
                    agent_id: payload.agent_id,
                    host_version: payload.host_version,
                    storage: None,
                    http: None,
                    db: None,
                    fs: None,
                    algo: None,
                    messaging: None,
                };

                let mut plugin = <#struct_name as Default>::default();
                match plugin.init(ctx) {
                    Ok(()) => {
                        let mut lock = INSTANCE.lock().expect("plugin mutex poisoned");
                        *lock = Some(plugin);
                        0
                    }
                    Err(e) => {
                        let msg = format!("{e}");
                        let data = rmp_serde::to_vec(&msg).unwrap_or_default();
                        -write_response(&data)
                    }
                }
            }

            /// Route a tool call. Input: msgpack `{ "tool": "name", "input": {...} }`.
            /// Returns msgpack result string or error.
            #[unsafe(no_mangle)]
            pub extern "C" fn __corvid_tool_call(ptr: i32, len: i32) -> i64 {
                let data = unsafe { read_input(ptr, len) };

                #[derive(serde::Deserialize)]
                struct ToolCallPayload {
                    tool: String,
                    input: serde_json::Value,
                    session_id: String,
                }

                let payload: ToolCallPayload = match rmp_serde::from_slice(&data) {
                    Ok(p) => p,
                    Err(e) => {
                        let msg = format!("tool call deserialize error: {e}");
                        let resp = rmp_serde::to_vec(&Err::<String, String>(msg)).unwrap_or_default();
                        return write_response(&resp);
                    }
                };

                let lock = INSTANCE.lock().expect("plugin mutex poisoned");
                let plugin = match lock.as_ref() {
                    Some(p) => p,
                    None => {
                        let resp = rmp_serde::to_vec(&Err::<String, String>(
                            "plugin not initialized".into(),
                        ))
                        .unwrap_or_default();
                        return write_response(&resp);
                    }
                };

                let manifest = <#struct_name as CorvidPlugin>::manifest();
                let ctx = corvid_plugin_sdk::ToolContext {
                    agent_id: String::new(),
                    session_id: payload.session_id,
                    capabilities: manifest.capabilities.clone(),
                };

                let result = plugin
                    .tools()
                    .iter()
                    .find(|t| t.name() == payload.tool)
                    .map(|t| t.execute(payload.input, &ctx))
                    .unwrap_or(Err(corvid_plugin_sdk::PluginError::BadInput(
                        format!("unknown tool: {}", payload.tool),
                    )));

                let resp = match result {
                    Ok(s) => rmp_serde::to_vec(&Ok::<String, String>(s)),
                    Err(e) => rmp_serde::to_vec(&Err::<String, String>(format!("{e}"))),
                };
                write_response(&resp.unwrap_or_default())
            }

            /// Handle an event. Input: msgpack PluginEvent.
            #[unsafe(no_mangle)]
            pub extern "C" fn __corvid_handle_event(ptr: i32, len: i32) -> i64 {
                let data = unsafe { read_input(ptr, len) };

                let event: corvid_plugin_sdk::PluginEvent = match rmp_serde::from_slice(&data) {
                    Ok(e) => e,
                    Err(e) => {
                        let msg = format!("event deserialize error: {e}");
                        let resp = rmp_serde::to_vec(&msg).unwrap_or_default();
                        return -write_response(&resp);
                    }
                };

                let mut lock = INSTANCE.lock().expect("plugin mutex poisoned");
                let plugin = match lock.as_mut() {
                    Some(p) => p,
                    None => return -1,
                };

                let manifest = <#struct_name as CorvidPlugin>::manifest();
                let ctx = corvid_plugin_sdk::ToolContext {
                    agent_id: String::new(),
                    session_id: String::new(),
                    capabilities: manifest.capabilities.clone(),
                };

                match plugin.on_event(event, &ctx) {
                    Ok(()) => 0,
                    Err(e) => {
                        let msg = format!("{e}");
                        let resp = rmp_serde::to_vec(&msg).unwrap_or_default();
                        -write_response(&resp)
                    }
                }
            }

            /// Clean shutdown.
            #[unsafe(no_mangle)]
            pub extern "C" fn __corvid_shutdown() {
                let mut lock = INSTANCE.lock().expect("plugin mutex poisoned");
                if let Some(plugin) = lock.as_mut() {
                    plugin.shutdown();
                }
                *lock = None;
            }

            /// WASM memory allocator — host calls this to reserve space for inputs.
            #[unsafe(no_mangle)]
            pub extern "C" fn __corvid_alloc(size: i32) -> i32 {
                let layout = std::alloc::Layout::from_size_align(size as usize, 1)
                    .expect("invalid alloc layout");
                unsafe { std::alloc::alloc(layout) as i32 }
            }

            /// WASM memory deallocator — host calls this to free response buffers.
            #[unsafe(no_mangle)]
            pub extern "C" fn __corvid_dealloc(ptr: i32, size: i32) {
                let layout = std::alloc::Layout::from_size_align(size as usize, 1)
                    .expect("invalid dealloc layout");
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
