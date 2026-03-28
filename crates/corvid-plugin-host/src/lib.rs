//! Plugin Host — Wasmtime-based WASM plugin runtime for corvid-agent.
//!
//! Runs as a sidecar binary communicating over Unix domain socket.
//! Manages plugin loading, sandboxing, hot-reload, and host function dispatch.

pub mod discovery;
pub mod engine;
pub mod executor;
pub mod host_functions;
pub mod loader;
pub mod registry;
pub mod sandbox;

pub use engine::build_engine;
pub use loader::{LoadError, LoadedPlugin};
pub use registry::{CallGuard, PluginRegistry, PluginSlot};
pub use sandbox::SandboxLimits;
