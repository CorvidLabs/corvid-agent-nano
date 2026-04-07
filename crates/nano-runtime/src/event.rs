//! Events that flow through the runtime and are dispatched to plugins.

use chrono::{DateTime, Utc};
use nano_transport::Message;
use serde::{Deserialize, Serialize};

/// An event in the runtime event loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    /// A new message was received from a transport.
    MessageReceived(Message),
    /// An outgoing message was confirmed sent.
    MessageSent { to: String, tx_id: String },
    /// A contact was added.
    ContactAdded { address: String, name: String },
    /// A contact was removed.
    ContactRemoved { address: String },
    /// A plugin was loaded.
    PluginLoaded { name: String },
    /// A plugin was unloaded.
    PluginUnloaded { name: String },
    /// Scheduled tick (for polling plugins).
    Timer { timestamp: DateTime<Utc> },
    /// Graceful shutdown starting.
    Shutdown,
    /// Plugin-defined custom event.
    Custom {
        kind: String,
        data: serde_json::Value,
    },
}

/// Which kinds of events a plugin subscribes to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventKind {
    MessageReceived,
    MessageSent,
    ContactAdded,
    ContactRemoved,
    PluginLoaded,
    PluginUnloaded,
    Timer,
    Shutdown,
    Custom(String),
    /// Subscribe to all events.
    All,
}

impl Event {
    /// Returns the [`EventKind`] for this event.
    pub fn kind(&self) -> EventKind {
        match self {
            Event::MessageReceived(_) => EventKind::MessageReceived,
            Event::MessageSent { .. } => EventKind::MessageSent,
            Event::ContactAdded { .. } => EventKind::ContactAdded,
            Event::ContactRemoved { .. } => EventKind::ContactRemoved,
            Event::PluginLoaded { .. } => EventKind::PluginLoaded,
            Event::PluginUnloaded { .. } => EventKind::PluginUnloaded,
            Event::Timer { .. } => EventKind::Timer,
            Event::Shutdown => EventKind::Shutdown,
            Event::Custom { kind, .. } => EventKind::Custom(kind.clone()),
        }
    }
}
