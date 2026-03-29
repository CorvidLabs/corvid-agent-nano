//! Plugin tool invocation — creates WASM stores with host functions and
//! executes plugin tool calls via the `__corvid_invoke` export.
//!
//! Also handles event dispatch to plugins via `__corvid_on_event`.

use std::sync::Arc;

use anyhow::{Context, Result};
use corvid_plugin_sdk::Capability;
use wasmtime::{AsContextMut, Engine, Instance, Linker, Module, Store};

use crate::host_functions;
use crate::host_functions::algo::AlgoBackend;
use crate::host_functions::messaging::MessagingBackend;
use crate::host_functions::storage::StorageBackend;
use crate::loader::PluginState;
use crate::sandbox::{MemoryLimiter, SandboxLimits};

/// Shared backends passed into each plugin invocation.
pub struct InvokeContext {
    pub storage: Arc<StorageBackend>,
    pub algo: Option<Arc<AlgoBackend>>,
    pub messaging: Option<Arc<MessagingBackend>>,
}

/// Create a WASM Store with all host functions linked for the given plugin.
fn create_execution_store(
    engine: &Engine,
    module: &Module,
    plugin_id: &str,
    capabilities: &[Capability],
    limits: &SandboxLimits,
    ctx: &InvokeContext,
) -> Result<(Store<PluginState>, Instance)> {
    // Extract capability-specific config
    let http_allowlist = capabilities
        .iter()
        .filter_map(|c| match c {
            Capability::Network { allowlist } => Some(allowlist.clone()),
            _ => None,
        })
        .next()
        .unwrap_or_default();

    let message_target_filter = capabilities
        .iter()
        .filter_map(|c| match c {
            Capability::AgentMessage { target_filter } => Some(target_filter.clone()),
            _ => None,
        })
        .next();

    let state = PluginState {
        limiter: MemoryLimiter::new(limits.memory_bytes),
        plugin_id: plugin_id.to_string(),
        storage: Some(Arc::clone(&ctx.storage)),
        http_allowlist,
        algo: ctx.algo.as_ref().map(Arc::clone),
        messaging: ctx.messaging.as_ref().map(Arc::clone),
        message_target_filter,
    };

    let mut store = Store::new(engine, state);
    store.limiter(|s| &mut s.limiter);
    store.set_fuel(limits.fuel_per_call)?;

    let mut linker = Linker::new(engine);
    host_functions::link_host_functions(&mut linker, capabilities, limits)?;

    let instance = linker
        .instantiate(&mut store, module)
        .context("failed to instantiate plugin for invocation")?;

    Ok((store, instance))
}

/// Write data into WASM linear memory via `__corvid_alloc`.
/// Returns the WASM pointer to a length-prefixed buffer `[4-byte LE len][data]`.
/// Returns 0 on failure.
fn write_to_wasm(store: &mut Store<PluginState>, instance: &Instance, data: &[u8]) -> Result<i32> {
    let total_len = 4 + data.len();

    let alloc_fn = instance
        .get_typed_func::<i32, i32>(store.as_context_mut(), "__corvid_alloc")
        .context("missing export: __corvid_alloc")?;

    let ptr = alloc_fn
        .call(&mut *store, total_len as i32)
        .context("__corvid_alloc call failed")?;

    if ptr == 0 {
        anyhow::bail!("__corvid_alloc returned null");
    }

    let memory = instance
        .get_memory(&mut *store, "memory")
        .context("no memory export")?;

    let mem_data = memory.data_mut(&mut *store);
    let start = ptr as usize;
    if start + total_len > mem_data.len() {
        anyhow::bail!("allocated pointer out of bounds");
    }

    mem_data[start..start + 4].copy_from_slice(&(data.len() as u32).to_le_bytes());
    mem_data[start + 4..start + total_len].copy_from_slice(data);

    Ok(ptr)
}

/// Read a length-prefixed response from WASM memory at the given pointer.
fn read_response(store: &mut Store<PluginState>, instance: &Instance, ptr: i32) -> Result<Vec<u8>> {
    let memory = instance
        .get_memory(&mut *store, "memory")
        .context("no memory export")?;

    let data = memory.data(&*store);
    let offset = ptr as usize;

    if offset + 4 > data.len() {
        anyhow::bail!("result pointer out of bounds");
    }

    let len = u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]) as usize;

    if offset + 4 + len > data.len() {
        anyhow::bail!("result data out of bounds (len={len})");
    }

    Ok(data[offset + 4..offset + 4 + len].to_vec())
}

/// Invoke a plugin tool via `__corvid_invoke(tool_ptr, tool_len, input_ptr, input_len) -> ptr`.
///
/// The input is a msgpack-serialized JSON value. The response is a
/// length-prefixed msgpack buffer written by the plugin to WASM memory.
#[allow(clippy::too_many_arguments)]
pub fn invoke_tool(
    engine: &Engine,
    module: &Module,
    plugin_id: &str,
    capabilities: &[Capability],
    limits: &SandboxLimits,
    ctx: &InvokeContext,
    tool_name: &str,
    input: &serde_json::Value,
) -> Result<serde_json::Value> {
    let (mut store, instance) =
        create_execution_store(engine, module, plugin_id, capabilities, limits, ctx)?;

    // Serialize input as msgpack
    let input_bytes = rmp_serde::to_vec(input).context("failed to serialize tool input")?;

    // Get the __corvid_invoke export
    let invoke_fn = instance
        .get_typed_func::<(i32, i32, i32, i32), i32>(&mut store, "__corvid_invoke")
        .context("missing export: __corvid_invoke")?;

    // Write tool name to WASM memory
    let tool_buf = write_to_wasm(&mut store, &instance, tool_name.as_bytes())?;

    // Write input to WASM memory
    let input_buf = write_to_wasm(&mut store, &instance, &input_bytes)?;

    // Call __corvid_invoke — it returns a pointer to the response
    let result_ptr = invoke_fn
        .call(
            &mut store,
            (
                tool_buf + 4,
                tool_name.len() as i32,
                input_buf + 4,
                input_bytes.len() as i32,
            ),
        )
        .context("__corvid_invoke call failed (fuel exhaustion or trap)")?;

    if result_ptr == 0 {
        anyhow::bail!("plugin returned null from __corvid_invoke");
    }

    let result_bytes = read_response(&mut store, &instance, result_ptr)?;
    let result: serde_json::Value =
        rmp_serde::from_slice(&result_bytes).context("failed to deserialize tool result")?;

    Ok(result)
}

/// Dispatch an event to a plugin via `__corvid_on_event(event_ptr, event_len) -> i32`.
///
/// The event is msgpack-serialized. Returns Ok(status) where status is
/// the plugin's return code (0 = handled, non-zero = error/ignored).
pub fn dispatch_event_to_plugin(
    engine: &Engine,
    module: &Module,
    plugin_id: &str,
    capabilities: &[Capability],
    limits: &SandboxLimits,
    ctx: &InvokeContext,
    event: &corvid_plugin_sdk::error::PluginEvent,
) -> Result<i32> {
    let (mut store, instance) =
        create_execution_store(engine, module, plugin_id, capabilities, limits, ctx)?;

    // Check if the plugin exports __corvid_on_event
    let event_fn = match instance.get_typed_func::<(i32, i32), i32>(&mut store, "__corvid_on_event")
    {
        Ok(f) => f,
        Err(_) => {
            tracing::debug!(
                plugin_id = %plugin_id,
                "plugin does not export __corvid_on_event"
            );
            return Ok(-1);
        }
    };

    // Serialize event as msgpack
    let event_bytes = rmp_serde::to_vec(event).context("failed to serialize event")?;

    // Write event to WASM memory
    let event_buf = write_to_wasm(&mut store, &instance, &event_bytes)?;

    // Call __corvid_on_event
    let status = event_fn
        .call(&mut store, (event_buf + 4, event_bytes.len() as i32))
        .context("__corvid_on_event call failed")?;

    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invoke_context_creation() {
        let ctx = InvokeContext {
            storage: Arc::new(StorageBackend::new()),
            algo: None,
            messaging: None,
        };
        assert!(ctx.algo.is_none());
        assert!(ctx.messaging.is_none());
    }
}
