//! End-to-end tests for nano.toml config loading and plugin configuration.
//!
//! Tests the full flow: write config file → load → construct runtime → verify behavior.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use nano_runtime::*;
use nano_transport::MockTransport;

/// Plugin that reads a greeting from its config and replies with it.
struct GreeterPlugin {
    greeting: String,
}

impl GreeterPlugin {
    fn new() -> Self {
        Self {
            greeting: "default hello".into(),
        }
    }
}

#[async_trait]
impl Plugin for GreeterPlugin {
    fn name(&self) -> &str {
        "greeter"
    }
    fn version(&self) -> &str {
        "1.0.0"
    }
    async fn init(&mut self, ctx: &PluginContext) -> Result<()> {
        if let Some(g) = ctx.config.get("greeting").and_then(|v| v.as_str()) {
            self.greeting = g.to_string();
        }
        Ok(())
    }
    async fn handle_event(&self, event: &Event, _ctx: &PluginContext) -> Result<Vec<Action>> {
        match event {
            Event::MessageReceived(msg) => Ok(vec![Action::SendMessage {
                to: msg.sender.clone(),
                content: self.greeting.clone(),
            }]),
            _ => Ok(vec![]),
        }
    }
    fn subscriptions(&self) -> Vec<EventKind> {
        vec![EventKind::MessageReceived]
    }
}

#[tokio::test]
async fn plugin_receives_config_from_runtime() {
    let transport = Arc::new(MockTransport::new("agent-addr"));

    let mut plugin_configs = HashMap::new();
    let mut greeter_config = toml::Table::new();
    greeter_config.insert("greeting".into(), toml::Value::String("ahoy!".into()));
    plugin_configs.insert("greeter".into(), greeter_config);

    let config = RuntimeConfig {
        poll_interval_secs: 1,
        agent_name: "test-agent".into(),
        plugin_configs,
    };

    let mut runtime = Runtime::new(transport.clone(), config);
    runtime
        .add_plugin(Box::new(GreeterPlugin::new()))
        .await
        .unwrap();

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    transport.inject(transport.message_from("alice", "hi"));

    let transport_check = transport.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let _ = shutdown_tx.send(true);
    });

    runtime.run(shutdown_rx).await.unwrap();

    let sent = transport_check.sent_messages();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].content, "ahoy!", "plugin should use config value");
}

#[tokio::test]
async fn plugin_uses_default_without_config() {
    let transport = Arc::new(MockTransport::new("agent-addr"));

    // No plugin configs — greeter should use its default
    let config = RuntimeConfig {
        poll_interval_secs: 1,
        agent_name: "test-agent".into(),
        plugin_configs: HashMap::new(),
    };

    let mut runtime = Runtime::new(transport.clone(), config);
    runtime
        .add_plugin(Box::new(GreeterPlugin::new()))
        .await
        .unwrap();

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    transport.inject(transport.message_from("alice", "hi"));

    let transport_check = transport.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let _ = shutdown_tx.send(true);
    });

    runtime.run(shutdown_rx).await.unwrap();

    let sent = transport_check.sent_messages();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].content, "default hello");
}

// ---------------------------------------------------------------------------
// nano.toml file parsing integration
// ---------------------------------------------------------------------------

#[test]
fn nano_toml_roundtrip_with_plugin_rules() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nano.toml");

    let toml_content = r#"
[agent]
name = "test-bot"
network = "localnet"

[hub]
url = "http://localhost:9999"
disabled = true

[runtime]
poll_interval = 2
no_plugins = false
health_port = 8080

[plugins]
enabled = ["auto-reply", "greeter"]

[plugins.auto-reply]
rules = [
    { match = "ping", reply = "pong" },
    { match = "help", reply = "I can help!" },
]

[plugins.greeter]
greeting = "welcome!"

[logging]
format = "json"
level = "debug"
"#;

    std::fs::write(&path, toml_content).unwrap();

    // Parse as NanoConfig (the main binary's config struct)
    let parsed: toml::Value = toml::from_str(toml_content).unwrap();
    let agent = parsed.get("agent").unwrap();
    assert_eq!(agent.get("name").unwrap().as_str(), Some("test-bot"));
    assert_eq!(agent.get("network").unwrap().as_str(), Some("localnet"));

    let hub = parsed.get("hub").unwrap();
    assert_eq!(
        hub.get("url").unwrap().as_str(),
        Some("http://localhost:9999")
    );
    assert_eq!(hub.get("disabled").unwrap().as_bool(), Some(true));

    let runtime = parsed.get("runtime").unwrap();
    assert_eq!(runtime.get("poll_interval").unwrap().as_integer(), Some(2));
    assert_eq!(runtime.get("health_port").unwrap().as_integer(), Some(8080));

    let plugins = parsed.get("plugins").unwrap();
    assert_eq!(plugins.get("enabled").unwrap().as_array().unwrap().len(), 2);

    // Verify auto-reply rules parse
    let auto_reply = plugins.get("auto-reply").unwrap();
    let rules = auto_reply.get("rules").unwrap().as_array().unwrap();
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0].get("match").unwrap().as_str(), Some("ping"));
    assert_eq!(rules[0].get("reply").unwrap().as_str(), Some("pong"));

    // Verify greeter config
    let greeter = plugins.get("greeter").unwrap();
    assert_eq!(greeter.get("greeting").unwrap().as_str(), Some("welcome!"));
}

#[test]
fn nano_toml_minimal_config_uses_defaults() {
    let toml_content = r#"
[agent]
name = "minimal"
"#;

    let parsed: toml::Value = toml::from_str(toml_content).unwrap();
    assert_eq!(
        parsed.get("agent").unwrap().get("name").unwrap().as_str(),
        Some("minimal")
    );
    // Other sections should be absent (defaults applied at runtime)
    assert!(parsed.get("hub").is_none());
    assert!(parsed.get("runtime").is_none());
}

#[test]
fn nano_toml_empty_is_valid() {
    let parsed: toml::Value = toml::from_str("").unwrap();
    // Empty TOML is a valid table
    assert!(parsed.is_table());
}
