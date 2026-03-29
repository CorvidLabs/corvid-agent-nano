//! Host function: Algorand chain read access.

use wasmtime::Linker;

use crate::loader::PluginState;

/// Link Algorand host functions into the WASM linker.
pub fn link(linker: &mut Linker<PluginState>) -> anyhow::Result<()> {
    // host_algo_state(app_id: i64, key_ptr, key_len) -> ptr to msgpack response
    linker.func_wrap(
        "env",
        "host_algo_state",
        |_caller: wasmtime::Caller<'_, PluginState>,
         _app_id: i64,
         _key_ptr: i32,
         _key_len: i32|
         -> i32 {
            // Full implementation will:
            // 1. Read key from WASM memory
            // 2. Query Algorand indexer/algod for app state
            // 3. Serialize result as msgpack
            // 4. Write to WASM memory and return pointer
            0 // placeholder
        },
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn algo_link_compiles() {
        let engine = wasmtime::Engine::default();
        let mut linker = wasmtime::Linker::new(&engine);
        assert!(super::link(&mut linker).is_ok());
    }
}
