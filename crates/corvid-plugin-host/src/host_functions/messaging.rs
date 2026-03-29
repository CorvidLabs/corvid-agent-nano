//! Host function: agent message bus.
//!
//! Provides `host_send_message(target_ptr, target_len, msg_ptr, msg_len)` which
//! sends a message to another agent via a pluggable backend. The plugin's
//! `target_filter` from its AgentMessage capability is enforced — messages
//! to targets not matching the filter are rejected.

use std::sync::Arc;

use wasmtime::Linker;

use crate::loader::PluginState;
use crate::wasm_mem;

/// Backend for dispatching messages to agents.
///
/// In production, this sends via the agent's messaging system (AlgoChat, etc).
/// In tests, it can be replaced with a mock.
pub struct MessagingBackend {
    inner: Box<dyn MessageDispatch + Send + Sync>,
}

/// Trait for sending messages — allows mocking in tests.
pub trait MessageDispatch: Send + Sync {
    /// Send a message to the given target agent.
    /// Returns Ok(()) on success, Err(reason) on failure.
    fn send(&self, from_plugin: &str, target: &str, message: &str) -> Result<(), String>;
}

impl MessagingBackend {
    pub fn new(dispatch: impl MessageDispatch + 'static) -> Self {
        Self {
            inner: Box::new(dispatch),
        }
    }

    pub fn send(&self, from_plugin: &str, target: &str, message: &str) -> Result<(), String> {
        self.inner.send(from_plugin, target, message)
    }
}

/// Check if a target matches the plugin's declared target_filter.
///
/// The filter supports glob-style patterns:
/// - `*` matches any agent
/// - `team-*` matches any agent starting with "team-"
/// - Exact match otherwise
pub fn matches_target_filter(target: &str, filter: &str) -> bool {
    if filter == "*" {
        return true;
    }
    if let Some(prefix) = filter.strip_suffix('*') {
        return target.starts_with(prefix);
    }
    target == filter
}

/// Link messaging host functions into the WASM linker.
pub fn link(linker: &mut Linker<PluginState>) -> anyhow::Result<()> {
    // host_send_message(target_ptr, target_len, msg_ptr, msg_len) -> status
    // Returns: 0 = success, -1 = error, -2 = target rejected by filter
    linker.func_wrap(
        "env",
        "host_send_message",
        |mut caller: wasmtime::Caller<'_, PluginState>,
         target_ptr: i32,
         target_len: i32,
         msg_ptr: i32,
         msg_len: i32|
         -> i32 {
            // Read target string from WASM memory
            let target = match wasm_mem::read_str(&mut caller, target_ptr, target_len) {
                Some(t) => t,
                None => {
                    tracing::warn!("host_send_message: failed to read target from WASM memory");
                    return -1;
                }
            };

            // Read message string from WASM memory
            let message = match wasm_mem::read_str(&mut caller, msg_ptr, msg_len) {
                Some(m) => m,
                None => {
                    tracing::warn!("host_send_message: failed to read message from WASM memory");
                    return -1;
                }
            };

            // Enforce target_filter from the plugin's AgentMessage capability
            let filter = match &caller.data().message_target_filter {
                Some(f) => f.clone(),
                None => {
                    tracing::warn!(
                        plugin_id = %caller.data().plugin_id,
                        "host_send_message: no target_filter configured"
                    );
                    return -2;
                }
            };

            if !matches_target_filter(&target, &filter) {
                tracing::warn!(
                    plugin_id = %caller.data().plugin_id,
                    target = %target,
                    filter = %filter,
                    "host_send_message: target rejected by filter"
                );
                return -2;
            }

            let plugin_id = caller.data().plugin_id.clone();
            let messaging = match &caller.data().messaging {
                Some(m) => Arc::clone(m),
                None => {
                    tracing::error!("host_send_message: messaging backend not initialized");
                    return -1;
                }
            };

            match messaging.send(&plugin_id, &target, &message) {
                Ok(()) => {
                    tracing::info!(
                        plugin_id = %plugin_id,
                        target = %target,
                        "host_send_message: message sent"
                    );
                    0
                }
                Err(e) => {
                    tracing::warn!(
                        plugin_id = %plugin_id,
                        target = %target,
                        error = %e,
                        "host_send_message: send failed"
                    );
                    -1
                }
            }
        },
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct MockDispatch {
        sent: Arc<Mutex<Vec<(String, String, String)>>>,
    }

    impl MockDispatch {
        fn new() -> (Self, Arc<Mutex<Vec<(String, String, String)>>>) {
            let sent = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    sent: Arc::clone(&sent),
                },
                sent,
            )
        }
    }

    impl MessageDispatch for MockDispatch {
        fn send(&self, from: &str, target: &str, message: &str) -> Result<(), String> {
            self.sent.lock().unwrap().push((
                from.to_string(),
                target.to_string(),
                message.to_string(),
            ));
            Ok(())
        }
    }

    #[test]
    fn messaging_link_compiles() {
        let engine = wasmtime::Engine::default();
        let mut linker = wasmtime::Linker::new(&engine);
        assert!(link(&mut linker).is_ok());
    }

    #[test]
    fn target_filter_wildcard() {
        assert!(matches_target_filter("any-agent", "*"));
        assert!(matches_target_filter("", "*"));
    }

    #[test]
    fn target_filter_prefix() {
        assert!(matches_target_filter("team-alpha", "team-*"));
        assert!(matches_target_filter("team-", "team-*"));
        assert!(!matches_target_filter("other-agent", "team-*"));
    }

    #[test]
    fn target_filter_exact() {
        assert!(matches_target_filter("corvid-agent", "corvid-agent"));
        assert!(!matches_target_filter("other", "corvid-agent"));
    }

    #[test]
    fn messaging_backend_sends() {
        let (dispatch, sent) = MockDispatch::new();
        let backend = MessagingBackend::new(dispatch);
        backend.send("my-plugin", "corvid", "hello").unwrap();
        let messages = sent.lock().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0],
            ("my-plugin".into(), "corvid".into(), "hello".into())
        );
    }

    #[test]
    fn messaging_backend_error() {
        struct FailingDispatch;
        impl MessageDispatch for FailingDispatch {
            fn send(&self, _from: &str, _target: &str, _msg: &str) -> Result<(), String> {
                Err("queue full".into())
            }
        }

        let backend = MessagingBackend::new(FailingDispatch);
        let err = backend.send("plugin", "target", "msg").unwrap_err();
        assert!(err.contains("queue full"));
    }
}
