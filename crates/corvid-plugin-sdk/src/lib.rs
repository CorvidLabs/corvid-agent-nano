//! Stable public contract for corvid-agent plugin authors.
//!
//! This crate defines the [`CorvidPlugin`] trait, [`PluginTool`] trait,
//! capability system, manifest format, error types, and host function
//! signatures. Plugin authors depend on this crate (plus optionally
//! `corvid-plugin-macros`) and nothing else.

pub mod capability;
pub mod context;
pub mod error;
pub mod host_api;
pub mod manifest;
pub mod service;
pub mod tool;

pub use capability::Capability;
pub use context::{InitContext, ToolContext};
pub use error::{EventKind, PluginError, PluginEvent};
pub use manifest::{PluginManifest, ToolInfo, TrustTier};
pub use service::{
    AlgoReadService, DbReadService, FsReadService, HttpService, MessagingService, StorageService,
};
pub use tool::PluginTool;

/// Current ABI version. Bumped on breaking trait/type layout changes.
pub const ABI_VERSION: u32 = 1;

/// Oldest ABI the host will accept. Maintains a 1-major window.
pub const ABI_MIN_COMPATIBLE: u32 = 1;

/// Core plugin interface. All plugins must implement this trait.
///
/// Requires `Send + Sync + 'static` for safe use across threads.
pub trait CorvidPlugin: Send + Sync + 'static {
    /// Static metadata — called at load time before instantiation.
    fn manifest() -> PluginManifest
    where
        Self: Sized;

    /// Tools this plugin exposes. Called after manifest validation.
    fn tools(&self) -> &[Box<dyn PluginTool>];

    /// Called once after instantiation with capability-gated context.
    fn init(&mut self, ctx: InitContext) -> Result<(), PluginError>;

    /// Handle events matching declared `event_filter`. Default no-op.
    fn on_event(&mut self, _event: PluginEvent, _ctx: &ToolContext) -> Result<(), PluginError> {
        Ok(())
    }

    /// Called before unload. Must not panic. Errors are logged and ignored.
    fn shutdown(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_version_consistency() {
        assert!(ABI_VERSION >= ABI_MIN_COMPATIBLE);
    }

    #[test]
    fn capability_display() {
        let cap = Capability::Network {
            allowlist: vec!["api.example.com".into()],
        };
        assert_eq!(cap.to_string(), "Network(api.example.com)");

        let cap = Capability::Storage {
            namespace: "my-plugin".into(),
        };
        assert_eq!(cap.to_string(), "Storage(my-plugin)");

        assert_eq!(Capability::AlgoRead.to_string(), "AlgoRead");
        assert_eq!(Capability::DbRead.to_string(), "DbRead");
        assert_eq!(Capability::FsProjectDir.to_string(), "FsProjectDir");
    }

    #[test]
    fn event_kind_discriminant() {
        let event = PluginEvent::AgentMessage {
            from: "alice".into(),
            content: serde_json::json!({"text": "hello"}),
        };
        assert_eq!(event.kind(), EventKind::AgentMessage);

        let event = PluginEvent::ScheduledTick {
            interval_ms: 5000,
            counter: 42,
        };
        assert_eq!(event.kind(), EventKind::ScheduledTick);
    }

    #[test]
    fn plugin_event_roundtrip() {
        let event = PluginEvent::HttpWebhook {
            path: "/hook".into(),
            body: serde_json::json!({"key": "val"}),
        };

        let packed = rmp_serde::to_vec(&event).unwrap();
        let unpacked: PluginEvent = rmp_serde::from_slice(&packed).unwrap();

        match unpacked {
            PluginEvent::HttpWebhook { path, body } => {
                assert_eq!(path, "/hook");
                assert_eq!(body, serde_json::json!({"key": "val"}));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn manifest_serialization() {
        let manifest = PluginManifest {
            id: "test-plugin".into(),
            version: "0.1.0".into(),
            author: "corvid".into(),
            description: "A test plugin".into(),
            capabilities: vec![Capability::AlgoRead, Capability::DbRead],
            event_filter: vec![EventKind::AgentMessage],
            trust_tier: TrustTier::Verified,
            min_host_version: "0.1.0".into(),
            tools: vec![],
            dependencies: vec![],
        };

        let json = serde_json::to_string(&manifest).unwrap();
        assert!(json.contains("test-plugin"));
        assert!(json.contains("Verified"));
    }

    #[test]
    fn manifest_tools_roundtrip() {
        let manifest = PluginManifest {
            id: "oracle-plugin".into(),
            version: "1.0.0".into(),
            author: "corvid".into(),
            description: "Oracle with tools".into(),
            capabilities: vec![],
            event_filter: vec![],
            trust_tier: TrustTier::Untrusted,
            min_host_version: "0.1.0".into(),
            tools: vec![ToolInfo {
                name: "get_price".into(),
                description: "Get current asset price".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "asset": { "type": "string" }
                    },
                    "required": ["asset"]
                }),
            }],
            dependencies: vec![],
        };

        // Verify msgpack roundtrip (same path as WASM manifest extraction)
        let packed = rmp_serde::to_vec(&manifest).unwrap();
        let unpacked: PluginManifest = rmp_serde::from_slice(&packed).unwrap();
        assert_eq!(unpacked.tools.len(), 1);
        assert_eq!(unpacked.tools[0].name, "get_price");

        // Manifest without tools deserializes without error (backward compat)
        let json_no_tools = serde_json::json!({
            "id": "old-plugin",
            "version": "1.0.0",
            "author": "corvid",
            "description": "Old plugin without tools field",
            "capabilities": [],
            "event_filter": [],
            "trust_tier": "Untrusted",
            "min_host_version": "0.1.0"
        });
        let old: PluginManifest = serde_json::from_value(json_no_tools).unwrap();
        assert_eq!(old.tools.len(), 0);
    }

    #[test]
    fn plugin_error_display() {
        let err = PluginError::Init("db connection failed".into());
        assert_eq!(err.to_string(), "init failed: db connection failed");

        let err = PluginError::MissingCapability(Capability::DbRead);
        assert_eq!(err.to_string(), "missing capability: DbRead");

        let err = PluginError::Timeout;
        assert_eq!(err.to_string(), "execution timed out");
    }

    #[test]
    fn trust_tier_serialization() {
        let tier = TrustTier::Untrusted;
        let json = serde_json::to_string(&tier).unwrap();
        assert_eq!(json, "\"Untrusted\"");

        let parsed: TrustTier = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, TrustTier::Untrusted);
    }
}
