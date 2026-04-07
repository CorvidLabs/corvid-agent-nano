//! Plugin-scoped state storage.
//!
//! Each plugin gets its own namespace — no plugin can read or write
//! another plugin's data.

use std::collections::HashMap;

use serde_json::Value;

/// In-memory state store with per-plugin namespaces.
#[derive(Debug, Default)]
pub struct StateStore {
    namespaces: HashMap<String, HashMap<String, Value>>,
}

impl StateStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a value in a plugin's namespace.
    pub fn set(&mut self, plugin: &str, key: &str, value: Value) {
        self.namespaces
            .entry(plugin.to_string())
            .or_default()
            .insert(key.to_string(), value);
    }

    /// Get a value from a plugin's namespace.
    pub fn get(&self, plugin: &str, key: &str) -> Option<&Value> {
        self.namespaces.get(plugin)?.get(key)
    }

    /// Get a snapshot of a plugin's entire namespace.
    pub fn snapshot(&self, plugin: &str) -> HashMap<String, Value> {
        self.namespaces
            .get(plugin)
            .cloned()
            .unwrap_or_default()
    }

    /// Remove a value from a plugin's namespace.
    pub fn remove(&mut self, plugin: &str, key: &str) -> Option<Value> {
        self.namespaces.get_mut(plugin)?.remove(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_get() {
        let mut store = StateStore::new();
        store.set("hub", "last_task_id", Value::String("abc".into()));
        assert_eq!(
            store.get("hub", "last_task_id"),
            Some(&Value::String("abc".into()))
        );
    }

    #[test]
    fn namespace_isolation() {
        let mut store = StateStore::new();
        store.set("hub", "key", Value::Bool(true));
        store.set("auto-reply", "key", Value::Bool(false));
        assert_eq!(store.get("hub", "key"), Some(&Value::Bool(true)));
        assert_eq!(store.get("auto-reply", "key"), Some(&Value::Bool(false)));
    }

    #[test]
    fn get_missing_returns_none() {
        let store = StateStore::new();
        assert!(store.get("hub", "nope").is_none());
    }

    #[test]
    fn snapshot_returns_copy() {
        let mut store = StateStore::new();
        store.set("p", "a", Value::from(1));
        store.set("p", "b", Value::from(2));
        let snap = store.snapshot("p");
        assert_eq!(snap.len(), 2);
        assert_eq!(snap["a"], Value::from(1));
    }

    #[test]
    fn snapshot_missing_plugin() {
        let store = StateStore::new();
        assert!(store.snapshot("missing").is_empty());
    }

    #[test]
    fn remove_value() {
        let mut store = StateStore::new();
        store.set("p", "key", Value::from(42));
        assert_eq!(store.remove("p", "key"), Some(Value::from(42)));
        assert!(store.get("p", "key").is_none());
    }

    #[test]
    fn remove_missing() {
        let mut store = StateStore::new();
        assert!(store.remove("p", "nope").is_none());
    }
}
