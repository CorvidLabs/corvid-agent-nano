//! LLM Personality Engine — pluggable LLM backend with persona config,
//! emotion state tracking, and conversation memory for corvid-agent.
//!
//! ## Tools
//! - `chat` — send a message; receive a response in the persona's voice
//! - `set_persona` — configure name, traits, speech_style, tone
//! - `get_persona` — read the current persona config
//! - `get_emotion` — read the current emotion state
//! - `clear_history` — wipe conversation history for a session
//!
//! ## Storage keys (namespace: "llm-personality")
//! - `persona`                  → JSON PersonaConfig
//! - `history:{session_id}`     → JSON Vec<HistoryMessage>
//! - `emotion`                  → String (current emotion label)
//!
//! ## Emotion states
//! happy | excited | teasing | thinking | neutral | confused
//!
//! Emotion is inferred from simple keyword heuristics on the LLM response.

use corvid_plugin_sdk::manifest::{PluginManifest, ToolInfo, TrustTier};
use corvid_plugin_sdk::Capability;
use serde::{Deserialize, Serialize};

// ── ABI version ─────────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn __corvid_abi_version() -> i32 {
    corvid_plugin_sdk::ABI_VERSION as i32
}

// ── Allocator ───────────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn __corvid_alloc(size: i32) -> i32 {
    use std::alloc::{alloc, Layout};
    let layout = Layout::from_size_align(size as usize, 4).unwrap();
    unsafe { alloc(layout) as i32 }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn write_response(data: &[u8]) -> i32 {
    let total = 4 + data.len();
    let ptr = __corvid_alloc(total as i32);
    if ptr == 0 {
        return 0;
    }
    let buf = unsafe { std::slice::from_raw_parts_mut(ptr as *mut u8, total) };
    buf[..4].copy_from_slice(&(data.len() as u32).to_le_bytes());
    buf[4..].copy_from_slice(data);
    ptr
}

fn write_json(v: &serde_json::Value) -> i32 {
    let bytes = rmp_serde::to_vec(v).unwrap_or_default();
    write_response(&bytes)
}

fn ok(msg: impl Into<String>) -> serde_json::Value {
    serde_json::json!({ "ok": true, "message": msg.into() })
}

fn err(msg: impl Into<String>) -> serde_json::Value {
    serde_json::json!({ "ok": false, "error": msg.into() })
}

// ── Host function imports ────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
extern "C" {
    fn host_kv_get(key_ptr: i32, key_len: i32) -> i32;
    fn host_kv_set(key_ptr: i32, key_len: i32, val_ptr: i32, val_len: i32) -> i32;
    fn host_llm_chat(req_ptr: i32, req_len: i32) -> i32;
}

// ── KV helpers ───────────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn kv_get(key: &str) -> Option<Vec<u8>> {
    let resp_ptr = unsafe { host_kv_get(key.as_ptr() as i32, key.len() as i32) };
    if resp_ptr == 0 {
        return None;
    }
    read_length_prefixed(resp_ptr)
}

#[cfg(not(target_arch = "wasm32"))]
fn kv_get(_key: &str) -> Option<Vec<u8>> {
    None
}

#[cfg(target_arch = "wasm32")]
fn kv_set(key: &str, value: &[u8]) -> bool {
    let result = unsafe {
        host_kv_set(
            key.as_ptr() as i32,
            key.len() as i32,
            value.as_ptr() as i32,
            value.len() as i32,
        )
    };
    result == 0
}

#[cfg(not(target_arch = "wasm32"))]
fn kv_set(_key: &str, _value: &[u8]) -> bool {
    false
}

/// Read a msgpack-encoded JSON value from KV storage.
fn kv_get_json(key: &str) -> Option<serde_json::Value> {
    let bytes = kv_get(key)?;
    // The host returns msgpack-encoded response; unwrap the outer response envelope
    let outer: serde_json::Value = rmp_serde::from_slice(&bytes).ok()?;
    // host_kv_get returns { "value": <msgpack bytes as array> } or { "error": ... }
    let inner_bytes = match &outer {
        serde_json::Value::Object(map) => map.get("value")?.as_array()?,
        _ => return None,
    };
    let raw: Vec<u8> = inner_bytes
        .iter()
        .filter_map(|v| v.as_u64().map(|n| n as u8))
        .collect();
    serde_json::from_slice(&raw).ok()
}

fn kv_set_json(key: &str, value: &serde_json::Value) -> bool {
    match serde_json::to_vec(value) {
        Ok(bytes) => kv_set(key, &bytes),
        Err(_) => false,
    }
}

/// Read a length-prefixed buffer from WASM memory.
fn read_length_prefixed(ptr: i32) -> Option<Vec<u8>> {
    if ptr == 0 {
        return None;
    }
    let len = unsafe {
        let p = ptr as *const u8;
        u32::from_le_bytes([*p, *p.add(1), *p.add(2), *p.add(3)]) as usize
    };
    let data =
        unsafe { std::slice::from_raw_parts((ptr as *const u8).add(4), len).to_vec() };
    Some(data)
}

// ── LLM helper ───────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
struct LlmMessage {
    role: String,
    content: String,
}

#[derive(Serialize, Deserialize)]
struct LlmRequest {
    messages: Vec<LlmMessage>,
    system: String,
}

#[derive(Serialize, Deserialize)]
struct LlmResponse {
    content: String,
    #[serde(default)]
    error: Option<String>,
}

#[cfg(target_arch = "wasm32")]
fn llm_chat(req: &LlmRequest) -> Result<String, String> {
    let req_bytes = rmp_serde::to_vec(req).map_err(|e| e.to_string())?;
    let resp_ptr =
        unsafe { host_llm_chat(req_bytes.as_ptr() as i32, req_bytes.len() as i32) };

    let resp_bytes = read_length_prefixed(resp_ptr)
        .ok_or_else(|| "null response from host_llm_chat".to_string())?;

    let resp: LlmResponse =
        rmp_serde::from_slice(&resp_bytes).map_err(|e| format!("parse error: {e}"))?;

    if let Some(e) = resp.error {
        return Err(e);
    }

    Ok(resp.content)
}

#[cfg(not(target_arch = "wasm32"))]
fn llm_chat(_req: &LlmRequest) -> Result<String, String> {
    Ok("(stub response — not running in WASM)".into())
}

// ── Persona config ────────────────────────────────────────────────────────────

const KEY_PERSONA: &str = "persona";
const KEY_EMOTION: &str = "emotion";

#[derive(Serialize, Deserialize, Clone)]
struct PersonaConfig {
    name: String,
    traits: Vec<String>,
    speech_style: String,
    tone: String,
}

impl Default for PersonaConfig {
    fn default() -> Self {
        Self {
            name: "Nano".into(),
            traits: vec!["helpful".into(), "concise".into(), "friendly".into()],
            speech_style: "clear and direct".into(),
            tone: "encouraging".into(),
        }
    }
}

impl PersonaConfig {
    fn to_system_prompt(&self) -> String {
        format!(
            "You are {name}, an AI assistant with the following traits: {traits}. \
             Your speech style is {speech}. Your tone is {tone}. \
             Stay in character throughout the conversation.",
            name = self.name,
            traits = self.traits.join(", "),
            speech = self.speech_style,
            tone = self.tone,
        )
    }
}

fn load_persona() -> PersonaConfig {
    kv_get_json(KEY_PERSONA)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

fn save_persona(persona: &PersonaConfig) {
    let _ = kv_set_json(KEY_PERSONA, &serde_json::to_value(persona).unwrap());
}

// ── Emotion tracking ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Emotion {
    Happy,
    Excited,
    Teasing,
    Thinking,
    Confused,
    Neutral,
}

impl Emotion {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Happy => "happy",
            Self::Excited => "excited",
            Self::Teasing => "teasing",
            Self::Thinking => "thinking",
            Self::Confused => "confused",
            Self::Neutral => "neutral",
        }
    }

    /// Infer emotion from LLM response text using keyword heuristics.
    fn infer_from(text: &str) -> Self {
        let lower = text.to_lowercase();
        if lower.contains("actually")
            || lower.contains("interesting")
            || lower.contains("let me think")
            || lower.contains("hmm")
        {
            return Self::Thinking;
        }
        if lower.contains("error")
            || lower.contains("sorry")
            || lower.contains("i don't understand")
            || lower.contains("unclear")
        {
            return Self::Confused;
        }
        if lower.contains("haha")
            || lower.contains("😄")
            || lower.contains("funny")
            || lower.contains("joking")
        {
            return Self::Teasing;
        }
        if lower.contains("!")
            && (lower.contains("great")
                || lower.contains("awesome")
                || lower.contains("amazing"))
        {
            return Self::Excited;
        }
        if lower.contains("happy")
            || lower.contains("glad")
            || lower.contains("wonderful")
            || lower.contains("pleased")
        {
            return Self::Happy;
        }
        Self::Neutral
    }
}

fn load_emotion() -> String {
    kv_get_json(KEY_EMOTION)
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "neutral".into())
}

fn save_emotion(emotion: &Emotion) {
    let _ = kv_set_json(KEY_EMOTION, &serde_json::Value::String(emotion.as_str().into()));
}

// ── Conversation history ──────────────────────────────────────────────────────

const MAX_HISTORY: usize = 20;

fn history_key(session_id: &str) -> String {
    format!("history:{session_id}")
}

fn load_history(session_id: &str) -> Vec<LlmMessage> {
    kv_get_json(&history_key(session_id))
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

fn save_history(session_id: &str, history: &[LlmMessage]) {
    let _ = kv_set_json(
        &history_key(session_id),
        &serde_json::to_value(history).unwrap(),
    );
}

// ── Manifest ─────────────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn __corvid_manifest() -> i32 {
    let manifest = PluginManifest {
        id: "llm-personality".into(),
        version: "0.1.0".into(),
        author: "corvid".into(),
        description: "LLM personality engine — persona config, emotion tracking, conversation memory".into(),
        capabilities: vec![
            Capability::Storage { namespace: "llm-personality".into() },
            Capability::LlmChat,
        ],
        event_filter: vec![],
        trust_tier: TrustTier::Trusted,
        min_host_version: "0.3.0".into(),
        tools: vec![
            ToolInfo {
                name: "chat".into(),
                description: "Send a message to the persona and receive a response. Maintains conversation history per session.".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "message": { "type": "string", "description": "User message" },
                        "session_id": {
                            "type": "string",
                            "description": "Session identifier for conversation memory (default: 'default')"
                        }
                    },
                    "required": ["message"]
                }),
            },
            ToolInfo {
                name: "set_persona".into(),
                description: "Configure the persona. Fields: name, traits (array), speech_style, tone.".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "traits": { "type": "array", "items": { "type": "string" } },
                        "speech_style": { "type": "string" },
                        "tone": { "type": "string" }
                    }
                }),
            },
            ToolInfo {
                name: "get_persona".into(),
                description: "Get the current persona configuration.".into(),
                input_schema: serde_json::json!({ "type": "object" }),
            },
            ToolInfo {
                name: "get_emotion".into(),
                description: "Get the current emotion state (happy|excited|teasing|thinking|confused|neutral).".into(),
                input_schema: serde_json::json!({ "type": "object" }),
            },
            ToolInfo {
                name: "clear_history".into(),
                description: "Clear conversation history for a session.".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string" }
                    }
                }),
            },
        ],
        dependencies: vec![],
    };

    let bytes = rmp_serde::to_vec(&manifest).unwrap();
    write_response(&bytes)
}

// ── Tool dispatch ─────────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn __corvid_invoke(
    tool_ptr: i32,
    tool_len: i32,
    input_ptr: i32,
    input_len: i32,
) -> i32 {
    let tool_name = unsafe {
        let slice = std::slice::from_raw_parts(tool_ptr as *const u8, tool_len as usize);
        std::str::from_utf8(slice).unwrap_or("unknown")
    };

    let input_bytes =
        unsafe { std::slice::from_raw_parts(input_ptr as *const u8, input_len as usize) };

    let input: serde_json::Value = rmp_serde::from_slice(input_bytes).unwrap_or_default();

    let result = match tool_name {
        "chat" => tool_chat(&input),
        "set_persona" => tool_set_persona(&input),
        "get_persona" => tool_get_persona(),
        "get_emotion" => tool_get_emotion(),
        "clear_history" => tool_clear_history(&input),
        _ => err(format!("unknown tool: {tool_name}")),
    };

    write_json(&result)
}

// ── Tool implementations ──────────────────────────────────────────────────────

fn tool_chat(input: &serde_json::Value) -> serde_json::Value {
    let message = match input.get("message").and_then(|v| v.as_str()) {
        Some(m) => m,
        None => return err("missing required field: message"),
    };
    let session_id = input
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let persona = load_persona();
    let mut history = load_history(session_id);

    // Append user message to history
    history.push(LlmMessage {
        role: "user".into(),
        content: message.into(),
    });

    // Trim history to MAX_HISTORY (keep most recent)
    if history.len() > MAX_HISTORY {
        let drain = history.len() - MAX_HISTORY;
        history.drain(0..drain);
    }

    let req = LlmRequest {
        messages: history.clone(),
        system: persona.to_system_prompt(),
    };

    match llm_chat(&req) {
        Ok(response) => {
            // Append assistant response to history
            history.push(LlmMessage {
                role: "assistant".into(),
                content: response.clone(),
            });

            // Save updated history
            save_history(session_id, &history);

            // Detect and save emotion
            let emotion = Emotion::infer_from(&response);
            save_emotion(&emotion);

            serde_json::json!({
                "response": response,
                "emotion": emotion.as_str(),
                "persona": persona.name,
            })
        }
        Err(e) => err(format!("LLM error: {e}")),
    }
}

fn tool_set_persona(input: &serde_json::Value) -> serde_json::Value {
    let mut persona = load_persona();

    if let Some(name) = input.get("name").and_then(|v| v.as_str()) {
        persona.name = name.into();
    }
    if let Some(traits) = input.get("traits").and_then(|v| v.as_array()) {
        persona.traits = traits
            .iter()
            .filter_map(|t| t.as_str().map(String::from))
            .collect();
    }
    if let Some(style) = input.get("speech_style").and_then(|v| v.as_str()) {
        persona.speech_style = style.into();
    }
    if let Some(tone) = input.get("tone").and_then(|v| v.as_str()) {
        persona.tone = tone.into();
    }

    save_persona(&persona);
    serde_json::json!({
        "ok": true,
        "persona": {
            "name": persona.name,
            "traits": persona.traits,
            "speech_style": persona.speech_style,
            "tone": persona.tone,
        }
    })
}

fn tool_get_persona() -> serde_json::Value {
    let persona = load_persona();
    serde_json::json!({
        "name": persona.name,
        "traits": persona.traits,
        "speech_style": persona.speech_style,
        "tone": persona.tone,
    })
}

fn tool_get_emotion() -> serde_json::Value {
    serde_json::json!({ "emotion": load_emotion() })
}

fn tool_clear_history(input: &serde_json::Value) -> serde_json::Value {
    let session_id = input
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let empty: Vec<LlmMessage> = vec![];
    save_history(session_id, &empty);

    ok(format!("history cleared for session '{session_id}'"))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persona_default() {
        let p = PersonaConfig::default();
        assert_eq!(p.name, "Nano");
        assert!(!p.traits.is_empty());
    }

    #[test]
    fn persona_system_prompt_contains_name() {
        let p = PersonaConfig {
            name: "Kira".into(),
            traits: vec!["playful".into()],
            speech_style: "casual".into(),
            tone: "sarcastic".into(),
        };
        let prompt = p.to_system_prompt();
        assert!(prompt.contains("Kira"));
        assert!(prompt.contains("playful"));
        assert!(prompt.contains("sarcastic"));
    }

    #[test]
    fn emotion_infer_thinking() {
        assert_eq!(Emotion::infer_from("Hmm, let me think about this..."), Emotion::Thinking);
        assert_eq!(Emotion::infer_from("Actually, that's interesting."), Emotion::Thinking);
    }

    #[test]
    fn emotion_infer_confused() {
        assert_eq!(Emotion::infer_from("Sorry, I don't understand what you mean."), Emotion::Confused);
    }

    #[test]
    fn emotion_infer_excited() {
        assert_eq!(Emotion::infer_from("That's awesome! Great job!"), Emotion::Excited);
    }

    #[test]
    fn emotion_infer_teasing() {
        assert_eq!(Emotion::infer_from("Haha, just joking 😄"), Emotion::Teasing);
    }

    #[test]
    fn emotion_infer_neutral() {
        assert_eq!(Emotion::infer_from("The answer is 42."), Emotion::Neutral);
    }

    #[test]
    fn history_trim() {
        // If we have more than MAX_HISTORY messages, oldest are dropped
        let mut history: Vec<LlmMessage> = (0..25)
            .map(|i| LlmMessage {
                role: "user".into(),
                content: format!("msg {i}"),
            })
            .collect();

        if history.len() > MAX_HISTORY {
            let drain = history.len() - MAX_HISTORY;
            history.drain(0..drain);
        }

        assert_eq!(history.len(), MAX_HISTORY);
        assert_eq!(history[0].content, "msg 5"); // oldest 5 dropped
    }

    #[test]
    fn tool_chat_missing_message() {
        let result = tool_chat(&serde_json::json!({}));
        assert_eq!(result["ok"], false);
        assert!(result["error"].as_str().unwrap().contains("missing"));
    }

    #[test]
    fn tool_set_persona_partial_update() {
        // Calling set_persona with only 'name' should not wipe other fields
        let result = tool_set_persona(&serde_json::json!({"name": "Kira"}));
        // In non-WASM, kv_get returns None so it uses default and updates name
        assert_eq!(result["persona"]["name"], "Kira");
        // traits should still be there (from default)
        assert!(result["persona"]["traits"].is_array());
    }

    #[test]
    fn tool_get_emotion_default() {
        // In non-WASM context, kv_get returns None → "neutral"
        let result = tool_get_emotion();
        assert_eq!(result["emotion"], "neutral");
    }

    #[test]
    fn tool_clear_history_default_session() {
        let result = tool_clear_history(&serde_json::json!({}));
        assert_eq!(result["ok"], true);
        assert!(result["message"].as_str().unwrap().contains("default"));
    }

    #[test]
    fn manifest_serialization() {
        // Verify the manifest can be msgpack-serialized without panicking
        // (tests the __corvid_manifest logic without WASM)
        let manifest = PluginManifest {
            id: "llm-personality".into(),
            version: "0.1.0".into(),
            author: "corvid".into(),
            description: "test".into(),
            capabilities: vec![
                Capability::Storage { namespace: "llm-personality".into() },
                Capability::LlmChat,
            ],
            event_filter: vec![],
            trust_tier: TrustTier::Trusted,
            min_host_version: "0.3.0".into(),
            tools: vec![],
            dependencies: vec![],
        };
        let bytes = rmp_serde::to_vec(&manifest).unwrap();
        let decoded: PluginManifest = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(decoded.id, "llm-personality");
        assert_eq!(decoded.capabilities.len(), 2);
    }
}
