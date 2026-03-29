//! Host function: Algorand chain read access.
//!
//! Provides `host_algo_state(app_id, key_ptr, key_len)` which queries
//! Algorand application state via a pluggable backend. The response is
//! a msgpack-serialized JSON value written back to WASM memory.

use std::sync::Arc;

use wasmtime::Linker;

use crate::loader::PluginState;
use crate::wasm_mem;

/// Backend for querying Algorand application state.
///
/// In production, this calls the algod/indexer REST API.
/// In tests, it can be replaced with a mock.
pub struct AlgoBackend {
    inner: Box<dyn AlgoQuery + Send + Sync>,
}

/// Trait for querying Algorand app state — allows mocking in tests.
pub trait AlgoQuery: Send + Sync {
    /// Look up a key in an application's global state.
    /// Returns the value as a JSON value, or None if not found.
    fn app_state(&self, app_id: u64, key: &str) -> Result<Option<serde_json::Value>, String>;
}

impl AlgoBackend {
    pub fn new(query: impl AlgoQuery + 'static) -> Self {
        Self {
            inner: Box::new(query),
        }
    }

    pub fn app_state(&self, app_id: u64, key: &str) -> Result<Option<serde_json::Value>, String> {
        self.inner.app_state(app_id, key)
    }
}

/// Msgpack response for successful algo state queries.
#[derive(serde::Serialize)]
struct AlgoStateResponse {
    found: bool,
    value: serde_json::Value,
}

/// Msgpack response for algo state errors.
#[derive(serde::Serialize)]
struct AlgoStateError {
    error: String,
}

/// Link Algorand host functions into the WASM linker.
pub fn link(linker: &mut Linker<PluginState>) -> anyhow::Result<()> {
    // host_algo_state(app_id: i64, key_ptr: i32, key_len: i32) -> ptr to msgpack response
    linker.func_wrap(
        "env",
        "host_algo_state",
        |mut caller: wasmtime::Caller<'_, PluginState>,
         app_id: i64,
         key_ptr: i32,
         key_len: i32|
         -> i32 {
            // Read key string from WASM memory
            let key = match wasm_mem::read_str(&mut caller, key_ptr, key_len) {
                Some(k) => k,
                None => {
                    tracing::warn!("host_algo_state: failed to read key from WASM memory");
                    return 0;
                }
            };

            let algo = match &caller.data().algo {
                Some(a) => Arc::clone(a),
                None => {
                    tracing::error!("host_algo_state: algo backend not initialized");
                    let err = rmp_serde::to_vec(&AlgoStateError {
                        error: "algo backend not available".into(),
                    })
                    .unwrap_or_default();
                    return wasm_mem::write_response(&mut caller, &err);
                }
            };

            let app_id = app_id as u64;

            match algo.app_state(app_id, &key) {
                Ok(Some(value)) => {
                    let resp = rmp_serde::to_vec(&AlgoStateResponse { found: true, value })
                        .unwrap_or_default();
                    wasm_mem::write_response(&mut caller, &resp)
                }
                Ok(None) => {
                    let resp = rmp_serde::to_vec(&AlgoStateResponse {
                        found: false,
                        value: serde_json::Value::Null,
                    })
                    .unwrap_or_default();
                    wasm_mem::write_response(&mut caller, &resp)
                }
                Err(e) => {
                    tracing::warn!(app_id, key = %key, error = %e, "host_algo_state: query failed");
                    let err = rmp_serde::to_vec(&AlgoStateError { error: e }).unwrap_or_default();
                    wasm_mem::write_response(&mut caller, &err)
                }
            }
        },
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockAlgoQuery {
        data: std::collections::HashMap<(u64, String), serde_json::Value>,
    }

    impl MockAlgoQuery {
        fn new() -> Self {
            Self {
                data: std::collections::HashMap::new(),
            }
        }

        fn with_state(mut self, app_id: u64, key: &str, value: serde_json::Value) -> Self {
            self.data.insert((app_id, key.to_string()), value);
            self
        }
    }

    impl AlgoQuery for MockAlgoQuery {
        fn app_state(&self, app_id: u64, key: &str) -> Result<Option<serde_json::Value>, String> {
            Ok(self.data.get(&(app_id, key.to_string())).cloned())
        }
    }

    #[test]
    fn algo_link_compiles() {
        let engine = wasmtime::Engine::default();
        let mut linker = wasmtime::Linker::new(&engine);
        assert!(link(&mut linker).is_ok());
    }

    #[test]
    fn algo_backend_found() {
        let backend = AlgoBackend::new(MockAlgoQuery::new().with_state(
            123,
            "counter",
            serde_json::json!(42),
        ));
        let result = backend.app_state(123, "counter").unwrap();
        assert_eq!(result, Some(serde_json::json!(42)));
    }

    #[test]
    fn algo_backend_not_found() {
        let backend = AlgoBackend::new(MockAlgoQuery::new());
        let result = backend.app_state(999, "missing").unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn algo_backend_error() {
        struct FailingQuery;
        impl AlgoQuery for FailingQuery {
            fn app_state(
                &self,
                _app_id: u64,
                _key: &str,
            ) -> Result<Option<serde_json::Value>, String> {
                Err("network timeout".into())
            }
        }

        let backend = AlgoBackend::new(FailingQuery);
        let err = backend.app_state(1, "key").unwrap_err();
        assert!(err.contains("network timeout"));
    }
}
