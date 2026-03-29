//! Event dispatch — routes PluginEvents to subscribing plugins.

use corvid_plugin_sdk::error::PluginEvent;
use tracing::{info, warn};

use crate::registry::PluginRegistry;

/// Dispatch a plugin event to all plugins whose event_filter matches.
///
/// Skips draining/unloaded plugins. Errors are logged and do not
/// propagate — one plugin's failure doesn't block event delivery to others.
pub async fn dispatch_event(registry: &PluginRegistry, event: &PluginEvent) {
    let event_kind = event.kind();
    let manifests = registry.list_manifests().await;

    for manifest in &manifests {
        // Check if this plugin subscribes to this event kind
        if !manifest.event_filter.contains(&event_kind) {
            continue;
        }

        let slot = match registry.get(&manifest.id).await {
            Some(s) => s,
            None => continue,
        };

        if !slot.is_active() {
            info!(
                plugin_id = %manifest.id,
                event = %event_kind,
                "skipping event dispatch — plugin not active"
            );
            continue;
        }

        // Acquire a call guard for the event handler
        let _guard = match slot.try_acquire() {
            Some(g) => g,
            None => {
                warn!(
                    plugin_id = %manifest.id,
                    "failed to acquire call guard for event dispatch"
                );
                continue;
            }
        };

        // Event dispatch to WASM would happen here via the instance.
        // For now we log the dispatch — full WASM event calling comes
        // when we integrate with the per-plugin Store instances.
        info!(
            plugin_id = %manifest.id,
            event = %event_kind,
            "dispatching event to plugin"
        );
    }
}

/// Dispatch an event and return the count of plugins that received it.
pub async fn dispatch_event_counted(registry: &PluginRegistry, event: &PluginEvent) -> usize {
    let event_kind = event.kind();
    let manifests = registry.list_manifests().await;
    let mut count = 0;

    for manifest in &manifests {
        if !manifest.event_filter.contains(&event_kind) {
            continue;
        }

        let slot = match registry.get(&manifest.id).await {
            Some(s) => s,
            None => continue,
        };

        if slot.is_active() {
            if let Some(_guard) = slot.try_acquire() {
                count += 1;
                info!(
                    plugin_id = %manifest.id,
                    event = %event_kind,
                    "dispatching event to plugin"
                );
            }
        }
    }

    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_plugin_sdk::error::EventKind;

    #[test]
    fn event_kind_matching() {
        let event = PluginEvent::AgentMessage {
            from: "alice".into(),
            content: serde_json::json!({"text": "hello"}),
        };
        assert_eq!(event.kind(), EventKind::AgentMessage);

        let filter = vec![EventKind::AgentMessage, EventKind::ScheduledTick];
        assert!(filter.contains(&event.kind()));

        let event2 = PluginEvent::AlgoTransaction {
            txid: "abc123".into(),
        };
        assert!(!filter.contains(&event2.kind()));
    }
}
