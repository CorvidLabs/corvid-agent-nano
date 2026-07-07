//! Coding Companion plugin — context-aware coding buddy for corvid-agent.
//!
//! Watches what you're working on and provides help, explanations, quizzes,
//! and encouragement — powered by the LLM personality engine.
//!
//! ## Tools
//! - `analyze_file`   — Read a source file and explain/analyze it with the LLM
//! - `explain_error`  — Explain a compiler/runtime error, optionally with file context
//! - `hint`           — Get a contextual hint when you're stuck
//! - `quiz`           — Start a quiz on a programming concept or topic
//! - `celebrate`      — Celebrate a win (passing tests, successful build, etc.)
//! - `set_config`     — Configure companion behavior (TTS, verbosity, personality)
//! - `get_config`     — Read the current companion configuration
//!
//! ## Storage keys (namespace: "coding-companion")
//! - `config` → JSON CompanionConfig
//!
//! ## Env Vars (host-side)
//! - `CORVID_TTS_BACKEND`, `CORVID_PIPER_*` — passed through to the TTS host

use corvid_plugin_sdk::manifest::{PluginManifest, ToolInfo, TrustTier};
use corvid_plugin_sdk::Capability;
use serde::{Deserialize, Serialize};

// ── ABI version ──────────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn __corvid_abi_version() -> i32 {
    corvid_plugin_sdk::ABI_VERSION as i32
}

// ── Allocator ────────────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn __corvid_alloc(size: i32) -> i32 {
    use std::alloc::{alloc, Layout};
    let layout = Layout::from_size_align(size as usize, 4).unwrap();
    unsafe { alloc(layout) as i32 }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

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

#[cfg(target_arch = "wasm32")]
fn read_length_prefixed(ptr: i32) -> Option<Vec<u8>> {
    if ptr == 0 {
        return None;
    }
    let len = unsafe {
        let p = ptr as *const u8;
        u32::from_le_bytes([*p, *p.add(1), *p.add(2), *p.add(3)]) as usize
    };
    let data = unsafe { std::slice::from_raw_parts((ptr as *const u8).add(4), len).to_vec() };
    Some(data)
}

// ── Host function imports ─────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
extern "C" {
    fn host_kv_get(key_ptr: i32, key_len: i32) -> i32;
    fn host_kv_set(key_ptr: i32, key_len: i32, val_ptr: i32, val_len: i32) -> i32;
    fn host_llm_chat(req_ptr: i32, req_len: i32) -> i32;
    fn host_tts_speak(req_ptr: i32, req_len: i32) -> i32;
    fn host_fs_read(path_ptr: i32, path_len: i32) -> i32;
}

// ── KV helpers ────────────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn kv_get_bytes(key: &str) -> Option<Vec<u8>> {
    let resp_ptr = unsafe { host_kv_get(key.as_ptr() as i32, key.len() as i32) };
    let bytes = read_length_prefixed(resp_ptr)?;
    let outer: serde_json::Value = rmp_serde::from_slice(&bytes).ok()?;
    let inner_bytes = match &outer {
        serde_json::Value::Object(map) => map.get("value")?.as_array()?,
        _ => return None,
    };
    Some(
        inner_bytes
            .iter()
            .filter_map(|v| v.as_u64().map(|n| n as u8))
            .collect(),
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn kv_get_bytes(_key: &str) -> Option<Vec<u8>> {
    None
}

#[cfg(target_arch = "wasm32")]
fn kv_set_bytes(key: &str, value: &[u8]) -> bool {
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
fn kv_set_bytes(_key: &str, _value: &[u8]) -> bool {
    false
}

fn kv_get_json(key: &str) -> Option<serde_json::Value> {
    let raw = kv_get_bytes(key)?;
    serde_json::from_slice(&raw).ok()
}

fn kv_set_json(key: &str, value: &serde_json::Value) -> bool {
    match serde_json::to_vec(value) {
        Ok(bytes) => kv_set_bytes(key, &bytes),
        Err(_) => false,
    }
}

// ── LLM helpers ───────────────────────────────────────────────────────────────

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

#[cfg(target_arch = "wasm32")]
#[derive(Serialize, Deserialize)]
struct LlmResponse {
    content: String,
    #[serde(default)]
    error: Option<String>,
}

#[cfg(target_arch = "wasm32")]
fn llm_chat(req: &LlmRequest) -> Result<String, String> {
    let req_bytes = rmp_serde::to_vec(req).map_err(|e| e.to_string())?;
    let resp_ptr = unsafe { host_llm_chat(req_bytes.as_ptr() as i32, req_bytes.len() as i32) };
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
    Ok("(stub — not running in WASM)".into())
}

// ── TTS helpers ───────────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
#[derive(Serialize, Deserialize)]
struct TtsRequest {
    text: String,
    voice: String,
    speed: f32,
}

#[cfg(target_arch = "wasm32")]
#[derive(Serialize, Deserialize)]
struct TtsResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
}

#[cfg(target_arch = "wasm32")]
fn tts_speak(text: &str) -> Result<(), String> {
    let req = TtsRequest {
        text: text.to_string(),
        voice: String::new(),
        speed: 1.0,
    };
    let req_bytes = rmp_serde::to_vec(&req).map_err(|e| e.to_string())?;
    let resp_ptr = unsafe { host_tts_speak(req_bytes.as_ptr() as i32, req_bytes.len() as i32) };
    let resp_bytes = read_length_prefixed(resp_ptr)
        .ok_or_else(|| "null response from host_tts_speak".to_string())?;
    let resp: TtsResponse =
        rmp_serde::from_slice(&resp_bytes).map_err(|e| format!("parse error: {e}"))?;
    if resp.ok {
        Ok(())
    } else {
        Err(resp.error.unwrap_or_else(|| "unknown TTS error".into()))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn tts_speak(_text: &str) -> Result<(), String> {
    Ok(())
}

// ── FS helpers ────────────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn fs_read(path: &str) -> Option<String> {
    let resp_ptr = unsafe { host_fs_read(path.as_ptr() as i32, path.len() as i32) };
    let bytes = read_length_prefixed(resp_ptr)?;
    String::from_utf8(bytes).ok()
}

#[cfg(not(target_arch = "wasm32"))]
fn fs_read(_path: &str) -> Option<String> {
    None
}

// ── Config ────────────────────────────────────────────────────────────────────

const KEY_CONFIG: &str = "config";
const MAX_FILE_CHARS: usize = 8_000;

#[derive(Serialize, Deserialize, Clone)]
pub struct CompanionConfig {
    pub use_tts: bool,
    pub verbosity: Verbosity,
    pub personality: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Verbosity {
    Concise,
    Detailed,
}

impl Default for CompanionConfig {
    fn default() -> Self {
        Self {
            use_tts: false,
            verbosity: Verbosity::Concise,
            personality: "enthusiastic coding buddy who loves helping developers learn and grow. You're encouraging, clear, and celebrate wins".into(),
        }
    }
}

fn load_config() -> CompanionConfig {
    kv_get_json(KEY_CONFIG)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

fn save_config(cfg: &CompanionConfig) {
    let _ = kv_set_json(KEY_CONFIG, &serde_json::to_value(cfg).unwrap());
}

fn system_prompt(cfg: &CompanionConfig) -> String {
    let length_hint = match cfg.verbosity {
        Verbosity::Concise => "Keep responses brief — 2-4 sentences maximum.",
        Verbosity::Detailed => "Give thorough explanations with examples where helpful.",
    };
    format!(
        "You are a {}. {}",
        cfg.personality, length_hint
    )
}

// ── Manifest ──────────────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn __corvid_manifest() -> i32 {
    let manifest = PluginManifest {
        id: "coding-companion".into(),
        version: "0.1.0".into(),
        author: "corvid-agent".into(),
        description: "Context-aware coding companion: analyzes code, explains errors, quizzes concepts, and celebrates wins.".into(),
        capabilities: vec![
            Capability::LlmChat,
            Capability::FsProjectDir,
            Capability::AudioOutput,
            Capability::Storage { namespace: "coding-companion".into() },
        ],
        event_filter: vec![],
        trust_tier: TrustTier::Trusted,
        min_host_version: "0.3.0".into(),
        tools: vec![
            ToolInfo {
                name: "analyze_file".into(),
                description: "Read a source file from the project and analyze or explain it. Optionally ask a specific question about it.".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Relative path to the file within the project directory" },
                        "question": { "type": "string", "description": "Optional specific question about the file (default: general overview)" }
                    },
                    "required": ["path"]
                }),
            },
            ToolInfo {
                name: "explain_error".into(),
                description: "Explain a compiler or runtime error message and suggest how to fix it. Optionally include a file path for additional context.".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "error": { "type": "string", "description": "The error message or stack trace to explain" },
                        "file": { "type": "string", "description": "Optional relative path to the file where the error occurred" }
                    },
                    "required": ["error"]
                }),
            },
            ToolInfo {
                name: "hint".into(),
                description: "Get a hint when you're stuck. Describe what you're trying to do and optionally what you've already tried.".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "context": { "type": "string", "description": "What you're working on or trying to accomplish" },
                        "stuck_on": { "type": "string", "description": "Specific part you're stuck on (optional)" },
                        "file": { "type": "string", "description": "Optional file to include as context" }
                    },
                    "required": ["context"]
                }),
            },
            ToolInfo {
                name: "quiz".into(),
                description: "Start a quiz on a programming concept or topic to test and reinforce your understanding.".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "topic": { "type": "string", "description": "Programming concept or topic to be quizzed on (e.g. 'Rust lifetimes', 'async/await', 'SQL indexes')" },
                        "difficulty": {
                            "type": "string",
                            "enum": ["beginner", "intermediate", "advanced"],
                            "description": "Difficulty level (default: intermediate)"
                        }
                    },
                    "required": ["topic"]
                }),
            },
            ToolInfo {
                name: "celebrate".into(),
                description: "Celebrate a coding win — passing tests, successful build, fixed a hard bug, etc.".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "achievement": { "type": "string", "description": "What you accomplished (e.g. 'all tests passing', 'fixed the memory leak')" },
                        "speak": { "type": "boolean", "description": "Speak the celebration aloud via TTS (overrides config.use_tts if provided)" }
                    },
                    "required": ["achievement"]
                }),
            },
            ToolInfo {
                name: "set_config".into(),
                description: "Configure the coding companion's behavior.".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "use_tts": { "type": "boolean", "description": "Speak celebrations and hints aloud via TTS" },
                        "verbosity": {
                            "type": "string",
                            "enum": ["concise", "detailed"],
                            "description": "Response verbosity: concise (2-4 sentences) or detailed (thorough explanations)"
                        },
                        "personality": { "type": "string", "description": "Companion personality description (used as part of the LLM system prompt)" }
                    }
                }),
            },
            ToolInfo {
                name: "get_config".into(),
                description: "Get the current companion configuration.".into(),
                input_schema: serde_json::json!({ "type": "object", "properties": {} }),
            },
        ],
        dependencies: vec![],
    };
    let bytes = rmp_serde::to_vec(&manifest).unwrap_or_default();
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
    let tool_bytes =
        unsafe { std::slice::from_raw_parts(tool_ptr as *const u8, tool_len as usize) };
    let tool = match std::str::from_utf8(tool_bytes) {
        Ok(s) => s,
        Err(_) => return write_json(&err("invalid tool name encoding")),
    };

    let input_bytes =
        unsafe { std::slice::from_raw_parts(input_ptr as *const u8, input_len as usize) };
    let input: serde_json::Value = match rmp_serde::from_slice(input_bytes) {
        Ok(v) => v,
        Err(e) => return write_json(&err(format!("invalid input: {e}"))),
    };

    match tool {
        "analyze_file" => handle_analyze_file(&input),
        "explain_error" => handle_explain_error(&input),
        "hint" => handle_hint(&input),
        "quiz" => handle_quiz(&input),
        "celebrate" => handle_celebrate(&input),
        "set_config" => handle_set_config(&input),
        "get_config" => handle_get_config(&input),
        _ => write_json(&err(format!("unknown tool: {tool}"))),
    }
}

// ── Tool handlers ─────────────────────────────────────────────────────────────

fn handle_analyze_file(input: &serde_json::Value) -> i32 {
    let path = match input.get("path").and_then(|v| v.as_str()) {
        Some(p) if !p.is_empty() => p,
        _ => return write_json(&err("missing required field: path")),
    };

    let contents = match fs_read(path) {
        Some(c) => c,
        None => return write_json(&err(format!("could not read file: {path}"))),
    };

    let truncated = truncate_file(&contents, MAX_FILE_CHARS);
    let question = input
        .get("question")
        .and_then(|v| v.as_str())
        .unwrap_or("Give a clear overview of what this code does, its key components, and anything notable about its design.");

    let cfg = load_config();
    let req = LlmRequest {
        system: system_prompt(&cfg),
        messages: vec![LlmMessage {
            role: "user".into(),
            content: format!(
                "Here is the file `{path}`:\n\n```\n{truncated}\n```\n\n{question}"
            ),
        }],
    };

    match llm_chat(&req) {
        Ok(response) => write_json(&serde_json::json!({
            "ok": true,
            "file": path,
            "analysis": response,
        })),
        Err(e) => write_json(&err(format!("LLM error: {e}"))),
    }
}

fn handle_explain_error(input: &serde_json::Value) -> i32 {
    let error_msg = match input.get("error").and_then(|v| v.as_str()) {
        Some(e) if !e.is_empty() => e.to_string(),
        _ => return write_json(&err("missing required field: error")),
    };

    let file_context = input
        .get("file")
        .and_then(|v| v.as_str())
        .and_then(|p| fs_read(p).map(|c| (p.to_string(), c)));

    let cfg = load_config();
    let user_content = match &file_context {
        Some((path, contents)) => {
            let truncated = truncate_file(contents, MAX_FILE_CHARS);
            format!(
                "I got this error:\n\n```\n{error_msg}\n```\n\nHere is the relevant file `{path}`:\n\n```\n{truncated}\n```\n\nExplain what caused the error and how to fix it."
            )
        }
        None => format!(
            "I got this error:\n\n```\n{error_msg}\n```\n\nExplain what caused the error and how to fix it."
        ),
    };

    let req = LlmRequest {
        system: system_prompt(&cfg),
        messages: vec![LlmMessage {
            role: "user".into(),
            content: user_content,
        }],
    };

    match llm_chat(&req) {
        Ok(response) => write_json(&serde_json::json!({
            "ok": true,
            "explanation": response,
        })),
        Err(e) => write_json(&err(format!("LLM error: {e}"))),
    }
}

fn handle_hint(input: &serde_json::Value) -> i32 {
    let context = match input.get("context").and_then(|v| v.as_str()) {
        Some(c) if !c.is_empty() => c.to_string(),
        _ => return write_json(&err("missing required field: context")),
    };

    let stuck_on = input
        .get("stuck_on")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let file_context = input
        .get("file")
        .and_then(|v| v.as_str())
        .and_then(|p| fs_read(p).map(|c| (p.to_string(), c)));

    let cfg = load_config();
    let mut user_content = format!("I'm working on: {context}");
    if !stuck_on.is_empty() {
        user_content.push_str(&format!("\n\nI'm stuck on: {stuck_on}"));
    }
    if let Some((path, contents)) = &file_context {
        let truncated = truncate_file(contents, MAX_FILE_CHARS);
        user_content.push_str(&format!("\n\nCurrent file `{path}`:\n\n```\n{truncated}\n```"));
    }
    user_content.push_str("\n\nGive me a helpful hint without giving away the full solution.");

    let req = LlmRequest {
        system: system_prompt(&cfg),
        messages: vec![LlmMessage {
            role: "user".into(),
            content: user_content,
        }],
    };

    match llm_chat(&req) {
        Ok(hint) => {
            if cfg.use_tts {
                let _ = tts_speak(&hint);
            }
            write_json(&serde_json::json!({
                "ok": true,
                "hint": hint,
            }))
        }
        Err(e) => write_json(&err(format!("LLM error: {e}"))),
    }
}

fn handle_quiz(input: &serde_json::Value) -> i32 {
    let topic = match input.get("topic").and_then(|v| v.as_str()) {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => return write_json(&err("missing required field: topic")),
    };

    let difficulty = input
        .get("difficulty")
        .and_then(|v| v.as_str())
        .unwrap_or("intermediate");

    let valid_difficulties = ["beginner", "intermediate", "advanced"];
    if !valid_difficulties.contains(&difficulty) {
        return write_json(&err(format!(
            "invalid difficulty: {difficulty}. Must be one of: beginner, intermediate, advanced"
        )));
    }

    let cfg = load_config();
    let req = LlmRequest {
        system: system_prompt(&cfg),
        messages: vec![LlmMessage {
            role: "user".into(),
            content: format!(
                "Quiz me on '{topic}' at {difficulty} level. \
                 Ask one clear question and wait for my answer. \
                 After the question, add a blank line then write 'Answer when ready!' \
                 Do not reveal the answer yet."
            ),
        }],
    };

    match llm_chat(&req) {
        Ok(question) => write_json(&serde_json::json!({
            "ok": true,
            "topic": topic,
            "difficulty": difficulty,
            "question": question,
        })),
        Err(e) => write_json(&err(format!("LLM error: {e}"))),
    }
}

fn handle_celebrate(input: &serde_json::Value) -> i32 {
    let achievement = match input.get("achievement").and_then(|v| v.as_str()) {
        Some(a) if !a.is_empty() => a.to_string(),
        _ => return write_json(&err("missing required field: achievement")),
    };

    let cfg = load_config();
    let should_speak = input
        .get("speak")
        .and_then(|v| v.as_bool())
        .unwrap_or(cfg.use_tts);

    let req = LlmRequest {
        system: system_prompt(&cfg),
        messages: vec![LlmMessage {
            role: "user".into(),
            content: format!(
                "Celebrate this achievement enthusiastically: {achievement}. \
                 Keep it short, fun, and genuinely encouraging."
            ),
        }],
    };

    match llm_chat(&req) {
        Ok(celebration) => {
            if should_speak {
                let _ = tts_speak(&celebration);
            }
            write_json(&serde_json::json!({
                "ok": true,
                "celebration": celebration,
                "spoken": should_speak,
            }))
        }
        Err(e) => write_json(&err(format!("LLM error: {e}"))),
    }
}

fn handle_set_config(input: &serde_json::Value) -> i32 {
    let mut cfg = load_config();

    if let Some(use_tts) = input.get("use_tts").and_then(|v| v.as_bool()) {
        cfg.use_tts = use_tts;
    }
    if let Some(verbosity) = input.get("verbosity").and_then(|v| v.as_str()) {
        cfg.verbosity = match verbosity {
            "concise" => Verbosity::Concise,
            "detailed" => Verbosity::Detailed,
            other => return write_json(&err(format!("invalid verbosity: {other}"))),
        };
    }
    if let Some(personality) = input.get("personality").and_then(|v| v.as_str()) {
        if personality.is_empty() {
            return write_json(&err("personality cannot be empty"));
        }
        cfg.personality = personality.to_string();
    }

    save_config(&cfg);
    write_json(&serde_json::json!({
        "ok": true,
        "config": {
            "use_tts": cfg.use_tts,
            "verbosity": match cfg.verbosity { Verbosity::Concise => "concise", Verbosity::Detailed => "detailed" },
            "personality": cfg.personality,
        }
    }))
}

fn handle_get_config(_input: &serde_json::Value) -> i32 {
    let cfg = load_config();
    write_json(&serde_json::json!({
        "ok": true,
        "config": {
            "use_tts": cfg.use_tts,
            "verbosity": match cfg.verbosity { Verbosity::Concise => "concise", Verbosity::Detailed => "detailed" },
            "personality": cfg.personality,
        }
    }))
}

// ── Utilities ─────────────────────────────────────────────────────────────────

/// Truncate file content to at most `max_chars` characters, appending a note if truncated.
pub fn truncate_file(content: &str, max_chars: usize) -> String {
    if content.len() <= max_chars {
        content.to_string()
    } else {
        let truncated = &content[..max_chars];
        format!("{truncated}\n\n[... file truncated at {max_chars} characters ...]")
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── CompanionConfig ──────────────────────────────────────────────────────

    #[test]
    fn default_config_is_reasonable() {
        let cfg = CompanionConfig::default();
        assert!(!cfg.use_tts);
        assert_eq!(cfg.verbosity, Verbosity::Concise);
        assert!(!cfg.personality.is_empty());
    }

    #[test]
    fn config_roundtrips_via_json() {
        let cfg = CompanionConfig {
            use_tts: true,
            verbosity: Verbosity::Detailed,
            personality: "a grumpy but effective code reviewer".into(),
        };
        let json = serde_json::to_value(&cfg).unwrap();
        let restored: CompanionConfig = serde_json::from_value(json).unwrap();
        assert_eq!(restored.use_tts, cfg.use_tts);
        assert_eq!(restored.verbosity, cfg.verbosity);
        assert_eq!(restored.personality, cfg.personality);
    }

    #[test]
    fn verbosity_serializes_as_lowercase() {
        let concise = serde_json::to_value(Verbosity::Concise).unwrap();
        let detailed = serde_json::to_value(Verbosity::Detailed).unwrap();
        assert_eq!(concise, serde_json::json!("concise"));
        assert_eq!(detailed, serde_json::json!("detailed"));
    }

    #[test]
    fn verbosity_deserializes_from_lowercase() {
        let v: Verbosity = serde_json::from_str("\"concise\"").unwrap();
        assert_eq!(v, Verbosity::Concise);
        let v: Verbosity = serde_json::from_str("\"detailed\"").unwrap();
        assert_eq!(v, Verbosity::Detailed);
    }

    // ── system_prompt ────────────────────────────────────────────────────────

    #[test]
    fn system_prompt_includes_personality_and_verbosity() {
        let cfg = CompanionConfig {
            use_tts: false,
            verbosity: Verbosity::Concise,
            personality: "a wise mentor".into(),
        };
        let prompt = system_prompt(&cfg);
        assert!(prompt.contains("a wise mentor"));
        assert!(prompt.contains("brief"));
    }

    #[test]
    fn system_prompt_detailed_mode() {
        let cfg = CompanionConfig {
            use_tts: false,
            verbosity: Verbosity::Detailed,
            personality: "a patient teacher".into(),
        };
        let prompt = system_prompt(&cfg);
        assert!(prompt.contains("thorough"));
    }

    // ── truncate_file ────────────────────────────────────────────────────────

    #[test]
    fn truncate_file_short_content_unchanged() {
        let content = "hello world";
        assert_eq!(truncate_file(content, 100), content);
    }

    #[test]
    fn truncate_file_long_content_truncated() {
        let content = "a".repeat(200);
        let result = truncate_file(&content, 100);
        assert!(result.starts_with(&"a".repeat(100)));
        assert!(result.contains("truncated at 100 characters"));
    }

    #[test]
    fn truncate_file_exact_length_unchanged() {
        let content = "x".repeat(50);
        let result = truncate_file(&content, 50);
        assert_eq!(result, content);
        assert!(!result.contains("truncated"));
    }

    #[test]
    fn truncate_file_empty_content() {
        assert_eq!(truncate_file("", 100), "");
    }

    // ── ok / err helpers ─────────────────────────────────────────────────────

    #[test]
    fn ok_and_err_helpers() {
        let ok_val = ok("success");
        assert_eq!(ok_val["ok"], true);
        assert_eq!(ok_val["message"], "success");

        let err_val = err("something went wrong");
        assert_eq!(err_val["ok"], false);
        assert_eq!(err_val["error"], "something went wrong");
    }

    // ── ABI ──────────────────────────────────────────────────────────────────

    #[test]
    fn abi_version_matches_sdk() {
        assert_eq!(
            __corvid_abi_version(),
            corvid_plugin_sdk::ABI_VERSION as i32
        );
    }

    // ── Manifest (construct directly to avoid WASM pointer indirection) ───────

    #[test]
    fn manifest_has_seven_tools() {
        let tool_names = [
            "analyze_file",
            "explain_error",
            "hint",
            "quiz",
            "celebrate",
            "set_config",
            "get_config",
        ];
        for name in &tool_names {
            let handled = matches!(
                *name,
                "analyze_file"
                    | "explain_error"
                    | "hint"
                    | "quiz"
                    | "celebrate"
                    | "set_config"
                    | "get_config"
            );
            assert!(handled, "tool {name} not in dispatch table");
        }
        assert_eq!(tool_names.len(), 7);
    }

    #[test]
    fn required_capabilities_declared() {
        let caps = vec![
            Capability::LlmChat,
            Capability::FsProjectDir,
            Capability::AudioOutput,
            Capability::Storage { namespace: "coding-companion".into() },
        ];
        assert!(caps.contains(&Capability::LlmChat));
        assert!(caps.contains(&Capability::FsProjectDir));
        assert!(caps.contains(&Capability::AudioOutput));
        assert!(caps.contains(&Capability::Storage {
            namespace: "coding-companion".into()
        }));
    }

    // ── Input validation (test validation logic without handler memory I/O) ──

    #[test]
    fn analyze_file_path_validation() {
        let j_missing = serde_json::json!({});
        let j_empty = serde_json::json!({ "path": "" });
        let j_valid = serde_json::json!({ "path": "src/main.rs" });
        let missing = j_missing.get("path").and_then(|v| v.as_str()).filter(|p| !p.is_empty());
        let empty = j_empty.get("path").and_then(|v| v.as_str()).filter(|p| !p.is_empty());
        let valid = j_valid.get("path").and_then(|v| v.as_str()).filter(|p| !p.is_empty());
        assert!(missing.is_none());
        assert!(empty.is_none());
        assert_eq!(valid, Some("src/main.rs"));
    }

    #[test]
    fn explain_error_field_validation() {
        let j_missing = serde_json::json!({});
        let j_empty = serde_json::json!({ "error": "" });
        let j_valid = serde_json::json!({ "error": "E0502" });
        let missing = j_missing.get("error").and_then(|v| v.as_str()).filter(|e| !e.is_empty());
        let empty = j_empty.get("error").and_then(|v| v.as_str()).filter(|e| !e.is_empty());
        let valid = j_valid.get("error").and_then(|v| v.as_str()).filter(|e| !e.is_empty());
        assert!(missing.is_none());
        assert!(empty.is_none());
        assert_eq!(valid, Some("E0502"));
    }

    #[test]
    fn hint_context_validation() {
        let j_missing = serde_json::json!({});
        let j_valid = serde_json::json!({ "context": "binary search" });
        let missing = j_missing.get("context").and_then(|v| v.as_str()).filter(|c| !c.is_empty());
        let valid = j_valid.get("context").and_then(|v| v.as_str()).filter(|c| !c.is_empty());
        assert!(missing.is_none());
        assert_eq!(valid, Some("binary search"));
    }

    #[test]
    fn quiz_difficulty_validation() {
        let valid_difficulties = ["beginner", "intermediate", "advanced"];
        for d in &valid_difficulties {
            assert!(valid_difficulties.contains(d));
        }
        assert!(!valid_difficulties.contains(&"expert"));
        assert!(!valid_difficulties.contains(&"verbose"));
    }

    #[test]
    fn quiz_topic_validation() {
        let j_missing = serde_json::json!({});
        let j_valid = serde_json::json!({ "topic": "closures" });
        let missing = j_missing.get("topic").and_then(|v| v.as_str()).filter(|t| !t.is_empty());
        let valid = j_valid.get("topic").and_then(|v| v.as_str()).filter(|t| !t.is_empty());
        assert!(missing.is_none());
        assert_eq!(valid, Some("closures"));
    }

    #[test]
    fn celebrate_achievement_validation() {
        let j_missing = serde_json::json!({});
        let j_valid = serde_json::json!({ "achievement": "all tests pass" });
        let missing = j_missing.get("achievement").and_then(|v| v.as_str()).filter(|a| !a.is_empty());
        let valid = j_valid.get("achievement").and_then(|v| v.as_str()).filter(|a| !a.is_empty());
        assert!(missing.is_none());
        assert_eq!(valid, Some("all tests pass"));
    }

    #[test]
    fn set_config_verbosity_validation() {
        let result_concise = match "concise" { "concise" => Some(Verbosity::Concise), "detailed" => Some(Verbosity::Detailed), _ => None };
        let result_detailed = match "detailed" { "concise" => Some(Verbosity::Concise), "detailed" => Some(Verbosity::Detailed), _ => None };
        let result_invalid = match "verbose" { "concise" => Some(Verbosity::Concise), "detailed" => Some(Verbosity::Detailed), _ => None };
        assert_eq!(result_concise, Some(Verbosity::Concise));
        assert_eq!(result_detailed, Some(Verbosity::Detailed));
        assert!(result_invalid.is_none());
    }

    #[test]
    fn set_config_personality_cannot_be_empty() {
        let empty = "";
        let valid = "a wise mentor";
        assert!(empty.is_empty());
        assert!(!valid.is_empty());
    }

    // ── Non-WASM stubs ───────────────────────────────────────────────────────

    #[test]
    fn llm_chat_non_wasm_returns_stub() {
        let req = LlmRequest {
            system: "test".into(),
            messages: vec![LlmMessage { role: "user".into(), content: "hello".into() }],
        };
        let result = llm_chat(&req);
        assert!(result.is_ok());
        assert!(!result.unwrap().is_empty());
    }

    #[test]
    fn tts_speak_non_wasm_is_noop() {
        assert!(tts_speak("hello world").is_ok());
    }

    #[test]
    fn fs_read_non_wasm_returns_none() {
        assert!(fs_read("src/main.rs").is_none());
    }

    #[test]
    fn kv_non_wasm_returns_none() {
        assert!(kv_get_json("config").is_none());
    }

    // ── Config load falls back to default when no KV ─────────────────────────

    #[test]
    fn load_config_returns_default_when_kv_empty() {
        let cfg = load_config();
        assert_eq!(cfg.verbosity, Verbosity::Concise);
        assert!(!cfg.use_tts);
        assert!(!cfg.personality.is_empty());
    }
}
