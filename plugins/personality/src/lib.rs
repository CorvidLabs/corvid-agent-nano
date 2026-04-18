//! LLM personality engine plugin for corvid-agent.
//!
//! Provides a pluggable "brain" with persona configuration, conversation memory,
//! and emotion state tracking. Other plugins (coding companion, avatar) consume
//! the emotion output. Supports Claude and OpenAI as LLM providers; Ollama is
//! supported via a user-configured public URL (localhost is SSRF-blocked).
//!
//! ## Tools
//! - `personality.chat` — send a message and get a persona-flavored response
//! - `personality.configure` — set LLM provider, model, API key, base URL
//! - `personality.set-persona` — set name, traits, speech style, tone
//! - `personality.get-state` — read current emotion and active persona
//!
//! ## State
//! All state is persisted via host KV (namespaced per plugin) so it survives
//! across invocations despite the stateless WASM Store model:
//! - key `config`  → msgpack PersonalityConfig
//! - key `persona` → msgpack Persona
//! - key `emotion` → msgpack EmotionState
//! - key `history:{session_id}` → msgpack Vec<Message>

use corvid_plugin_sdk::{
    Capability, EventKind, InitContext, PluginError, PluginEvent, PluginManifest, PluginTool,
    ToolContext, TrustTier,
};
use serde::{Deserialize, Serialize};

// Re-export CorvidPlugin for macro use
use corvid_plugin_sdk::CorvidPlugin;

// ── Host function wrappers (WASM only) ───────────────────────────────────────

#[cfg(target_arch = "wasm32")]
mod host {
    use corvid_plugin_sdk::host_api;

    /// Read a value from the plugin KV store. Returns None if key not found.
    pub fn kv_get(key: &str) -> Option<Vec<u8>> {
        let key_bytes = key.as_bytes();
        let ptr =
            unsafe { host_api::host_kv_get(key_bytes.as_ptr() as i32, key_bytes.len() as i32) };
        if ptr == 0 {
            return None;
        }
        unsafe {
            let len_bytes = std::slice::from_raw_parts(ptr as *const u8, 4);
            let len =
                u32::from_le_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]]) as usize;
            let data = std::slice::from_raw_parts((ptr + 4) as *const u8, len);
            Some(data.to_vec())
        }
    }

    /// Write a value to the plugin KV store. Returns true on success.
    pub fn kv_set(key: &str, value: &[u8]) -> bool {
        let key_bytes = key.as_bytes();
        let result = unsafe {
            host_api::host_kv_set(
                key_bytes.as_ptr() as i32,
                key_bytes.len() as i32,
                value.as_ptr() as i32,
                value.len() as i32,
            )
        };
        result == 0
    }

    /// Perform an HTTP POST with custom headers. Returns the response body bytes.
    pub fn http_post(
        url: &str,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> Result<Vec<u8>, String> {
        // Encode as HttpPostRequest msgpack
        #[derive(serde::Serialize)]
        struct HttpPostRequest {
            headers: Vec<(String, String)>,
            body: Vec<u8>,
        }
        let req = HttpPostRequest { headers, body };
        let req_bytes = rmp_serde::to_vec(&req).map_err(|e| e.to_string())?;

        let url_bytes = url.as_bytes();
        let ptr = unsafe {
            host_api::host_http_post(
                url_bytes.as_ptr() as i32,
                url_bytes.len() as i32,
                req_bytes.as_ptr() as i32,
                req_bytes.len() as i32,
            )
        };

        if ptr == 0 {
            return Err("http_post returned null".into());
        }

        unsafe {
            let len_bytes = std::slice::from_raw_parts(ptr as *const u8, 4);
            let len =
                u32::from_le_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]]) as usize;
            let data = std::slice::from_raw_parts((ptr + 4) as *const u8, len);
            Ok(data.to_vec())
        }
    }
}

// Stub for non-WASM builds (tests run natively)
#[cfg(not(target_arch = "wasm32"))]
mod host {
    pub fn kv_get(_key: &str) -> Option<Vec<u8>> {
        None
    }
    pub fn kv_set(_key: &str, _value: &[u8]) -> bool {
        true
    }
    pub fn http_post(
        _url: &str,
        _headers: Vec<(String, String)>,
        _body: Vec<u8>,
    ) -> Result<Vec<u8>, String> {
        Err("http_post not available in native test builds".into())
    }
}

// ── Domain types ─────────────────────────────────────────────────────────────

/// LLM provider variant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Claude,
    OpenAI,
    /// Any provider reachable at a custom base URL (e.g. Ollama on a public host).
    Custom,
}

/// Persisted LLM configuration (stored under key `config`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalityConfig {
    pub provider: Provider,
    pub model: String,
    pub api_key: String,
    /// Base URL override — required for Custom provider, optional for Claude/OpenAI.
    pub base_url: Option<String>,
}

impl Default for PersonalityConfig {
    fn default() -> Self {
        Self {
            provider: Provider::Claude,
            model: "claude-haiku-4-5-20251001".into(),
            api_key: String::new(),
            base_url: None,
        }
    }
}

/// Persona definition (stored under key `persona`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Persona {
    pub name: String,
    pub traits: Vec<String>,
    pub speech_style: String,
    pub tone: String,
}

impl Default for Persona {
    fn default() -> Self {
        Self {
            name: "Kira".into(),
            traits: vec!["curious".into(), "playful".into(), "helpful".into()],
            speech_style: "casual and warm".into(),
            tone: "encouraging".into(),
        }
    }
}

/// Detected emotion state (stored under key `emotion`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmotionState {
    pub current: String,
    pub intensity: u8, // 0–100
}

impl Default for EmotionState {
    fn default() -> Self {
        Self {
            current: "neutral".into(),
            intensity: 50,
        }
    }
}

/// A single conversation turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String, // "user" or "assistant"
    pub content: String,
}

// ── KV helpers ───────────────────────────────────────────────────────────────

fn load_config() -> PersonalityConfig {
    host::kv_get("config")
        .and_then(|b| rmp_serde::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_config(cfg: &PersonalityConfig) {
    if let Ok(b) = rmp_serde::to_vec(cfg) {
        host::kv_set("config", &b);
    }
}

fn load_persona() -> Persona {
    host::kv_get("persona")
        .and_then(|b| rmp_serde::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_persona(p: &Persona) {
    if let Ok(b) = rmp_serde::to_vec(p) {
        host::kv_set("persona", &b);
    }
}

fn load_emotion() -> EmotionState {
    host::kv_get("emotion")
        .and_then(|b| rmp_serde::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_emotion(e: &EmotionState) {
    if let Ok(b) = rmp_serde::to_vec(e) {
        host::kv_set("emotion", &b);
    }
}

fn load_history(session_id: &str) -> Vec<Message> {
    let key = format!("history:{session_id}");
    host::kv_get(&key)
        .and_then(|b| rmp_serde::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_history(session_id: &str, history: &[Message]) {
    let key = format!("history:{session_id}");
    if let Ok(b) = rmp_serde::to_vec(history) {
        host::kv_set(&key, &b);
    }
}

// ── Emotion detection ────────────────────────────────────────────────────────

/// Detect the emotion conveyed by a response string.
///
/// Uses simple keyword/pattern matching — no extra LLM call required.
pub fn detect_emotion(text: &str) -> EmotionState {
    let lower = text.to_lowercase();

    // Order matters: more specific patterns first
    if lower.contains("haha")
        || lower.contains("lol")
        || lower.contains(":)")
        || lower.contains("😄")
        || lower.contains("😂")
    {
        return EmotionState {
            current: "happy".into(),
            intensity: 80,
        };
    }
    if lower.contains("wow")
        || lower.contains("ooh")
        || lower.contains("amazing")
        || lower.contains("!!!")
        || lower.contains("😮")
        || lower.contains("🤩")
    {
        return EmotionState {
            current: "excited".into(),
            intensity: 90,
        };
    }
    if lower.contains("hmm")
        || lower.contains("let me think")
        || lower.contains("interesting question")
        || lower.contains("🤔")
    {
        return EmotionState {
            current: "thinking".into(),
            intensity: 60,
        };
    }
    if lower.contains("unfortunately")
        || lower.contains("sorry to hear")
        || lower.contains("sigh")
        || lower.contains("😔")
    {
        return EmotionState {
            current: "sad".into(),
            intensity: 60,
        };
    }
    if lower.contains("careful")
        || lower.contains("watch out")
        || lower.contains("warning")
        || lower.contains("⚠️")
    {
        return EmotionState {
            current: "cautious".into(),
            intensity: 70,
        };
    }

    EmotionState {
        current: "neutral".into(),
        intensity: 50,
    }
}

// ── Persona system prompt ────────────────────────────────────────────────────

/// Build a system prompt from the active persona.
pub fn build_system_prompt(persona: &Persona) -> String {
    let traits = persona.traits.join(", ");
    format!(
        "You are {}. Your personality traits are: {}. \
         Your speech style is: {}. Your tone is: {}. \
         Stay in character. Be concise but warm.",
        persona.name, traits, persona.speech_style, persona.tone
    )
}

// ── LLM API callers ──────────────────────────────────────────────────────────

/// Call the Claude Messages API.
fn call_claude(
    cfg: &PersonalityConfig,
    system: &str,
    history: &[Message],
    user_message: &str,
) -> Result<String, String> {
    let base_url = cfg
        .base_url
        .as_deref()
        .unwrap_or("https://api.anthropic.com");
    let url = format!("{base_url}/v1/messages");

    let mut messages: Vec<serde_json::Value> = history
        .iter()
        .map(|m| serde_json::json!({"role": m.role, "content": m.content}))
        .collect();
    messages.push(serde_json::json!({"role": "user", "content": user_message}));

    let body = serde_json::json!({
        "model": cfg.model,
        "max_tokens": 1024,
        "system": system,
        "messages": messages,
    });

    let body_bytes = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
    let headers = vec![
        ("Content-Type".into(), "application/json".into()),
        ("x-api-key".into(), cfg.api_key.clone()),
        ("anthropic-version".into(), "2023-06-01".into()),
    ];

    let resp_bytes = host::http_post(&url, headers, body_bytes)?;

    // Response is msgpack HttpResponse { status, body }
    #[derive(Deserialize)]
    struct HttpResp {
        status: u16,
        body: Vec<u8>,
    }
    let resp: HttpResp =
        rmp_serde::from_slice(&resp_bytes).map_err(|e| format!("parse http response: {e}"))?;

    if resp.status != 200 {
        let err_text = String::from_utf8_lossy(&resp.body);
        return Err(format!("Claude API error {}: {err_text}", resp.status));
    }

    let json: serde_json::Value =
        serde_json::from_slice(&resp.body).map_err(|e| format!("parse json: {e}"))?;

    json["content"][0]["text"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| format!("unexpected Claude response shape: {json}"))
}

/// Call the OpenAI Chat Completions API.
fn call_openai(
    cfg: &PersonalityConfig,
    system: &str,
    history: &[Message],
    user_message: &str,
) -> Result<String, String> {
    let base_url = cfg
        .base_url
        .as_deref()
        .unwrap_or("https://api.openai.com");
    let url = format!("{base_url}/v1/chat/completions");

    let mut messages = vec![serde_json::json!({"role": "system", "content": system})];
    for m in history {
        messages.push(serde_json::json!({"role": m.role, "content": m.content}));
    }
    messages.push(serde_json::json!({"role": "user", "content": user_message}));

    let body = serde_json::json!({
        "model": cfg.model,
        "max_tokens": 1024,
        "messages": messages,
    });

    let body_bytes = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
    let headers = vec![
        ("Content-Type".into(), "application/json".into()),
        ("Authorization".into(), format!("Bearer {}", cfg.api_key)),
    ];

    let resp_bytes = host::http_post(&url, headers, body_bytes)?;

    #[derive(Deserialize)]
    struct HttpResp {
        status: u16,
        body: Vec<u8>,
    }
    let resp: HttpResp =
        rmp_serde::from_slice(&resp_bytes).map_err(|e| format!("parse http response: {e}"))?;

    if resp.status != 200 {
        let err_text = String::from_utf8_lossy(&resp.body);
        return Err(format!("OpenAI API error {}: {err_text}", resp.status));
    }

    let json: serde_json::Value =
        serde_json::from_slice(&resp.body).map_err(|e| format!("parse json: {e}"))?;

    json["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| format!("unexpected OpenAI response shape: {json}"))
}

/// Dispatch to the configured LLM provider.
fn call_llm(
    cfg: &PersonalityConfig,
    system: &str,
    history: &[Message],
    user_message: &str,
) -> Result<String, String> {
    match &cfg.provider {
        Provider::Claude => call_claude(cfg, system, history, user_message),
        Provider::OpenAI => call_openai(cfg, system, history, user_message),
        Provider::Custom => {
            // Custom provider uses the OpenAI-compatible API format
            call_openai(cfg, system, history, user_message)
        }
    }
}

// ── Tools ─────────────────────────────────────────────────────────────────────

/// `personality.chat` — send a message and get a persona-flavored LLM response.
struct ChatTool;

impl PluginTool for ChatTool {
    fn name(&self) -> &str {
        "personality.chat"
    }

    fn description(&self) -> &str {
        "Send a message to the personality engine and receive a response shaped by the active persona."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "message": { "type": "string", "description": "The user message" },
                "session_id": {
                    "type": "string",
                    "description": "Conversation session ID for history tracking (optional)",
                    "default": "default"
                }
            },
            "required": ["message"]
        })
    }

    fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<String, PluginError> {
        let message = input["message"]
            .as_str()
            .ok_or_else(|| PluginError::BadInput("message is required".into()))?;
        let session_id = input["session_id"].as_str().unwrap_or("default");

        let cfg = load_config();
        if cfg.api_key.is_empty() && cfg.provider != Provider::Custom {
            return Err(PluginError::Init(
                "API key not set. Call personality.configure first.".into(),
            ));
        }

        let persona = load_persona();
        let system = build_system_prompt(&persona);
        let history = load_history(session_id);

        let response = call_llm(&cfg, &system, &history, message)
            .map_err(|e| PluginError::Exec(e))?;

        let emotion = detect_emotion(&response);

        // Persist updated history (cap at 20 turns to bound KV size)
        let mut new_history = history;
        new_history.push(Message {
            role: "user".into(),
            content: message.to_string(),
        });
        new_history.push(Message {
            role: "assistant".into(),
            content: response.clone(),
        });
        if new_history.len() > 40 {
            new_history.drain(0..new_history.len() - 40);
        }
        save_history(session_id, &new_history);
        save_emotion(&emotion);

        let result = serde_json::json!({
            "response": response,
            "emotion": emotion.current,
            "emotion_intensity": emotion.intensity,
            "persona": persona.name,
        });
        Ok(serde_json::to_string(&result).unwrap_or_default())
    }
}

/// `personality.configure` — set the LLM provider and credentials.
struct ConfigureTool;

impl PluginTool for ConfigureTool {
    fn name(&self) -> &str {
        "personality.configure"
    }

    fn description(&self) -> &str {
        "Set the LLM provider (claude, openai, custom), model, API key, and optional base URL."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "provider": {
                    "type": "string",
                    "enum": ["claude", "openai", "custom"],
                    "description": "LLM provider to use"
                },
                "model": { "type": "string", "description": "Model ID (e.g. claude-haiku-4-5-20251001)" },
                "api_key": { "type": "string", "description": "API key for the provider" },
                "base_url": {
                    "type": "string",
                    "description": "Custom base URL (required for custom provider, e.g. Ollama)"
                }
            },
            "required": ["provider", "api_key"]
        })
    }

    fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<String, PluginError> {
        let provider_str = input["provider"]
            .as_str()
            .ok_or_else(|| PluginError::BadInput("provider is required".into()))?;
        let api_key = input["api_key"]
            .as_str()
            .ok_or_else(|| PluginError::BadInput("api_key is required".into()))?;

        let provider = match provider_str {
            "claude" => Provider::Claude,
            "openai" => Provider::OpenAI,
            "custom" => Provider::Custom,
            other => {
                return Err(PluginError::BadInput(format!(
                    "unknown provider: {other}. Use: claude, openai, custom"
                )))
            }
        };

        let mut cfg = load_config();
        cfg.provider = provider;
        cfg.api_key = api_key.to_string();

        if let Some(model) = input["model"].as_str() {
            cfg.model = model.to_string();
        }
        if let Some(base_url) = input["base_url"].as_str() {
            cfg.base_url = Some(base_url.to_string());
        }

        if cfg.provider == Provider::Custom && cfg.base_url.is_none() {
            return Err(PluginError::BadInput(
                "base_url is required for custom provider".into(),
            ));
        }

        save_config(&cfg);
        Ok(serde_json::json!({"ok": true, "provider": provider_str, "model": cfg.model}).to_string())
    }
}

/// `personality.set-persona` — configure the active persona.
struct SetPersonaTool;

impl PluginTool for SetPersonaTool {
    fn name(&self) -> &str {
        "personality.set-persona"
    }

    fn description(&self) -> &str {
        "Configure the agent's persona: name, traits, speech style, and tone."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Persona name (e.g. Kira)" },
                "traits": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Personality traits (e.g. [\"playful\", \"nerdy\"])"
                },
                "speech_style": {
                    "type": "string",
                    "description": "How the persona speaks (e.g. \"casual, uses emoji\")"
                },
                "tone": {
                    "type": "string",
                    "description": "Overall tone (e.g. \"encouraging\", \"sarcastic\")"
                }
            },
            "required": ["name"]
        })
    }

    fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<String, PluginError> {
        let name = input["name"]
            .as_str()
            .ok_or_else(|| PluginError::BadInput("name is required".into()))?;

        let mut persona = load_persona();
        persona.name = name.to_string();

        if let Some(traits) = input["traits"].as_array() {
            persona.traits = traits
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
        }
        if let Some(style) = input["speech_style"].as_str() {
            persona.speech_style = style.to_string();
        }
        if let Some(tone) = input["tone"].as_str() {
            persona.tone = tone.to_string();
        }

        save_persona(&persona);
        Ok(serde_json::json!({"ok": true, "persona": {"name": persona.name, "traits": persona.traits}}).to_string())
    }
}

/// `personality.get-state` — return current emotion and active persona.
struct GetStateTool;

impl PluginTool for GetStateTool {
    fn name(&self) -> &str {
        "personality.get-state"
    }

    fn description(&self) -> &str {
        "Get the current emotion state and active persona configuration."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> Result<String, PluginError> {
        let emotion = load_emotion();
        let persona = load_persona();
        let cfg = load_config();

        let result = serde_json::json!({
            "emotion": {
                "current": emotion.current,
                "intensity": emotion.intensity,
            },
            "persona": {
                "name": persona.name,
                "traits": persona.traits,
                "speech_style": persona.speech_style,
                "tone": persona.tone,
            },
            "config": {
                "provider": format!("{:?}", cfg.provider).to_lowercase(),
                "model": cfg.model,
                "has_api_key": !cfg.api_key.is_empty(),
            }
        });
        Ok(serde_json::to_string(&result).unwrap_or_default())
    }
}

// ── Plugin struct ─────────────────────────────────────────────────────────────

#[corvid_plugin_macros::corvid_plugin]
pub struct PersonalityPlugin {
    tools: Vec<Box<dyn PluginTool>>,
}

// Default populates tools so __corvid_invoke works without a prior __corvid_init call.
// The WASM host creates a fresh Store per invocation, so tools must be ready on Default.
impl Default for PersonalityPlugin {
    fn default() -> Self {
        Self {
            tools: vec![
                Box::new(ChatTool),
                Box::new(ConfigureTool),
                Box::new(SetPersonaTool),
                Box::new(GetStateTool),
            ],
        }
    }
}

impl CorvidPlugin for PersonalityPlugin {
    fn manifest() -> PluginManifest {
        PluginManifest {
            id: "personality".into(),
            version: "0.1.0".into(),
            author: "CorvidLabs".into(),
            description: "LLM personality engine with persona, emotion tracking, and conversation memory".into(),
            capabilities: vec![
                Capability::Storage {
                    namespace: "personality".into(),
                },
                Capability::Network {
                    allowlist: vec![
                        "api.anthropic.com".into(),
                        "api.openai.com".into(),
                    ],
                },
            ],
            event_filter: vec![EventKind::AgentMessage],
            trust_tier: TrustTier::Verified,
            min_host_version: "0.3.0".into(),
            tools: vec![],
            dependencies: vec![],
        }
    }

    fn tools(&self) -> &[Box<dyn PluginTool>] {
        &self.tools
    }

    fn init(&mut self, _ctx: InitContext) -> Result<(), PluginError> {
        self.tools = vec![
            Box::new(ChatTool),
            Box::new(ConfigureTool),
            Box::new(SetPersonaTool),
            Box::new(GetStateTool),
        ];
        Ok(())
    }

    fn on_event(&mut self, event: PluginEvent, _ctx: &ToolContext) -> Result<(), PluginError> {
        // Forward incoming agent messages to the chat tool for autonomous responses
        if let PluginEvent::AgentMessage { from: _, content } = event {
            if let Some(msg) = content.get("text").and_then(|v| v.as_str()) {
                let chat = ChatTool;
                let input = serde_json::json!({ "message": msg, "session_id": "agent-event" });
                let ctx = ToolContext {
                    agent_id: String::new(),
                    session_id: "agent-event".into(),
                    capabilities: PersonalityPlugin::manifest().capabilities,
                };
                let _ = chat.execute(input, &ctx);
            }
        }
        Ok(())
    }
}


// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Emotion detection ─────────────────────────────────────────────────────

    #[test]
    fn detects_happy_from_lol() {
        let e = detect_emotion("haha that's so funny lol");
        assert_eq!(e.current, "happy");
        assert!(e.intensity > 50);
    }

    #[test]
    fn detects_excited_from_wow() {
        let e = detect_emotion("Wow that's amazing!!! 🤩");
        assert_eq!(e.current, "excited");
    }

    #[test]
    fn detects_thinking_from_hmm() {
        let e = detect_emotion("Hmm, let me think about that...");
        assert_eq!(e.current, "thinking");
    }

    #[test]
    fn detects_sad_from_unfortunately() {
        let e = detect_emotion("Unfortunately that didn't work out.");
        assert_eq!(e.current, "sad");
    }

    #[test]
    fn detects_cautious_from_warning() {
        let e = detect_emotion("Warning: this could cause issues.");
        assert_eq!(e.current, "cautious");
    }

    #[test]
    fn defaults_to_neutral() {
        let e = detect_emotion("Here is the documentation for the function.");
        assert_eq!(e.current, "neutral");
        assert_eq!(e.intensity, 50);
    }

    #[test]
    fn case_insensitive_detection() {
        let e = detect_emotion("UNFORTUNATELY this broke");
        assert_eq!(e.current, "sad");
    }

    // ── System prompt ─────────────────────────────────────────────────────────

    #[test]
    fn system_prompt_includes_persona_fields() {
        let persona = Persona {
            name: "Nova".into(),
            traits: vec!["curious".into(), "sarcastic".into()],
            speech_style: "terse".into(),
            tone: "dry".into(),
        };
        let prompt = build_system_prompt(&persona);
        assert!(prompt.contains("Nova"));
        assert!(prompt.contains("curious"));
        assert!(prompt.contains("sarcastic"));
        assert!(prompt.contains("terse"));
        assert!(prompt.contains("dry"));
    }

    // ── Persona defaults ──────────────────────────────────────────────────────

    #[test]
    fn persona_default_has_name() {
        let p = Persona::default();
        assert!(!p.name.is_empty());
        assert!(!p.traits.is_empty());
    }

    // ── Config serialization ──────────────────────────────────────────────────

    #[test]
    fn config_msgpack_roundtrip() {
        let cfg = PersonalityConfig {
            provider: Provider::Claude,
            model: "claude-sonnet-4-6".into(),
            api_key: "sk-test".into(),
            base_url: None,
        };
        let packed = rmp_serde::to_vec(&cfg).unwrap();
        let unpacked: PersonalityConfig = rmp_serde::from_slice(&packed).unwrap();
        assert_eq!(unpacked.provider, Provider::Claude);
        assert_eq!(unpacked.model, "claude-sonnet-4-6");
        assert_eq!(unpacked.api_key, "sk-test");
    }

    #[test]
    fn config_with_base_url_roundtrip() {
        let cfg = PersonalityConfig {
            provider: Provider::Custom,
            model: "llama3".into(),
            api_key: String::new(),
            base_url: Some("https://ollama.example.com".into()),
        };
        let packed = rmp_serde::to_vec(&cfg).unwrap();
        let unpacked: PersonalityConfig = rmp_serde::from_slice(&packed).unwrap();
        assert_eq!(unpacked.provider, Provider::Custom);
        assert_eq!(unpacked.base_url, Some("https://ollama.example.com".into()));
    }

    // ── Emotion serialization ─────────────────────────────────────────────────

    #[test]
    fn emotion_msgpack_roundtrip() {
        let e = EmotionState {
            current: "excited".into(),
            intensity: 90,
        };
        let packed = rmp_serde::to_vec(&e).unwrap();
        let unpacked: EmotionState = rmp_serde::from_slice(&packed).unwrap();
        assert_eq!(unpacked.current, "excited");
        assert_eq!(unpacked.intensity, 90);
    }

    // ── Tool schema validation ────────────────────────────────────────────────

    #[test]
    fn chat_tool_schema_requires_message() {
        let schema = ChatTool.input_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("message")));
    }

    #[test]
    fn configure_tool_schema_has_provider_enum() {
        let schema = ConfigureTool.input_schema();
        let provider_enum = schema["properties"]["provider"]["enum"].as_array().unwrap();
        assert!(provider_enum
            .iter()
            .any(|v| v.as_str() == Some("claude")));
    }

    #[test]
    fn set_persona_tool_requires_name() {
        let schema = SetPersonaTool.input_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("name")));
    }

    // ── Tool names ────────────────────────────────────────────────────────────

    #[test]
    fn tool_names_are_correct() {
        assert_eq!(ChatTool.name(), "personality.chat");
        assert_eq!(ConfigureTool.name(), "personality.configure");
        assert_eq!(SetPersonaTool.name(), "personality.set-persona");
        assert_eq!(GetStateTool.name(), "personality.get-state");
    }

    // ── execute: configure validation ─────────────────────────────────────────

    #[test]
    fn configure_rejects_unknown_provider() {
        let tool = ConfigureTool;
        let ctx = ToolContext {
            agent_id: "test".into(),
            session_id: "s1".into(),
            capabilities: vec![],
        };
        let result = tool.execute(
            serde_json::json!({"provider": "grok", "api_key": "k"}),
            &ctx,
        );
        assert!(matches!(result, Err(PluginError::BadInput(_))));
    }

    #[test]
    fn configure_rejects_custom_without_base_url() {
        let tool = ConfigureTool;
        let ctx = ToolContext {
            agent_id: "test".into(),
            session_id: "s1".into(),
            capabilities: vec![],
        };
        let result = tool.execute(
            serde_json::json!({"provider": "custom", "api_key": "ignored"}),
            &ctx,
        );
        assert!(matches!(result, Err(PluginError::BadInput(_))));
    }

    // ── execute: set-persona ──────────────────────────────────────────────────

    #[test]
    fn set_persona_rejects_missing_name() {
        let tool = SetPersonaTool;
        let ctx = ToolContext {
            agent_id: "test".into(),
            session_id: "s1".into(),
            capabilities: vec![],
        };
        let result = tool.execute(serde_json::json!({"traits": ["cool"]}), &ctx);
        assert!(matches!(result, Err(PluginError::BadInput(_))));
    }

    #[test]
    fn set_persona_accepts_minimal_input() {
        let tool = SetPersonaTool;
        let ctx = ToolContext {
            agent_id: "test".into(),
            session_id: "s1".into(),
            capabilities: vec![],
        };
        let result = tool.execute(serde_json::json!({"name": "Nova"}), &ctx);
        // In native test builds, kv_set is a no-op stub, so this should succeed
        assert!(result.is_ok());
        let json: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert_eq!(json["ok"], true);
    }

    // ── execute: get-state ────────────────────────────────────────────────────

    #[test]
    fn get_state_returns_defaults() {
        let tool = GetStateTool;
        let ctx = ToolContext {
            agent_id: "test".into(),
            session_id: "s1".into(),
            capabilities: vec![],
        };
        let result = tool.execute(serde_json::json!({}), &ctx);
        assert!(result.is_ok());
        let json: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
        assert!(json["emotion"]["current"].is_string());
        assert!(json["persona"]["name"].is_string());
    }

    // ── Manifest ──────────────────────────────────────────────────────────────

    #[test]
    fn manifest_id_is_valid() {
        let m = PersonalityPlugin::manifest();
        assert_eq!(m.id, "personality");
        assert!(!m.capabilities.is_empty());
        assert!(m.capabilities.iter().any(|c| matches!(c, Capability::Storage { .. })));
        assert!(m.capabilities.iter().any(|c| matches!(c, Capability::Network { .. })));
    }

    #[test]
    fn manifest_msgpack_roundtrip() {
        let m = PersonalityPlugin::manifest();
        let packed = rmp_serde::to_vec(&m).unwrap();
        let unpacked: PluginManifest = rmp_serde::from_slice(&packed).unwrap();
        assert_eq!(unpacked.id, "personality");
        assert_eq!(unpacked.version, "0.1.0");
    }
}
