//! Host function implementations — capability-gated services provided to plugins.
//!
//! Each module implements host functions that are linked into the WASM instance
//! at instantiation time based on the plugin's declared capabilities.
//! Functions not linked are never callable — attempts cause a WASM trap.

pub mod algo;
pub mod http;
pub mod messaging;
pub mod storage;

use corvid_plugin_sdk::Capability;
use wasmtime::Linker;

use crate::loader::PluginState;
use crate::sandbox::SandboxLimits;

/// Link host functions into a WASM linker based on granted capabilities.
///
/// **Critical:** Only link functions for capabilities the plugin declared
/// and was granted. Never use `wasmtime_wasi::add_to_linker` for untrusted
/// plugins — it includes filesystem, clocks, random, and process access
/// that bypasses the capability model.
pub fn link_host_functions(
    linker: &mut Linker<PluginState>,
    capabilities: &[Capability],
    _limits: &SandboxLimits,
) -> anyhow::Result<()> {
    for cap in capabilities {
        match cap {
            Capability::Storage { .. } => {
                storage::link(linker)?;
            }
            Capability::Network { .. } => {
                http::link(linker)?;
            }
            Capability::AlgoRead => {
                algo::link(linker)?;
            }
            Capability::AgentMessage { .. } => {
                messaging::link(linker)?;
            }
            Capability::DbRead | Capability::FsProjectDir => {
                // DB and FS host functions are provided through different
                // mechanisms (direct store access). Linking stubs here for
                // future expansion.
            }
            _ => {
                // Unknown/future capabilities — no host functions to link
                tracing::debug!("no host function linkage for capability: {cap}");
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_empty_capabilities() {
        let engine = wasmtime::Engine::default();
        let mut linker = Linker::new(&engine);
        assert!(link_host_functions(&mut linker, &[], &SandboxLimits::for_tier(corvid_plugin_sdk::TrustTier::Untrusted)).is_ok());
    }
}
