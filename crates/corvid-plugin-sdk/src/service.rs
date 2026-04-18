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

/// A single message in an LLM conversation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LlmMessage {
    pub role: String,
    pub content: String,
}

/// Request sent to the host LLM service.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct LlmRequest {
    /// Conversation messages (user/assistant turns).
    pub messages: Vec<LlmMessage>,
    /// Optional system prompt override. If empty, the host uses its default.
    #[serde(default)]
    pub system: String,
}

/// Response from the host LLM service.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct LlmResponse {
    pub content: String,
    #[serde(default)]
    pub error: Option<String>,
}

/// Host-provided LLM chat service. Provider and API key are managed by the host.
pub trait LlmService: Send + Sync {
    fn chat(&self, req: &LlmRequest) -> Result<LlmResponse, PluginError>;
}

/// Request sent to the host TTS service.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct TtsRequest {
    /// Text to synthesize and speak aloud.
    pub text: String,
    /// Voice model name (e.g. "en_US-lessac-medium"). Empty = use default.
    #[serde(default)]
    pub voice: String,
    /// Speech rate multiplier. 1.0 = normal, 0.5 = half speed, 2.0 = double.
    #[serde(default = "default_speed")]
    pub speed: f32,
}

fn default_speed() -> f32 {
    1.0
}

/// Response from the host TTS service.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct TtsResponse {
    pub ok: bool,
    #[serde(default)]
    pub error: Option<String>,
}

/// Host-provided text-to-speech output service.
pub trait TtsService: Send + Sync {
    fn speak(&self, req: &TtsRequest) -> Result<TtsResponse, PluginError>;
    fn list_voices(&self) -> Result<Vec<String>, PluginError>;
}
