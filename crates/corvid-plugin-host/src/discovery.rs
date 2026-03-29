//! Plugin discovery — list manifests and tools for the TypeScript bridge.

use serde::{Deserialize, Serialize};

use corvid_plugin_sdk::PluginManifest;

use crate::registry::PluginRegistry;

/// Tool info returned to the TypeScript bridge for auto-registration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInfo {
    /// Tool name (unique within plugin).
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON Schema v7 for the tool's input.
    pub input_schema: serde_json::Value,
}

/// Response for `plugin.list` JSON-RPC method.
#[derive(Debug, Serialize, Deserialize)]
pub struct ListResponse {
    pub plugins: Vec<PluginManifest>,
}

/// Response for `plugin.tools` JSON-RPC method.
#[derive(Debug, Serialize, Deserialize)]
pub struct ToolsResponse {
    pub tools: Vec<PluginToolEntry>,
}

/// A tool entry with its owning plugin ID.
#[derive(Debug, Serialize, Deserialize)]
pub struct PluginToolEntry {
    pub plugin_id: String,
    pub tool: ToolInfo,
}

/// List all loaded plugin manifests.
pub async fn list_plugins(registry: &PluginRegistry) -> ListResponse {
    ListResponse {
        plugins: registry.list_manifests().await,
    }
}

/// List tools for all plugins or filtered by plugin ID.
///
/// Tool discovery is currently based on manifest data. Full WASM-based
/// tool schema extraction will be added when per-plugin Store instances
/// are fully integrated.
pub async fn list_tools(registry: &PluginRegistry, plugin_id: Option<&str>) -> ToolsResponse {
    let manifests = registry.list_manifests().await;
    let tools = Vec::new();

    for manifest in manifests {
        if let Some(id) = plugin_id {
            if manifest.id != id {
                continue;
            }
        }

        // Tool schemas will be extracted from WASM instances in the full
        // integration. For now, we report plugins are loaded but tools
        // require the WASM instance to enumerate.
        let _ = manifest;

        // Placeholder: when full WASM integration is done, we'll call
        // __corvid_tool_schemas on each plugin's instance to get ToolInfo.
    }

    ToolsResponse { tools }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_info_serialization() {
        let tool = ToolInfo {
            name: "set_threshold".into(),
            description: "Set the oracle threshold".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "value": { "type": "number" }
                }
            }),
        };

        let json = serde_json::to_string(&tool).unwrap();
        assert!(json.contains("set_threshold"));
        assert!(json.contains("oracle threshold"));

        let roundtrip: ToolInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip.name, "set_threshold");
    }

    #[test]
    fn plugin_tool_entry_serialization() {
        let entry = PluginToolEntry {
            plugin_id: "algo-oracle".into(),
            tool: ToolInfo {
                name: "fetch_state".into(),
                description: "Fetch app state".into(),
                input_schema: serde_json::json!({}),
            },
        };

        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("algo-oracle"));
        assert!(json.contains("fetch_state"));
    }
}
