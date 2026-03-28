//! Host function: agent message bus.

use wasmtime::Linker;

use crate::loader::PluginState;

/// Link messaging host functions into the WASM linker.
pub fn link(linker: &mut Linker<PluginState>) -> anyhow::Result<()> {
    // host_send_message(target_ptr, target_len, msg_ptr, msg_len) -> status
    linker.func_wrap("env", "host_send_message", |_caller: wasmtime::Caller<'_, PluginState>, _target_ptr: i32, _target_len: i32, _msg_ptr: i32, _msg_len: i32| -> i32 {
        // Full implementation will:
        // 1. Read target and message from WASM memory
        // 2. Validate target against plugin's declared target_filter
        // 3. Dispatch message through the agent messaging system
        // 4. Return 0 for success, -1 for error
        0 // placeholder
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn messaging_link_compiles() {
        let engine = wasmtime::Engine::default();
        let mut linker = wasmtime::Linker::new(&engine);
        assert!(super::link(&mut linker).is_ok());
    }
}
