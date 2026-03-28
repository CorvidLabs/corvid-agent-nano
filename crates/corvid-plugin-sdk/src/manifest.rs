use serde::{Deserialize, Serialize};

use crate::capability::Capability;
use crate::error::EventKind;

/// Trust tier hint declared by the plugin. The host assigns the actual tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrustTier {
    Trusted,
    Verified,
    Untrusted,
}

/// Static metadata describing a plugin. Returned by [`CorvidPlugin::manifest()`].
///
/// The `id` field must match `^[a-z][a-z0-9-]{0,49}$`.
/// The `version` and `min_host_version` fields must be valid semver.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Plugin identifier. Regex: `^[a-z][a-z0-9-]{0,49}$`
    pub id: String,

    /// Semver version string.
    pub version: String,

    /// Author name or organization.
    pub author: String,

    /// Human-readable description.
    pub description: String,

    /// Required capabilities. Host rejects unknown capabilities.
    pub capabilities: Vec<Capability>,

    /// Events this plugin subscribes to.
    pub event_filter: Vec<EventKind>,

    /// Declared trust tier (hint — host assigns actual tier).
    pub trust_tier: TrustTier,

    /// Minimum compatible host version (semver).
    pub min_host_version: String,
}
