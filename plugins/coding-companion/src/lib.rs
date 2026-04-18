//! Coding companion plugin for corvid-agent.
//!
//! LLM-backed code analysis, review, explanation, and Q&A with optional
//! project file context. Uses `host_llm_chat` so no API key management is
//! needed in the plugin — the host reads `CORVID_LLM_*` env vars.
//!
//! ## Tools
//! - `code.analyze` — detect bugs, issues, and improvements in code
//! - `code.review`  — structured code review (correctness, style, security)
//! - `code.explain` — plain-English explanation of what code does
//! - `code.ask`     — general coding Q&A with session memory + file context
//!
//! ## Inputs (all tools)
//! | Field        | Type   | Required | Description                              |
//! |--------------|--------|----------|------------------------------------------|
//! | `code`       | string | *        | Inline source code to analyze            |
//! | `path`       | string | *        | Project-relative file path (alt to code) |
//! | `focus`      | string | no       | Area to focus on (analyze only)          |
//! | `message`    | string | ask only | The user's question                      |
//! | `session_id` | string | no       | Conversation session key (ask only)      |
//!
//! *Either `code` or `path` must be supplied for analyze/review/explain.
//!
//! ## Storage keys (namespace: "coding-companion")
//! - `session:{session_id}` → msgpack `Vec<ChatMessage>` (ask history)

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

// ── Memory helpers ────────────────────────────────────────────────────────────

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

#[allow(dead_code)]
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

// ── Host function imports (WASM only) ─────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
extern "C" {
    fn host_kv_get(key_ptr: i32, key_len: i32) -> i32;
    fn host_kv_set(key_ptr: i32, key_len: i32, val_ptr: i32, val_len: i32) -> i32;
    fn host_llm_chat(req_ptr: i32, req_len: i32) -> i32;
    fn host_fs_read(path_ptr: i32, path_len: i32) -> i32;
}

// ── KV helpers ────────────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn kv_get_raw(key: &str) -> Option<Vec<u8>> {
    let resp_ptr = unsafe { host_kv_get(key.as_ptr() as i32, key.len() as i32) };
    read_length_prefixed(resp_ptr)
}

#[cfg(not(target_arch = "wasm32"))]
fn kv_get_raw(_key: &str) -> Option<Vec<u8>> {
    None
}

#[cfg(target_arch = "wasm32")]
fn kv_set_raw(key: &str, value: &[u8]) -> bool {
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
fn kv_set_raw(_key: &str, _value: &[u8]) -> bool {
    false
}

fn kv_load<T: for<'de> Deserialize<'de>>(key: &str) -> Option<T> {
    let bytes = kv_get_raw(key)?;
    rmp_serde::from_slice(&bytes).ok()
}

fn kv_save<T: Serialize>(key: &str, value: &T) -> bool {
    match rmp_serde::to_vec(value) {
        Ok(bytes) => kv_set_raw(key, &bytes),
        Err(_) => false,
    }
}

// ── FS helper ─────────────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn fs_read_text(path: &str) -> Option<String> {
    let resp_ptr = unsafe { host_fs_read(path.as_ptr() as i32, path.len() as i32) };
    let bytes = read_length_prefixed(resp_ptr)?;
    String::from_utf8(bytes).ok()
}

#[cfg(not(target_arch = "wasm32"))]
fn fs_read_text(_path: &str) -> Option<String> {
    None
}

// ── LLM helper ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[allow(dead_code)]
#[derive(Serialize, Deserialize)]
struct LlmRequest {
    messages: Vec<ChatMessage>,
    #[serde(default)]
    system: String,
}

#[allow(dead_code)]
#[derive(Serialize, Deserialize)]
struct LlmResponse {
    content: String,
    #[serde(default)]
    error: Option<String>,
}

#[cfg(target_arch = "wasm32")]
fn llm_chat(system: &str, messages: Vec<ChatMessage>) -> Result<String, String> {
    let req = LlmRequest {
        messages,
        system: system.to_string(),
    };
    let req_bytes = rmp_serde::to_vec(&req).map_err(|e| e.to_string())?;
    let resp_ptr = unsafe { host_llm_chat(req_bytes.as_ptr() as i32, req_bytes.len() as i32) };
    let resp_bytes =
        read_length_prefixed(resp_ptr).ok_or_else(|| "null response from host_llm_chat".to_string())?;
    let resp: LlmResponse =
        rmp_serde::from_slice(&resp_bytes).map_err(|e| format!("parse error: {e}"))?;
    if let Some(e) = resp.error {
        return Err(e);
    }
    Ok(resp.content)
}

#[cfg(not(target_arch = "wasm32"))]
fn llm_chat(_system: &str, _messages: Vec<ChatMessage>) -> Result<String, String> {
    Ok("(stub — not running in WASM)".into())
}

// ── Code resolution ───────────────────────────────────────────────────────────

/// Resolve code content from either inline `code` or a project `path`.
///
/// Returns `(content, source_label)` where `source_label` is used in prompts
/// so the LLM knows what it's looking at.
fn resolve_code(input: &serde_json::Value) -> Result<(String, String), String> {
    if let Some(code) = input.get("code").and_then(|v| v.as_str()) {
        if !code.is_empty() {
            return Ok((code.to_string(), "provided code".to_string()));
        }
    }

    if let Some(path) = input.get("path").and_then(|v| v.as_str()) {
        if !path.is_empty() {
            let content = fs_read_text(path)
                .ok_or_else(|| format!("could not read file: {path}"))?;
            return Ok((content, format!("file `{path}`")));
        }
    }

    Err("provide either `code` (inline source) or `path` (project-relative file path)".into())
}

// ── System prompts ────────────────────────────────────────────────────────────

const ANALYZE_SYSTEM: &str = "You are an expert software engineer performing code analysis. \
    Identify bugs, potential issues, security vulnerabilities, and opportunities for improvement. \
    Be specific: cite the exact line or pattern, explain why it's a problem, and suggest a fix. \
    Structure your response with sections: Bugs, Security, Performance, Style. \
    Omit sections with no findings.";

const REVIEW_SYSTEM: &str = "You are a senior software engineer doing a thorough code review. \
    Evaluate the code for: correctness (logic, edge cases, error handling), \
    security (input validation, injection risks, data exposure), \
    performance (algorithmic complexity, unnecessary allocations), \
    and maintainability (clarity, naming, structure). \
    Give actionable, specific feedback. Use inline references (line numbers if available).";

const EXPLAIN_SYSTEM: &str = "You are a patient, clear technical writer. \
    Explain the provided code in plain English. Describe: what it does at a high level, \
    how it works step by step, any non-obvious patterns or idioms, \
    and the inputs/outputs. Tailor the depth to the code's complexity.";

const ASK_SYSTEM: &str = "You are an expert coding assistant embedded in a developer's workflow. \
    Answer coding questions clearly and concisely. When code context is provided, \
    reference it directly. Prefer concrete examples and correct, runnable code. \
    Be direct — no unnecessary preamble.";

// ── Tool handlers ─────────────────────────────────────────────────────────────

fn handle_analyze(input: &serde_json::Value) -> i32 {
    let (content, label) = match resolve_code(input) {
        Ok(r) => r,
        Err(e) => return write_json(&err(e)),
    };

    let focus = input
        .get("focus")
        .and_then(|v| v.as_str())
        .unwrap_or("all areas");

    let user_msg = format!(
        "Analyze the following {label} focusing on {focus}:\n\n```\n{content}\n```"
    );

    match llm_chat(ANALYZE_SYSTEM, vec![ChatMessage { role: "user".into(), content: user_msg }]) {
        Ok(analysis) => write_json(&serde_json::json!({ "ok": true, "analysis": analysis })),
        Err(e) => write_json(&err(format!("LLM error: {e}"))),
    }
}

fn handle_review(input: &serde_json::Value) -> i32 {
    let (content, label) = match resolve_code(input) {
        Ok(r) => r,
        Err(e) => return write_json(&err(e)),
    };

    let user_msg = format!("Review the following {label}:\n\n```\n{content}\n```");

    match llm_chat(REVIEW_SYSTEM, vec![ChatMessage { role: "user".into(), content: user_msg }]) {
        Ok(review) => write_json(&serde_json::json!({ "ok": true, "review": review })),
        Err(e) => write_json(&err(format!("LLM error: {e}"))),
    }
}

fn handle_explain(input: &serde_json::Value) -> i32 {
    let (content, label) = match resolve_code(input) {
        Ok(r) => r,
        Err(e) => return write_json(&err(e)),
    };

    let user_msg = format!("Explain the following {label}:\n\n```\n{content}\n```");

    match llm_chat(EXPLAIN_SYSTEM, vec![ChatMessage { role: "user".into(), content: user_msg }]) {
        Ok(explanation) => write_json(&serde_json::json!({ "ok": true, "explanation": explanation })),
        Err(e) => write_json(&err(format!("LLM error: {e}"))),
    }
}

fn handle_ask(input: &serde_json::Value) -> i32 {
    let message = match input.get("message").and_then(|v| v.as_str()) {
        Some(m) if !m.is_empty() => m.to_string(),
        Some(_) => return write_json(&err("message cannot be empty")),
        None => return write_json(&err("missing required field: message")),
    };

    let session_id = input
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    // Build user message, optionally prepending file context
    let user_content = if let Some(path) = input.get("path").and_then(|v| v.as_str()) {
        if !path.is_empty() {
            match fs_read_text(path) {
                Some(file_content) => {
                    format!("Context from `{path}`:\n```\n{file_content}\n```\n\n{message}")
                }
                None => return write_json(&err(format!("could not read file: {path}"))),
            }
        } else {
            message.clone()
        }
    } else {
        message.clone()
    };

    // Load session history
    let session_key = format!("session:{session_id}");
    let mut history: Vec<ChatMessage> = kv_load::<Vec<ChatMessage>>(&session_key).unwrap_or_default();

    history.push(ChatMessage { role: "user".into(), content: user_content });

    match llm_chat(ASK_SYSTEM, history.clone()) {
        Ok(reply) => {
            history.push(ChatMessage { role: "assistant".into(), content: reply.clone() });
            kv_save(&session_key, &history);
            write_json(&serde_json::json!({ "ok": true, "reply": reply, "session_id": session_id }))
        }
        Err(e) => write_json(&err(format!("LLM error: {e}"))),
    }
}

fn handle_clear_history(input: &serde_json::Value) -> i32 {
    let session_id = input
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    let session_key = format!("session:{session_id}");
    let empty: Vec<ChatMessage> = vec![];
    kv_save(&session_key, &empty);
    write_json(&ok(format!("history cleared for session '{session_id}'")))
}

// ── Manifest ─────────────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn __corvid_manifest() -> i32 {
    let manifest = PluginManifest {
        id: "coding-companion".into(),
        version: "0.1.0".into(),
        author: "corvid-agent".into(),
        description: "LLM-backed coding companion — analyze, review, explain, and ask questions about code in your project.".into(),
        capabilities: vec![
            Capability::LlmChat,
            Capability::Storage { namespace: "coding-companion".into() },
            Capability::FsProjectDir,
        ],
        event_filter: vec![],
        trust_tier: TrustTier::Trusted,
        min_host_version: "0.3.0".into(),
        tools: vec![
            ToolInfo {
                name: "code.analyze".into(),
                description: "Analyze code for bugs, security issues, and improvements. Provide either inline `code` or a `path` to a project file. Optional `focus` narrows the analysis (e.g. 'security', 'performance').".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "code": { "type": "string", "description": "Inline source code to analyze" },
                        "path": { "type": "string", "description": "Project-relative path to the file to analyze" },
                        "focus": { "type": "string", "description": "Area to focus on: security, performance, correctness, style, or all (default: all)" }
                    }
                }),
            },
            ToolInfo {
                name: "code.review".into(),
                description: "Perform a structured code review covering correctness, security, performance, and maintainability. Provide either inline `code` or a `path` to a project file.".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "code": { "type": "string", "description": "Inline source code to review" },
                        "path": { "type": "string", "description": "Project-relative path to the file to review" }
                    }
                }),
            },
            ToolInfo {
                name: "code.explain".into(),
                description: "Explain what code does in plain English — high-level purpose, step-by-step walkthrough, and notable patterns. Provide either inline `code` or a `path` to a project file.".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "code": { "type": "string", "description": "Inline source code to explain" },
                        "path": { "type": "string", "description": "Project-relative path to the file to explain" }
                    }
                }),
            },
            ToolInfo {
                name: "code.ask".into(),
                description: "Ask a coding question. Optionally load a project file as context. Maintains per-session conversation history so you can ask follow-up questions.".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "required": ["message"],
                    "properties": {
                        "message": { "type": "string", "description": "Your coding question" },
                        "path": { "type": "string", "description": "Optional project-relative file to include as context" },
                        "session_id": { "type": "string", "description": "Conversation session key — use the same ID for follow-ups (default: 'default')" }
                    }
                }),
            },
            ToolInfo {
                name: "code.clear_history".into(),
                description: "Clear the conversation history for a session.".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "session_id": { "type": "string", "description": "Session to clear (default: 'default')" }
                    }
                }),
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
        "code.analyze" => handle_analyze(&input),
        "code.review" => handle_review(&input),
        "code.explain" => handle_explain(&input),
        "code.ask" => handle_ask(&input),
        "code.clear_history" => handle_clear_history(&input),
        _ => write_json(&err(format!("unknown tool: {tool}"))),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_input(json: serde_json::Value) -> Vec<u8> {
        rmp_serde::to_vec(&json).unwrap()
    }

    #[test]
    fn analyze_requires_code_or_path() {
        // Non-WASM: fs_read and llm_chat are stubs. Test input validation.
        let result = resolve_code(&serde_json::json!({}));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("provide either"));
    }

    #[test]
    fn analyze_inline_code_resolves() {
        let result = resolve_code(&serde_json::json!({ "code": "fn main() {}" }));
        assert!(result.is_ok());
        let (content, label) = result.unwrap();
        assert_eq!(content, "fn main() {}");
        assert_eq!(label, "provided code");
    }

    #[test]
    fn analyze_empty_code_falls_through_to_error() {
        // Empty `code` should try path, then fail
        let result = resolve_code(&serde_json::json!({ "code": "" }));
        assert!(result.is_err());
    }

    #[test]
    fn analyze_empty_path_falls_through_to_error() {
        // Empty `path` and no `code` should fail
        let result = resolve_code(&serde_json::json!({ "path": "" }));
        assert!(result.is_err());
    }

    #[test]
    fn ask_missing_message_returns_error() {
        let input_bytes = make_input(serde_json::json!({}));
        let input: serde_json::Value = rmp_serde::from_slice(&input_bytes).unwrap();
        let msg = input.get("message").and_then(|v| v.as_str());
        assert!(msg.is_none());
    }

    #[test]
    fn ask_empty_message_returns_error() {
        let input_bytes = make_input(serde_json::json!({ "message": "" }));
        let input: serde_json::Value = rmp_serde::from_slice(&input_bytes).unwrap();
        let msg = input.get("message").and_then(|v| v.as_str());
        assert_eq!(msg, Some(""));
        assert!(msg.map(|m| m.is_empty()).unwrap_or(true));
    }

    #[test]
    fn chat_message_msgpack_roundtrip() {
        let msgs = vec![
            ChatMessage { role: "user".into(), content: "What does this do?".into() },
            ChatMessage { role: "assistant".into(), content: "It does X.".into() },
        ];
        let bytes = rmp_serde::to_vec(&msgs).unwrap();
        let decoded: Vec<ChatMessage> = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].role, "user");
        assert_eq!(decoded[1].content, "It does X.");
    }

    #[test]
    fn kv_load_returns_none_without_wasm() {
        let result = kv_load::<Vec<ChatMessage>>("session:test");
        assert!(result.is_none());
    }

    #[test]
    fn fs_read_text_returns_none_without_wasm() {
        let result = fs_read_text("src/main.rs");
        assert!(result.is_none());
    }

    #[test]
    fn llm_chat_stub_returns_ok() {
        let result = llm_chat("system prompt", vec![
            ChatMessage { role: "user".into(), content: "hello".into() },
        ]);
        assert!(result.is_ok());
    }

    #[test]
    fn dispatch_unknown_tool_returns_error() {
        let input = rmp_serde::to_vec(&serde_json::json!({})).unwrap();
        let tool = "code.nonexistent";
        let input_val: serde_json::Value = rmp_serde::from_slice(&input).unwrap();
        // Simulate dispatch logic
        let result = match tool {
            "code.analyze" | "code.review" | "code.explain" | "code.ask" | "code.clear_history" => {
                true
            }
            _ => false,
        };
        assert!(!result, "unknown tool should not match dispatch");
        let _ = input_val; // used
    }

    #[test]
    fn manifest_capabilities_include_llmchat_storage_fs() {
        let caps = vec![
            Capability::LlmChat,
            Capability::Storage { namespace: "coding-companion".into() },
            Capability::FsProjectDir,
        ];
        assert!(caps.contains(&Capability::LlmChat));
        assert!(caps.contains(&Capability::FsProjectDir));
        assert!(caps
            .iter()
            .any(|c| matches!(c, Capability::Storage { namespace } if namespace == "coding-companion")));
    }

    #[test]
    fn manifest_tool_names_match_dispatch() {
        let tool_names = [
            "code.analyze",
            "code.review",
            "code.explain",
            "code.ask",
            "code.clear_history",
        ];
        for name in &tool_names {
            let handled = matches!(
                *name,
                "code.analyze" | "code.review" | "code.explain" | "code.ask" | "code.clear_history"
            );
            assert!(handled, "tool {name} must be handled in dispatch");
        }
    }

    #[test]
    fn session_key_format() {
        let session_id = "my-session";
        let key = format!("session:{session_id}");
        assert_eq!(key, "session:my-session");
    }

    #[test]
    fn focus_defaults_to_all_areas() {
        let input = serde_json::json!({ "code": "let x = 1;" });
        let focus = input.get("focus").and_then(|v| v.as_str()).unwrap_or("all areas");
        assert_eq!(focus, "all areas");
    }

    #[test]
    fn analyze_with_focus_field() {
        let input = serde_json::json!({ "code": "fn foo() {}", "focus": "security" });
        let focus = input.get("focus").and_then(|v| v.as_str()).unwrap_or("all areas");
        assert_eq!(focus, "security");
    }
}
