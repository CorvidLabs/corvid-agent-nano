//! Event-driven plugin runtime for corvid-agent-nano.
//!
//! The runtime coordinates:
//! - **Transport**: polling for inbound messages, sending outbound ones
//! - **Plugins**: receiving events, returning actions
//! - **State**: scoped key-value storage per plugin
//! - **Event bus**: internal event routing between plugins

pub mod action;
pub mod event;
pub mod plugin;
pub mod plugins;
pub mod runtime;
pub mod state;

pub use action::{Action, LogLevel};
pub use event::{Event, EventKind};
pub use plugin::{Plugin, PluginContext};
pub use runtime::{Runtime, RuntimeConfig};
pub use state::StateStore;
