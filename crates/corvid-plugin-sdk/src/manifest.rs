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

/// Metadata for a single tool exposed by a plugin.
///
/// Declared in the manifest so the host can enumerate tools without
/// calling into the WASM instance at list time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInfo {
    /// Unique tool name within the plugin.
    pub name: String,

    /// Human-readable description shown in the tool registry.
    pub description: String,

    /// JSON Schema v7 describing the expected input object.
    #[serde(default)]
    pub input_schema: serde_json::Value,
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

    /// Tools this plugin exposes. Used for auto-registration in the
    /// TypeScript tool registry without calling into the WASM instance.
    #[serde(default)]
    pub tools: Vec<ToolInfo>,

    /// Plugin IDs this plugin depends on. The host ensures dependencies
    /// are loaded and active before initializing this plugin.
    /// Each entry is a plugin ID (e.g. `"algo-oracle"`).
    #[serde(default)]
    pub dependencies: Vec<String>,
}
