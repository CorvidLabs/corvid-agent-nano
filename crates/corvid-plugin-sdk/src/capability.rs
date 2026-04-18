use serde::{Deserialize, Serialize};
use std::fmt;

/// Capabilities a plugin can request. The host rejects unknown capabilities
/// with a hard load failure (never silently dropped).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Capability {
    /// Outbound HTTP to allowlisted domains.
    Network { allowlist: Vec<String> },

    /// Scoped key-value storage.
    Storage { namespace: String },

    /// Read-only Algorand chain access.
    AlgoRead,

    /// Read-only database access (SELECT only).
    DbRead,

    /// Read-only filesystem access within project directory.
    FsProjectDir,

    /// Send messages to matching agents.
    AgentMessage { target_filter: String },

    /// Access the host-managed LLM service (Claude, OpenAI, Ollama).
    ///
    /// The host configures the provider, model, and API key via env vars.
    /// Plugins call `host_llm_chat` with a list of messages and receive
    /// a text response. This keeps API keys out of WASM memory.
    LlmChat,
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network { allowlist } => write!(f, "Network({})", allowlist.join(", ")),
            Self::Storage { namespace } => write!(f, "Storage({namespace})"),
            Self::AlgoRead => write!(f, "AlgoRead"),
            Self::DbRead => write!(f, "DbRead"),
            Self::FsProjectDir => write!(f, "FsProjectDir"),
            Self::AgentMessage { target_filter } => {
                write!(f, "AgentMessage({target_filter})")
            }
            Self::LlmChat => write!(f, "LlmChat"),
        }
    }
}
