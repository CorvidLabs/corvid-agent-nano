//! The event-driven runtime — the heart of nano.
//!
//! Coordinates transport polling, plugin dispatch, and action execution.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use nano_transport::{OutboundMessage, Transport};

use crate::action::{Action, LogLevel};
use crate::event::{Event, EventKind};
use crate::plugin::{Plugin, PluginContext};
use crate::state::StateStore;

/// Runtime configuration.
pub struct RuntimeConfig {
    /// How often to poll the transport for messages (in seconds).
    pub poll_interval_secs: u64,
    /// Agent display name.
    pub agent_name: String,
    /// Per-plugin config sections from nano.toml.
    pub plugin_configs: HashMap<String, toml::Table>,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            poll_interval_secs: 5,
            agent_name: "can".to_string(),
            plugin_configs: HashMap::new(),
        }
    }
}

/// The nano runtime — event loop, plugin host, transport coordination.
pub struct Runtime {
    transport: Arc<dyn Transport>,
    plugins: Vec<Box<dyn Plugin>>,
    state: StateStore,
    config: RuntimeConfig,
    /// Channel for plugins/internal code to inject events into the loop.
    event_tx: mpsc::Sender<Event>,
    event_rx: mpsc::Receiver<Event>,
}

impl Runtime {
    /// Create a new runtime with the given transport and config.
    pub fn new(transport: Arc<dyn Transport>, config: RuntimeConfig) -> Self {
        let (event_tx, event_rx) = mpsc::channel(256);
        Self {
            transport,
            plugins: Vec::new(),
            state: StateStore::new(),
            config,
            event_tx,
            event_rx,
        }
    }

    /// Register a plugin with the runtime.
    pub async fn add_plugin(&mut self, mut plugin: Box<dyn Plugin>) -> Result<()> {
        let name = plugin.name().to_string();
        let ctx = self.make_context(&name);
        plugin.init(&ctx).await?;
        info!(plugin = %name, version = %plugin.version(), "plugin loaded");
        self.plugins.push(plugin);

        // Notify other plugins
        let _ = self.event_tx.send(Event::PluginLoaded { name }).await;
        Ok(())
    }

    /// Get a sender for injecting events into the runtime loop.
    pub fn event_sender(&self) -> mpsc::Sender<Event> {
        self.event_tx.clone()
    }

    /// Run the event loop. Blocks until shutdown signal or ctrl-c.
    pub async fn run(&mut self, mut shutdown: tokio::sync::watch::Receiver<bool>) -> Result<()> {
        let poll_interval = Duration::from_secs(self.config.poll_interval_secs);

        info!(
            agent = %self.config.agent_name,
            transport = %self.transport.name(),
            plugins = self.plugins.len(),
            poll_secs = self.config.poll_interval_secs,
            "runtime started"
        );

        let mut poll_ticker = tokio::time::interval(poll_interval);
        // Don't burst-catch-up if processing takes a while
        poll_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                // Transport poll tick
                _ = poll_ticker.tick() => {
                    match self.transport.recv().await {
                        Ok(messages) => {
                            for msg in messages {
                                debug!(from = %msg.sender, "message received");
                                let event = Event::MessageReceived(msg);
                                self.dispatch_event(&event).await;
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, "transport recv failed");
                        }
                    }
                }

                // Internal event bus
                Some(event) = self.event_rx.recv() => {
                    self.dispatch_event(&event).await;
                }

                // Shutdown signal
                Ok(()) = shutdown.changed() => {
                    if *shutdown.borrow() {
                        info!("shutdown signal received");
                        break;
                    }
                }
            }
        }

        // Graceful shutdown: notify plugins
        self.dispatch_event(&Event::Shutdown).await;
        for plugin in &self.plugins {
            if let Err(e) = plugin.shutdown().await {
                warn!(plugin = %plugin.name(), error = %e, "plugin shutdown error");
            }
        }

        info!("runtime stopped");
        Ok(())
    }

    /// Dispatch an event to all subscribed plugins and execute resulting actions.
    async fn dispatch_event(&mut self, event: &Event) {
        let event_kind = event.kind();

        for i in 0..self.plugins.len() {
            let subs = self.plugins[i].subscriptions();
            if !subs.contains(&EventKind::All) && !subs.contains(&event_kind) {
                continue;
            }

            let name = self.plugins[i].name().to_string();
            let ctx = self.make_context(&name);

            match self.plugins[i].handle_event(event, &ctx).await {
                Ok(actions) => {
                    for action in actions {
                        if let Err(e) = self.execute_action(&name, action).await {
                            error!(plugin = %name, error = %e, "action execution failed");
                        }
                    }
                }
                Err(e) => {
                    error!(plugin = %name, error = %e, "plugin event handler failed");
                }
            }
        }
    }

    /// Execute a single action returned by a plugin.
    async fn execute_action(&mut self, plugin_name: &str, action: Action) -> Result<()> {
        match action {
            Action::SendMessage { to, content } => {
                debug!(plugin = %plugin_name, to = %to, "executing SendMessage");
                let result = self
                    .transport
                    .send(OutboundMessage {
                        to: to.clone(),
                        content,
                    })
                    .await?;
                info!(plugin = %plugin_name, to = %to, tx_id = %result.id, "message sent");

                // Emit MessageSent event
                let _ = self
                    .event_tx
                    .send(Event::MessageSent {
                        to,
                        tx_id: result.id,
                    })
                    .await;
            }
            Action::StoreState { key, value } => {
                debug!(plugin = %plugin_name, key = %key, "executing StoreState");
                self.state.set(plugin_name, &key, value);
            }
            Action::EmitEvent { kind, data } => {
                debug!(plugin = %plugin_name, kind = %kind, "executing EmitEvent");
                let _ = self.event_tx.send(Event::Custom { kind, data }).await;
            }
            Action::Log { level, message } => match level {
                LogLevel::Trace => tracing::trace!(plugin = %plugin_name, "{}", message),
                LogLevel::Debug => tracing::debug!(plugin = %plugin_name, "{}", message),
                LogLevel::Info => tracing::info!(plugin = %plugin_name, "{}", message),
                LogLevel::Warn => tracing::warn!(plugin = %plugin_name, "{}", message),
                LogLevel::Error => tracing::error!(plugin = %plugin_name, "{}", message),
            },
        }
        Ok(())
    }

    /// Build a PluginContext for the given plugin.
    fn make_context(&self, plugin_name: &str) -> PluginContext {
        PluginContext {
            agent_address: self.transport.local_address().to_string(),
            agent_name: self.config.agent_name.clone(),
            state: self.state.snapshot(plugin_name),
            config: self
                .config
                .plugin_configs
                .get(plugin_name)
                .cloned()
                .unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::EventKind;
    use nano_transport::NullTransport;

    // A test plugin that echoes messages back
    struct EchoPlugin {
        received: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl EchoPlugin {
        fn new() -> (Self, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
            let received = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            (
                Self {
                    received: received.clone(),
                },
                received,
            )
        }
    }

    #[async_trait::async_trait]
    impl Plugin for EchoPlugin {
        fn name(&self) -> &str {
            "echo"
        }
        fn version(&self) -> &str {
            "0.1.0"
        }
        async fn init(&mut self, _ctx: &PluginContext) -> Result<()> {
            Ok(())
        }
        async fn handle_event(&self, event: &Event, _ctx: &PluginContext) -> Result<Vec<Action>> {
            match event {
                Event::MessageReceived(msg) => {
                    self.received.lock().unwrap().push(msg.content.clone());
                    Ok(vec![Action::SendMessage {
                        to: msg.sender.clone(),
                        content: format!("echo: {}", msg.content),
                    }])
                }
                _ => Ok(vec![]),
            }
        }
        fn subscriptions(&self) -> Vec<EventKind> {
            vec![EventKind::MessageReceived]
        }
    }

    // A plugin that subscribes to All events
    struct SpyPlugin {
        events: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl SpyPlugin {
        fn new() -> (Self, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
            let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            (
                Self {
                    events: events.clone(),
                },
                events,
            )
        }
    }

    #[async_trait::async_trait]
    impl Plugin for SpyPlugin {
        fn name(&self) -> &str {
            "spy"
        }
        fn version(&self) -> &str {
            "0.1.0"
        }
        async fn init(&mut self, _ctx: &PluginContext) -> Result<()> {
            Ok(())
        }
        async fn handle_event(&self, event: &Event, _ctx: &PluginContext) -> Result<Vec<Action>> {
            self.events
                .lock()
                .unwrap()
                .push(format!("{:?}", event.kind()));
            Ok(vec![])
        }
        fn subscriptions(&self) -> Vec<EventKind> {
            vec![EventKind::All]
        }
    }

    #[tokio::test]
    async fn runtime_creates_and_loads_plugin() {
        let transport = Arc::new(NullTransport::new("test-addr"));
        let mut runtime = Runtime::new(transport, RuntimeConfig::default());
        let (echo, _received) = EchoPlugin::new();
        runtime.add_plugin(Box::new(echo)).await.unwrap();
        assert_eq!(runtime.plugins.len(), 1);
    }

    #[tokio::test]
    async fn runtime_dispatches_message_to_plugin() {
        let transport = Arc::new(NullTransport::new("test-addr"));
        let mut runtime = Runtime::new(transport, RuntimeConfig::default());

        let (echo, received) = EchoPlugin::new();
        runtime.add_plugin(Box::new(echo)).await.unwrap();

        let msg = nano_transport::Message {
            sender: "alice".into(),
            recipient: "test-addr".into(),
            content: "hello".into(),
            timestamp: chrono::Utc::now(),
            metadata: serde_json::Value::Null,
        };

        runtime.dispatch_event(&Event::MessageReceived(msg)).await;

        let r = received.lock().unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0], "hello");
    }

    #[tokio::test]
    async fn runtime_skips_unsubscribed_events() {
        let transport = Arc::new(NullTransport::new("test-addr"));
        let mut runtime = Runtime::new(transport, RuntimeConfig::default());

        let (echo, received) = EchoPlugin::new();
        runtime.add_plugin(Box::new(echo)).await.unwrap();

        // Echo only subscribes to MessageReceived, not Timer
        runtime
            .dispatch_event(&Event::Timer {
                timestamp: chrono::Utc::now(),
            })
            .await;

        assert!(received.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn runtime_all_subscription_receives_everything() {
        let transport = Arc::new(NullTransport::new("test-addr"));
        let mut runtime = Runtime::new(transport, RuntimeConfig::default());

        let (spy, events) = SpyPlugin::new();
        runtime.add_plugin(Box::new(spy)).await.unwrap();

        // PluginLoaded event from adding the spy itself
        let initial_count = events.lock().unwrap().len();

        runtime
            .dispatch_event(&Event::Timer {
                timestamp: chrono::Utc::now(),
            })
            .await;

        runtime.dispatch_event(&Event::Shutdown).await;

        let all = events.lock().unwrap();
        // Should have Timer + Shutdown (plus possibly PluginLoaded from init)
        assert!(all.len() >= initial_count + 2);
    }

    #[tokio::test]
    async fn runtime_store_state_action() {
        let transport = Arc::new(NullTransport::new("test-addr"));
        let mut runtime = Runtime::new(transport, RuntimeConfig::default());

        runtime
            .execute_action(
                "test-plugin",
                Action::StoreState {
                    key: "counter".into(),
                    value: serde_json::Value::from(42),
                },
            )
            .await
            .unwrap();

        assert_eq!(
            runtime.state.get("test-plugin", "counter"),
            Some(&serde_json::Value::from(42))
        );
    }

    #[tokio::test]
    async fn runtime_plugin_context_has_config() {
        let transport = Arc::new(NullTransport::new("test-addr"));
        let mut plugin_configs = HashMap::new();
        let mut hub_config = toml::Table::new();
        hub_config.insert(
            "url".to_string(),
            toml::Value::String("http://localhost:3578".into()),
        );
        plugin_configs.insert("hub".to_string(), hub_config);

        let config = RuntimeConfig {
            plugin_configs,
            ..Default::default()
        };

        let runtime = Runtime::new(transport, config);
        let ctx = runtime.make_context("hub");
        assert_eq!(
            ctx.config.get("url").and_then(|v| v.as_str()),
            Some("http://localhost:3578")
        );
    }

    #[tokio::test]
    async fn runtime_run_and_shutdown() {
        let transport = Arc::new(NullTransport::new("test-addr"));
        let mut runtime = Runtime::new(
            transport,
            RuntimeConfig {
                poll_interval_secs: 1,
                ..Default::default()
            },
        );

        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        // Shut down after a short delay
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let _ = shutdown_tx.send(true);
        });

        runtime.run(shutdown_rx).await.unwrap();
        // If we get here, the runtime shut down cleanly
    }
}
