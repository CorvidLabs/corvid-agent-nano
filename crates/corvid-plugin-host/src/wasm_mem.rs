//! WASM linear memory access helpers.
//!
//! Provides safe read/write operations across the WASM boundary.
//! All host functions use these helpers to exchange data with plugins.

use wasmtime::{AsContext, Caller};

use crate::loader::PluginState;

/// Read `len` bytes from WASM linear memory starting at `ptr`.
///
/// Returns `None` if memory export is missing or the range is out of bounds.
pub fn read_bytes(caller: &mut Caller<'_, PluginState>, ptr: i32, len: i32) -> Option<Vec<u8>> {
    let memory = caller.get_export("memory")?.into_memory()?;
    let data = memory.data(&*caller);
    let start = ptr as usize;
    let end = start.checked_add(len as usize)?;
    if end > data.len() {
        return None;
    }
    Some(data[start..end].to_vec())
}

/// Read a UTF-8 string from WASM linear memory.
pub fn read_str(caller: &mut Caller<'_, PluginState>, ptr: i32, len: i32) -> Option<String> {
    let bytes = read_bytes(caller, ptr, len)?;
    String::from_utf8(bytes).ok()
}

/// Allocate space in WASM linear memory via the plugin's `__corvid_alloc` export,
/// then write `data` into it. Returns the WASM pointer to a length-prefixed buffer:
/// `[4-byte LE length][data bytes]`.
///
/// Returns 0 if allocation or write fails.
pub fn write_response(caller: &mut Caller<'_, PluginState>, data: &[u8]) -> i32 {
    let total_len = 4 + data.len();

    // Get the allocator export
    let alloc_fn = match caller.get_export("__corvid_alloc") {
        Some(ext) => match ext.into_func() {
            Some(f) => f,
            None => return 0,
        },
        None => return 0,
    };

    let alloc = match alloc_fn.typed::<i32, i32>(caller.as_context()) {
        Ok(f) => f,
        Err(_) => return 0,
    };

    let ptr = match alloc.call(&mut *caller, total_len as i32) {
        Ok(p) => p,
        Err(_) => return 0,
    };

    if ptr == 0 {
        return 0;
    }

    // Write length prefix + data
    let memory = match caller.get_export("memory") {
        Some(ext) => match ext.into_memory() {
            Some(m) => m,
            None => return 0,
        },
        None => return 0,
    };

    let mem_data = memory.data_mut(&mut *caller);
    let start = ptr as usize;
    if start + total_len > mem_data.len() {
        return 0;
    }

    mem_data[start..start + 4].copy_from_slice(&(data.len() as u32).to_le_bytes());
    mem_data[start + 4..start + total_len].copy_from_slice(data);

    ptr
}
