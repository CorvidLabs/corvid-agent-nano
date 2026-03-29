//! Host function: scoped key-value storage.
//!
//! Each plugin gets its own namespace — no cross-plugin reads by construction.
//! Storage is backed by a simple in-memory HashMap for now; persistent backends
//! (SQLite, filesystem) will be added based on deployment needs.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use wasmtime::Linker;

use crate::loader::PluginState;

/// In-memory storage backend keyed by `{plugin_id}:{key}`.
///
/// Thread-safe via Arc<Mutex<>>. Production deployments should swap
/// this for SQLite or filesystem-backed storage.
pub struct StorageBackend {
    data: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl StorageBackend {
    pub fn new() -> Self {
        Self {
            data: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn get(&self, namespace: &str, key: &str) -> Option<Vec<u8>> {
        let full_key = format!("{namespace}:{key}");
        self.data.lock().ok()?.get(&full_key).cloned()
    }

    pub fn set(&self, namespace: &str, key: &str, value: Vec<u8>) -> bool {
        let full_key = format!("{namespace}:{key}");
        match self.data.lock() {
            Ok(mut map) => {
                map.insert(full_key, value);
                true
            }
            Err(_) => false,
        }
    }
}

impl Default for StorageBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// Link storage host functions into the WASM linker.
///
/// Provides `host_kv_get` and `host_kv_set` in the "env" namespace.
pub fn link(linker: &mut Linker<PluginState>) -> anyhow::Result<()> {
    // host_kv_get(key_ptr, key_len) -> ptr to msgpack response
    linker.func_wrap(
        "env",
        "host_kv_get",
        |_caller: wasmtime::Caller<'_, PluginState>, _key_ptr: i32, _key_len: i32| -> i32 {
            // Full implementation will:
            // 1. Read key bytes from WASM memory at (key_ptr, key_len)
            // 2. Look up in StorageBackend with plugin_id namespace
            // 3. Write msgpack response back to WASM memory
            // 4. Return pointer to response
            0 // null ptr = not found (placeholder)
        },
    )?;

    // host_kv_set(key_ptr, key_len, val_ptr, val_len) -> status
    linker.func_wrap(
        "env",
        "host_kv_set",
        |_caller: wasmtime::Caller<'_, PluginState>,
         _key_ptr: i32,
         _key_len: i32,
         _val_ptr: i32,
         _val_len: i32|
         -> i32 {
            // Full implementation will:
            // 1. Read key and value bytes from WASM memory
            // 2. Store in StorageBackend with plugin_id namespace
            // 3. Return 0 for success, -1 for error
            0 // success (placeholder)
        },
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_isolation() {
        let backend = StorageBackend::new();

        backend.set("plugin-a", "key1", b"value-a".to_vec());
        backend.set("plugin-b", "key1", b"value-b".to_vec());

        assert_eq!(backend.get("plugin-a", "key1"), Some(b"value-a".to_vec()));
        assert_eq!(backend.get("plugin-b", "key1"), Some(b"value-b".to_vec()));
        assert_eq!(backend.get("plugin-a", "key2"), None);
    }

    #[test]
    fn storage_overwrite() {
        let backend = StorageBackend::new();
        backend.set("ns", "k", b"v1".to_vec());
        backend.set("ns", "k", b"v2".to_vec());
        assert_eq!(backend.get("ns", "k"), Some(b"v2".to_vec()));
    }
}
