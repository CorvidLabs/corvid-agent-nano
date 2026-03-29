use serde::{Deserialize, Serialize};
use std::fmt;

use crate::capability::Capability;

/// Errors that can occur during plugin lifecycle and tool execution.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PluginError {
    /// Plugin initialization failed.
    #[error("init failed: {0}")]
    Init(String),

    /// Tool execution failed.
    #[error("execution failed: {0}")]
    Exec(String),

    /// Required capability not granted.
    #[error("missing capability: {0}")]
    MissingCapability(Capability),

    /// Invalid input to tool.
    #[error("bad input: {0}")]
    BadInput(String),

    /// Execution exceeded wall-clock limit.
    #[error("execution timed out")]
    Timeout,

    /// Plugin is draining (hot-reload in progress).
    #[error("plugin unavailable (draining)")]
    Unavailable,
}

/// Events that plugins can subscribe to and handle.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum PluginEvent {
    /// Incoming agent message.
    AgentMessage {
        from: String,
        content: serde_json::Value,
    },

    /// Relevant on-chain transaction.
    AlgoTransaction { txid: String },

    /// Periodic timer tick.
    ScheduledTick { interval_ms: u64, counter: u64 },

    /// Incoming webhook request.
    HttpWebhook {
        path: String,
        body: serde_json::Value,
    },
}

/// Discriminant-only version of [`PluginEvent`] for subscription filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum EventKind {
    AgentMessage,
    AlgoTransaction,
    ScheduledTick,
    HttpWebhook,
}

impl fmt::Display for EventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AgentMessage => write!(f, "AgentMessage"),
            Self::AlgoTransaction => write!(f, "AlgoTransaction"),
            Self::ScheduledTick => write!(f, "ScheduledTick"),
            Self::HttpWebhook => write!(f, "HttpWebhook"),
        }
    }
}

impl PluginEvent {
    /// Returns the [`EventKind`] discriminant for this event.
    pub fn kind(&self) -> EventKind {
        match self {
            Self::AgentMessage { .. } => EventKind::AgentMessage,
            Self::AlgoTransaction { .. } => EventKind::AlgoTransaction,
            Self::ScheduledTick { .. } => EventKind::ScheduledTick,
            Self::HttpWebhook { .. } => EventKind::HttpWebhook,
        }
    }
}
