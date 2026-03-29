use crate::error::PluginError;

/// Host-provided scoped key-value storage.
pub trait StorageService: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, PluginError>;
    fn set(&self, key: &str, value: &[u8]) -> Result<(), PluginError>;
}

/// Host-provided allowlisted outbound HTTP.
pub trait HttpService: Send + Sync {
    fn get(&self, url: &str) -> Result<Vec<u8>, PluginError>;
    fn post(&self, url: &str, body: &[u8]) -> Result<Vec<u8>, PluginError>;
}

/// Host-provided read-only database access.
pub trait DbReadService: Send + Sync {
    fn query(&self, sql: &str) -> Result<serde_json::Value, PluginError>;
}

/// Host-provided sandboxed filesystem read.
pub trait FsReadService: Send + Sync {
    fn read(&self, path: &str) -> Result<Vec<u8>, PluginError>;
}

/// Host-provided Algorand chain read access.
pub trait AlgoReadService: Send + Sync {
    fn app_state(&self, app_id: u64, key: &str) -> Result<serde_json::Value, PluginError>;
}

/// Host-provided agent message bus.
pub trait MessagingService: Send + Sync {
    fn send(&self, target: &str, message: &str) -> Result<(), PluginError>;
}
