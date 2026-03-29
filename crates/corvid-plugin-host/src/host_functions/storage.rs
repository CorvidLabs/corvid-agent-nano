//! Host function: scoped key-value storage.
//!
//! Each plugin gets its own namespace — no cross-plugin reads by construction.
//! Storage is backed by a simple in-memory HashMap for now; persistent backends
//! (SQLite, filesystem) will be added based on deployment needs.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use wasmtime::Linker;

use crate::loader::PluginState;
use crate::wasm_mem;

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
    // host_kv_get(key_ptr, key_len) -> ptr to length-prefixed response (0 = not found)
    linker.func_wrap(
        "env",
        "host_kv_get",
        |mut caller: wasmtime::Caller<'_, PluginState>, key_ptr: i32, key_len: i32| -> i32 {
            // Read key string from WASM memory
            let key = match wasm_mem::read_str(&mut caller, key_ptr, key_len) {
                Some(k) => k,
                None => {
                    tracing::warn!("host_kv_get: failed to read key from WASM memory");
                    return 0;
                }
            };

            let plugin_id = caller.data().plugin_id.clone();
            let storage = match &caller.data().storage {
                Some(s) => Arc::clone(s),
                None => {
                    tracing::error!("host_kv_get: storage backend not initialized");
                    return 0;
                }
            };

            // Look up value in namespace-scoped storage
            let value = match storage.get(&plugin_id, &key) {
                Some(v) => v,
                None => return 0, // null ptr = not found
            };

            // Write response back to WASM memory via __corvid_alloc
            wasm_mem::write_response(&mut caller, &value)
        },
    )?;

    // host_kv_set(key_ptr, key_len, val_ptr, val_len) -> status (0 = ok, -1 = error)
    linker.func_wrap(
        "env",
        "host_kv_set",
        |mut caller: wasmtime::Caller<'_, PluginState>,
         key_ptr: i32,
         key_len: i32,
         val_ptr: i32,
         val_len: i32|
         -> i32 {
            // Read key string from WASM memory
            let key = match wasm_mem::read_str(&mut caller, key_ptr, key_len) {
                Some(k) => k,
                None => {
                    tracing::warn!("host_kv_set: failed to read key from WASM memory");
                    return -1;
                }
            };

            // Read value bytes from WASM memory
            let value = match wasm_mem::read_bytes(&mut caller, val_ptr, val_len) {
                Some(v) => v,
                None => {
                    tracing::warn!("host_kv_set: failed to read value from WASM memory");
                    return -1;
                }
            };

            let plugin_id = caller.data().plugin_id.clone();
            let storage = match &caller.data().storage {
                Some(s) => Arc::clone(s),
                None => {
                    tracing::error!("host_kv_set: storage backend not initialized");
                    return -1;
                }
            };

            if storage.set(&plugin_id, &key, value) {
                0 // success
            } else {
                -1 // storage lock poisoned
            }
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

    #[test]
    fn storage_empty_key() {
        let backend = StorageBackend::new();
        backend.set("ns", "", b"val".to_vec());
        assert_eq!(backend.get("ns", ""), Some(b"val".to_vec()));
    }

    #[test]
    fn storage_binary_values() {
        let backend = StorageBackend::new();
        let binary = vec![0u8, 1, 2, 255, 254, 253];
        backend.set("ns", "bin", binary.clone());
        assert_eq!(backend.get("ns", "bin"), Some(binary));
    }
}
