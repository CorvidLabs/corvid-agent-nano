use std::sync::Arc;

use crate::capability::Capability;
use crate::service::{
    AlgoReadService, DbReadService, FsReadService, HttpService, MessagingService, StorageService,
};

/// Context passed to [`CorvidPlugin::init()`].
///
/// Service handles are `None` for capabilities not granted to the plugin.
pub struct InitContext {
    /// Current agent's identity.
    pub agent_id: String,

    /// Plugin host version.
    pub host_version: String,

    /// Scoped key-value storage. Present only when `Storage` capability granted.
    pub storage: Option<Arc<dyn StorageService>>,

    /// Outbound HTTP. Present only when `Network` capability granted.
    pub http: Option<Arc<dyn HttpService>>,

    /// Read-only database access. Present only when `DbRead` capability granted.
    pub db: Option<Arc<dyn DbReadService>>,

    /// Sandboxed filesystem read. Present only when `FsProjectDir` capability granted.
    pub fs: Option<Arc<dyn FsReadService>>,

    /// Algorand chain read access. Present only when `AlgoRead` capability granted.
    pub algo: Option<Arc<dyn AlgoReadService>>,

    /// Agent message bus. Present only when `AgentMessage` capability granted.
    pub messaging: Option<Arc<dyn MessagingService>>,
}

/// Context passed to tool execution and event handling.
pub struct ToolContext {
    /// Current agent's identity.
    pub agent_id: String,

    /// Session identifier for this invocation.
    pub session_id: String,

    /// Capabilities granted to this plugin.
    pub capabilities: Vec<Capability>,
}
