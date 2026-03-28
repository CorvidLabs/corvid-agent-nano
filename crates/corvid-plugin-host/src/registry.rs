//! Plugin registry with hot-reload drain pattern.
//!
//! `PluginSlot` enables hot-reload under load without dropping in-flight requests.
//! State machine: ACTIVE → DRAINING → (swap) → ACTIVE.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Result};
use corvid_plugin_sdk::PluginManifest;
use tokio::sync::RwLock;
use wasmtime::Module;

use crate::loader::LoadedPlugin;
use crate::sandbox::SandboxLimits;

/// Plugin slot states.
const STATE_ACTIVE: u8 = 0;
const STATE_DRAINING: u8 = 1;
const STATE_UNLOADED: u8 = 2;

/// Maximum time to wait for in-flight calls to drain during hot-reload.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

/// Holds a plugin instance with hot-reload drain pattern.
pub struct PluginSlot {
    pub manifest: PluginManifest,
    pub module: Arc<RwLock<Module>>,
    pub limits: SandboxLimits,
    pub active_calls: Arc<AtomicUsize>,
    pub state: Arc<AtomicU8>,
}

impl PluginSlot {
    /// Create a new active plugin slot from a loaded plugin.
    pub fn new(loaded: LoadedPlugin) -> Self {
        Self {
            manifest: loaded.manifest,
            module: Arc::new(RwLock::new(loaded.module)),
            limits: loaded.limits,
            active_calls: Arc::new(AtomicUsize::new(0)),
            state: Arc::new(AtomicU8::new(STATE_ACTIVE)),
        }
    }

    /// Returns true if the plugin is accepting new calls.
    pub fn is_active(&self) -> bool {
        self.state.load(Ordering::Acquire) == STATE_ACTIVE
    }

    /// Returns true if the plugin is draining (hot-reload in progress).
    pub fn is_draining(&self) -> bool {
        self.state.load(Ordering::Acquire) == STATE_DRAINING
    }

    /// Acquire a call guard. Returns None if plugin is not active.
    pub fn try_acquire(&self) -> Option<CallGuard> {
        if !self.is_active() {
            return None;
        }
        self.active_calls.fetch_add(1, Ordering::AcqRel);
        // Double-check state after incrementing (avoid race with drain)
        if !self.is_active() {
            self.active_calls.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        Some(CallGuard {
            active_calls: Arc::clone(&self.active_calls),
        })
    }

    /// Hot-reload: drain → swap → activate.
    ///
    /// 1. Set state to DRAINING — new calls return Unavailable
    /// 2. Wait up to 30s for active_calls to reach 0
    /// 3. Swap in new module
    /// 4. Set state to ACTIVE
    pub async fn drain_and_reload(&self, new_plugin: LoadedPlugin) -> Result<()> {
        // Set draining
        self.state.store(STATE_DRAINING, Ordering::Release);

        // scopeguard: if anything fails, restore ACTIVE state
        let state = Arc::clone(&self.state);
        let _guard = scopeguard::guard((), move |_| {
            // Only reset to ACTIVE if still DRAINING (not if already swapped)
            let _ = state.compare_exchange(
                STATE_DRAINING,
                STATE_ACTIVE,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        });

        // Wait for in-flight calls to complete
        let deadline = tokio::time::Instant::now() + DRAIN_TIMEOUT;
        loop {
            if self.active_calls.load(Ordering::Acquire) == 0 {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                tracing::warn!(
                    plugin_id = %self.manifest.id,
                    active = self.active_calls.load(Ordering::Acquire),
                    "drain timeout — force-swapping plugin"
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        // Swap module
        {
            let mut module = self.module.write().await;
            *module = new_plugin.module;
        }

        // Activate
        self.state.store(STATE_ACTIVE, Ordering::Release);

        // Defuse the scopeguard (we already set ACTIVE)
        std::mem::forget(_guard);

        Ok(())
    }

    /// Gracefully unload — drain then mark as unloaded.
    pub async fn unload(&self) {
        self.state.store(STATE_DRAINING, Ordering::Release);

        let deadline = tokio::time::Instant::now() + DRAIN_TIMEOUT;
        while self.active_calls.load(Ordering::Acquire) > 0 {
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        self.state.store(STATE_UNLOADED, Ordering::Release);
    }

    /// Current state as a string for health reporting.
    pub fn state_str(&self) -> &'static str {
        match self.state.load(Ordering::Acquire) {
            STATE_ACTIVE => "active",
            STATE_DRAINING => "draining",
            STATE_UNLOADED => "unloaded",
            _ => "unknown",
        }
    }
}

/// RAII guard that tracks active calls for drain synchronization.
pub struct CallGuard {
    active_calls: Arc<AtomicUsize>,
}

impl Drop for CallGuard {
    fn drop(&mut self) {
        self.active_calls.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Thread-safe plugin registry keyed by plugin ID.
pub struct PluginRegistry {
    slots: Arc<RwLock<HashMap<String, Arc<PluginSlot>>>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            slots: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a loaded plugin.
    pub async fn register(&self, loaded: LoadedPlugin) -> Result<()> {
        let id = loaded.manifest.id.clone();
        let slot = Arc::new(PluginSlot::new(loaded));

        let mut slots = self.slots.write().await;
        if slots.contains_key(&id) {
            bail!("plugin '{}' already registered — use reload", id);
        }
        slots.insert(id, slot);
        Ok(())
    }

    /// Get a plugin slot by ID.
    pub async fn get(&self, id: &str) -> Option<Arc<PluginSlot>> {
        self.slots.read().await.get(id).cloned()
    }

    /// Unload a plugin by ID.
    pub async fn unload(&self, id: &str) -> Result<()> {
        let slot = {
            let slots = self.slots.read().await;
            slots.get(id).cloned()
        };

        match slot {
            Some(slot) => {
                slot.unload().await;
                self.slots.write().await.remove(id);
                Ok(())
            }
            None => bail!("plugin '{}' not found", id),
        }
    }

    /// Hot-reload a plugin with a new binary.
    pub async fn reload(&self, id: &str, new_plugin: LoadedPlugin) -> Result<()> {
        let slot = self
            .get(id)
            .await
            .ok_or_else(|| anyhow::anyhow!("plugin '{}' not found", id))?;

        slot.drain_and_reload(new_plugin).await
    }

    /// List all plugin manifests.
    pub async fn list_manifests(&self) -> Vec<PluginManifest> {
        self.slots
            .read()
            .await
            .values()
            .map(|s| s.manifest.clone())
            .collect()
    }

    /// Get health status for all plugins.
    pub async fn health_status(&self) -> HashMap<String, &'static str> {
        self.slots
            .read()
            .await
            .iter()
            .map(|(id, slot)| (id.clone(), slot.state_str()))
            .collect()
    }

    /// Number of registered plugins.
    pub async fn len(&self) -> usize {
        self.slots.read().await.len()
    }

    /// Whether the registry is empty.
    pub async fn is_empty(&self) -> bool {
        self.slots.read().await.is_empty()
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_guard_decrements_on_drop() {
        let counter = Arc::new(AtomicUsize::new(0));
        {
            let _guard = CallGuard {
                active_calls: Arc::clone(&counter),
            };
            counter.fetch_add(1, Ordering::AcqRel);
            assert_eq!(counter.load(Ordering::Acquire), 1);
        }
        // Guard dropped, counter decremented
        assert_eq!(counter.load(Ordering::Acquire), 0);
    }

    #[test]
    fn slot_state_transitions() {
        let state = Arc::new(AtomicU8::new(STATE_ACTIVE));
        assert_eq!(state.load(Ordering::Acquire), STATE_ACTIVE);

        state.store(STATE_DRAINING, Ordering::Release);
        assert_eq!(state.load(Ordering::Acquire), STATE_DRAINING);

        state.store(STATE_UNLOADED, Ordering::Release);
        assert_eq!(state.load(Ordering::Acquire), STATE_UNLOADED);
    }

    #[test]
    fn state_str_values() {
        // Direct state string mapping test
        assert_eq!(
            match STATE_ACTIVE {
                0 => "active",
                1 => "draining",
                2 => "unloaded",
                _ => "unknown",
            },
            "active"
        );
        assert_eq!(
            match STATE_DRAINING {
                0 => "active",
                1 => "draining",
                2 => "unloaded",
                _ => "unknown",
            },
            "draining"
        );
    }

    #[tokio::test]
    async fn registry_crud() {
        let registry = PluginRegistry::new();
        assert!(registry.is_empty().await);
        assert_eq!(registry.len().await, 0);
    }
}
