//! Configuration file support for nano.toml.
//!
//! Provides persistent defaults that CLI flags and env vars override.
//! Precedence: CLI flag > env var > nano.toml > built-in default.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Top-level nano.toml configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NanoConfig {
    #[serde(default)]
    pub agent: AgentConfig,

    #[serde(default)]
    pub network: NetworkConfig,

    #[serde(default)]
    pub hub: HubConfig,

    #[serde(default)]
    pub runtime: RuntimeConfig,

    #[serde(default)]
    pub plugins: PluginsConfig,

    #[serde(default)]
    pub logging: LoggingConfig,
}

/// Agent identity and display settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Agent display name.
    #[serde(default = "default_agent_name")]
    pub name: String,

    /// Default Algorand network preset: localnet, testnet, mainnet.
    #[serde(default)]
    pub network: Option<String>,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            name: default_agent_name(),
            network: None,
        }
    }
}

fn default_agent_name() -> String {
    "can".into()
}

/// Algorand network endpoint overrides.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkConfig {
    pub algod_url: Option<String>,
    pub algod_token: Option<String>,
    pub indexer_url: Option<String>,
    pub indexer_token: Option<String>,
}

/// Hub connection settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubConfig {
    /// Hub URL. Defaults to localhost.
    #[serde(default = "default_hub_url")]
    pub url: String,

    /// Disable hub forwarding (P2P mode).
    #[serde(default)]
    pub disabled: bool,
}

impl Default for HubConfig {
    fn default() -> Self {
        Self {
            url: default_hub_url(),
            disabled: false,
        }
    }
}

fn default_hub_url() -> String {
    "http://localhost:3578".into()
}

/// Runtime behavior settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeConfig {
    /// Message poll interval in seconds.
    #[serde(default = "default_poll_interval")]
    pub poll_interval: u64,

    /// Disable the plugin host sidecar.
    #[serde(default)]
    pub no_plugins: bool,

    /// Health check HTTP port (None = disabled).
    #[serde(default)]
    pub health_port: Option<u16>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            poll_interval: default_poll_interval(),
            no_plugins: false,
            health_port: None,
        }
    }
}

fn default_poll_interval() -> u64 {
    5
}

/// Plugin system configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PluginsConfig {
    /// List of enabled plugin IDs.
    #[serde(default)]
    pub enabled: Vec<String>,

    /// Per-plugin configuration tables.
    #[serde(flatten)]
    pub plugin_configs: HashMap<String, toml::Value>,
}

/// Logging configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Log format: "text" or "json".
    #[serde(default)]
    pub format: Option<String>,

    /// Log level filter (e.g. "info", "debug").
    #[serde(default)]
    pub level: Option<String>,
}

impl NanoConfig {
    /// Load config from `{data_dir}/nano.toml`. Returns default if file doesn't exist.
    pub fn load(data_dir: &str) -> Result<Self> {
        let path = Path::new(data_dir).join("nano.toml");
        Self::load_from(&path)
    }

    /// Load config from a specific path. Returns default if file doesn't exist.
    pub fn load_from(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(path)?;
        let config: NanoConfig = toml::from_str(&content)?;
        Ok(config)
    }

    /// Save config to `{data_dir}/nano.toml`.
    pub fn save(&self, data_dir: &str) -> Result<()> {
        let path = Path::new(data_dir).join("nano.toml");
        self.save_to(&path)
    }

    /// Save config to a specific path.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Generate a config file for a newly set up agent.
    pub fn for_new_agent(name: &str, network: &str) -> Self {
        Self {
            agent: AgentConfig {
                name: name.into(),
                network: Some(network.into()),
            },
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_default_config() {
        let config = NanoConfig::default();
        assert_eq!(config.agent.name, "can");
        assert!(config.agent.network.is_none());
        assert_eq!(config.hub.url, "http://localhost:3578");
        assert!(!config.hub.disabled);
        assert_eq!(config.runtime.poll_interval, 5);
        assert!(!config.runtime.no_plugins);
    }

    #[test]
    fn test_load_missing_file() {
        let config = NanoConfig::load_from(Path::new("/nonexistent/nano.toml")).unwrap();
        assert_eq!(config.agent.name, "can");
    }

    #[test]
    fn test_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nano.toml");

        let config = NanoConfig::for_new_agent("my-agent", "testnet");
        config.save_to(&path).unwrap();

        let loaded = NanoConfig::load_from(&path).unwrap();
        assert_eq!(loaded.agent.name, "my-agent");
        assert_eq!(loaded.agent.network.as_deref(), Some("testnet"));
    }

    #[test]
    fn test_parse_full_config() {
        let toml_str = r#"
[agent]
name = "kira"
network = "testnet"

[network]
algod_url = "https://custom-api.example.com"
algod_token = "my-token"

[hub]
url = "https://hub.corvid.dev"
disabled = false

[runtime]
poll_interval = 10
no_plugins = true
health_port = 9090

[plugins]
enabled = ["hello-world", "auto-reply"]

[logging]
format = "json"
level = "debug"
"#;

        let config: NanoConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.agent.name, "kira");
        assert_eq!(config.agent.network.as_deref(), Some("testnet"));
        assert_eq!(
            config.network.algod_url.as_deref(),
            Some("https://custom-api.example.com")
        );
        assert_eq!(config.runtime.poll_interval, 10);
        assert!(config.runtime.no_plugins);
        assert_eq!(config.runtime.health_port, Some(9090));
        assert_eq!(config.plugins.enabled, vec!["hello-world", "auto-reply"]);
        assert_eq!(config.logging.format.as_deref(), Some("json"));
        assert_eq!(config.logging.level.as_deref(), Some("debug"));
    }

    #[test]
    fn test_partial_config() {
        let toml_str = r#"
[agent]
name = "test"
"#;
        let config: NanoConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.agent.name, "test");
        // Everything else should be default
        assert_eq!(config.hub.url, "http://localhost:3578");
        assert_eq!(config.runtime.poll_interval, 5);
    }

    #[test]
    fn test_save_creates_valid_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nano.toml");

        let mut config = NanoConfig::for_new_agent("test-agent", "mainnet");
        config.hub.url = "https://hub.example.com".into();
        config.runtime.poll_interval = 15;
        config.save_to(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("test-agent"));
        assert!(content.contains("mainnet"));
        assert!(content.contains("hub.example.com"));

        // Verify it parses back
        let loaded = NanoConfig::load_from(&path).unwrap();
        assert_eq!(loaded.agent.name, "test-agent");
        assert_eq!(loaded.runtime.poll_interval, 15);
    }

    #[test]
    fn test_empty_file_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nano.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"").unwrap();

        let config = NanoConfig::load_from(&path).unwrap();
        assert_eq!(config.agent.name, "can");
        assert_eq!(config.runtime.poll_interval, 5);
    }
}
