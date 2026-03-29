use crate::context::ToolContext;
use crate::error::PluginError;

/// A discrete callable unit within a plugin.
pub trait PluginTool: Send + Sync {
    /// Unique tool name within the plugin.
    fn name(&self) -> &str;

    /// Human-readable description.
    fn description(&self) -> &str;

    /// JSON Schema v7 describing the input format.
    fn input_schema(&self) -> serde_json::Value;

    /// Execute the tool synchronously. Host wraps in blocking thread pool.
    fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> Result<String, PluginError>;
}
