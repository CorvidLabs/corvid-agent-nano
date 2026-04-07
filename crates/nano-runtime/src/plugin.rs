//! The Plugin trait — the core extension mechanism for nano.
//!
//! Built-in plugins (hub, auto-reply) and future user plugins all implement
//! this trait. Plugins subscribe to events and return actions.

use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;

use crate::action::Action;
use crate::event::{Event, EventKind};

/// Read-only context provided to plugins during event handling.
pub struct PluginContext {
    /// The agent's address on the primary transport.
    pub agent_address: String,
    /// The agent's display name.
    pub agent_name: String,
    /// Plugin-scoped state (read-only snapshot).
    pub state: HashMap<String, serde_json::Value>,
    /// Plugin-specific config from nano.toml `[plugins.<name>]`.
    pub config: toml::Table,
}

/// The native plugin trait.
///
/// Plugins are loaded at startup and receive events from the runtime.
/// They return zero or more [`Action`]s that the runtime executes.
#[async_trait]
pub trait Plugin: Send + Sync {
    /// Unique plugin name (e.g. "hub", "auto-reply").
    fn name(&self) -> &str;

    /// Plugin version.
    fn version(&self) -> &str;

    /// Called once when the plugin loads. Use this for one-time setup.
    async fn init(&mut self, ctx: &PluginContext) -> Result<()>;

    /// Handle an event and return zero or more actions.
    async fn handle_event(&self, event: &Event, ctx: &PluginContext) -> Result<Vec<Action>>;

    /// Which event kinds this plugin subscribes to.
    fn subscriptions(&self) -> Vec<EventKind>;

    /// Called on graceful shutdown. Default is a no-op.
    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}
