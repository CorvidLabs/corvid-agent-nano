//! Actions that plugins return to request the runtime to do something.
//!
//! Plugins never directly mutate state — they return actions and the runtime
//! applies them. This keeps everything safe and auditable.

use serde::{Deserialize, Serialize};

/// An action a plugin requests the runtime to perform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Action {
    /// Send a message through the transport.
    SendMessage {
        to: String,
        content: String,
    },
    /// Persist a key-value pair in the plugin's scoped state.
    StoreState {
        key: String,
        value: serde_json::Value,
    },
    /// Emit a custom event into the event bus.
    EmitEvent {
        kind: String,
        data: serde_json::Value,
    },
    /// Structured log entry.
    Log {
        level: LogLevel,
        message: String,
    },
}

/// Log levels for plugin log actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}
