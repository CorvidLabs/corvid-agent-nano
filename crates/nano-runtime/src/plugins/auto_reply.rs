//! Auto-reply plugin — pattern-matching responder for when no AI is connected.
//!
//! Configured via nano.toml:
//! ```toml
//! [plugins.auto-reply]
//! rules = [
//!     { match = "ping", reply = "pong" },
//!     { match = "status", reply = "online" },
//! ]
//! ```

use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use tracing::{debug, info};

use crate::action::Action;
use crate::event::{Event, EventKind};
use crate::plugin::{Plugin, PluginContext};

/// A match/reply rule.
#[derive(Debug, Clone, Deserialize)]
struct Rule {
    /// Substring to match (case-insensitive).
    #[serde(rename = "match")]
    pattern: String,
    /// Reply text.
    reply: String,
}

/// Auto-reply plugin.
pub struct AutoReplyPlugin {
    rules: Vec<Rule>,
}

impl AutoReplyPlugin {
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    /// Create with pre-set rules (for testing or programmatic use).
    pub fn with_rules(rules: Vec<(String, String)>) -> Self {
        Self {
            rules: rules
                .into_iter()
                .map(|(pattern, reply)| Rule { pattern, reply })
                .collect(),
        }
    }
}

impl Default for AutoReplyPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Plugin for AutoReplyPlugin {
    fn name(&self) -> &str {
        "auto-reply"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    async fn init(&mut self, ctx: &PluginContext) -> Result<()> {
        // Load rules from config
        if let Some(toml::Value::Array(arr)) = ctx.config.get("rules") {
            for item in arr {
                if let toml::Value::Table(tbl) = item {
                    let pattern = tbl
                        .get("match")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let reply = tbl
                        .get("reply")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    if !pattern.is_empty() {
                        self.rules.push(Rule { pattern, reply });
                    }
                }
            }
        }
        info!(rules = self.rules.len(), "auto-reply plugin initialized");
        Ok(())
    }

    async fn handle_event(&self, event: &Event, _ctx: &PluginContext) -> Result<Vec<Action>> {
        match event {
            Event::MessageReceived(msg) => {
                let content_lower = msg.content.to_lowercase();
                for rule in &self.rules {
                    if content_lower.contains(&rule.pattern.to_lowercase()) {
                        debug!(
                            pattern = %rule.pattern,
                            sender = %msg.sender,
                            "auto-reply matched"
                        );
                        return Ok(vec![Action::SendMessage {
                            to: msg.sender.clone(),
                            content: rule.reply.clone(),
                        }]);
                    }
                }
                Ok(vec![])
            }
            _ => Ok(vec![]),
        }
    }

    fn subscriptions(&self) -> Vec<EventKind> {
        vec![EventKind::MessageReceived]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nano_transport::Message;

    fn test_ctx() -> PluginContext {
        PluginContext {
            agent_address: "test".into(),
            agent_name: "test".into(),
            state: Default::default(),
            config: Default::default(),
        }
    }

    fn make_msg(content: &str) -> Event {
        Event::MessageReceived(Message {
            sender: "alice".into(),
            recipient: "bob".into(),
            content: content.into(),
            timestamp: chrono::Utc::now(),
            metadata: serde_json::Value::Null,
        })
    }

    #[tokio::test]
    async fn matches_ping_pong() {
        let plugin = AutoReplyPlugin::with_rules(vec![("ping".into(), "pong".into())]);
        let actions = plugin
            .handle_event(&make_msg("ping"), &test_ctx())
            .await
            .unwrap();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            Action::SendMessage { to, content } => {
                assert_eq!(to, "alice");
                assert_eq!(content, "pong");
            }
            _ => panic!("expected SendMessage"),
        }
    }

    #[tokio::test]
    async fn case_insensitive_match() {
        let plugin = AutoReplyPlugin::with_rules(vec![("ping".into(), "pong".into())]);
        let actions = plugin
            .handle_event(&make_msg("PING"), &test_ctx())
            .await
            .unwrap();
        assert_eq!(actions.len(), 1);
    }

    #[tokio::test]
    async fn substring_match() {
        let plugin = AutoReplyPlugin::with_rules(vec![("status".into(), "online".into())]);
        let actions = plugin
            .handle_event(&make_msg("what is your status?"), &test_ctx())
            .await
            .unwrap();
        assert_eq!(actions.len(), 1);
    }

    #[tokio::test]
    async fn no_match_returns_empty() {
        let plugin = AutoReplyPlugin::with_rules(vec![("ping".into(), "pong".into())]);
        let actions = plugin
            .handle_event(&make_msg("hello"), &test_ctx())
            .await
            .unwrap();
        assert!(actions.is_empty());
    }

    #[tokio::test]
    async fn first_match_wins() {
        let plugin = AutoReplyPlugin::with_rules(vec![
            ("hello".into(), "hi!".into()),
            ("hello".into(), "hey!".into()),
        ]);
        let actions = plugin
            .handle_event(&make_msg("hello"), &test_ctx())
            .await
            .unwrap();
        assert_eq!(actions.len(), 1);
        match &actions[0] {
            Action::SendMessage { content, .. } => assert_eq!(content, "hi!"),
            _ => panic!("expected SendMessage"),
        }
    }

    #[tokio::test]
    async fn ignores_non_message_events() {
        let plugin = AutoReplyPlugin::with_rules(vec![("ping".into(), "pong".into())]);
        let actions = plugin
            .handle_event(
                &Event::Timer {
                    timestamp: chrono::Utc::now(),
                },
                &test_ctx(),
            )
            .await
            .unwrap();
        assert!(actions.is_empty());
    }

    #[test]
    fn plugin_name_and_version() {
        let plugin = AutoReplyPlugin::new();
        assert_eq!(plugin.name(), "auto-reply");
        assert!(!plugin.version().is_empty());
    }

    #[tokio::test]
    async fn init_loads_rules_from_config() {
        let mut plugin = AutoReplyPlugin::new();
        let mut config = toml::Table::new();

        // Build rules as TOML array of tables
        let mut rule = toml::Table::new();
        rule.insert("match".into(), toml::Value::String("test".into()));
        rule.insert("reply".into(), toml::Value::String("works!".into()));

        config.insert(
            "rules".into(),
            toml::Value::Array(vec![toml::Value::Table(rule)]),
        );

        let ctx = PluginContext {
            agent_address: "test".into(),
            agent_name: "test".into(),
            state: Default::default(),
            config,
        };

        plugin.init(&ctx).await.unwrap();
        assert_eq!(plugin.rules.len(), 1);
        assert_eq!(plugin.rules[0].pattern, "test");
        assert_eq!(plugin.rules[0].reply, "works!");
    }
}
