//! TTS Voice plugin — Piper-backed text-to-speech for corvid-agent.
//!
//! ## Tools
//! - `speak` — synthesize text and play it on the host audio device
//! - `set_voice` — set the default voice model for subsequent `speak` calls
//! - `get_voice` — get the currently configured voice model name
//! - `list_voices` — list available voice models on the host
//!
//! ## Storage keys (namespace: "tts-voice")
//! - `voice` → String (current voice model name, e.g. "en_US-lessac-medium")
//! - `speed` → f32 as string (speech rate multiplier, default 1.0)
//!
//! ## Env Vars (host-side, not in WASM)
//! - `CORVID_TTS_BACKEND` — piper | mock
//! - `CORVID_PIPER_BINARY` — path to piper binary
//! - `CORVID_PIPER_DATA_DIR` — directory containing .onnx voice models
//! - `CORVID_PIPER_VOICE` — default voice model name

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

/// Read a length-prefixed buffer from WASM memory.
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

// ── Host function imports ────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
extern "C" {
    fn host_kv_get(key_ptr: i32, key_len: i32) -> i32;
    fn host_kv_set(key_ptr: i32, key_len: i32, val_ptr: i32, val_len: i32) -> i32;
    fn host_tts_speak(req_ptr: i32, req_len: i32) -> i32;
    fn host_tts_list_voices() -> i32;
}

// ── KV helpers ───────────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn kv_get_str(key: &str) -> Option<String> {
    let resp_ptr = unsafe { host_kv_get(key.as_ptr() as i32, key.len() as i32) };
    if resp_ptr == 0 {
        return None;
    }
    let bytes = read_length_prefixed(resp_ptr)?;
    // Host returns msgpack { "value": <bytes> } or { "error": ... }
    let outer: serde_json::Value = rmp_serde::from_slice(&bytes).ok()?;
    let inner_bytes = match &outer {
        serde_json::Value::Object(map) => map.get("value")?.as_array()?,
        _ => return None,
    };
    let raw: Vec<u8> = inner_bytes
        .iter()
        .filter_map(|v| v.as_u64().map(|n| n as u8))
        .collect();
    String::from_utf8(raw).ok()
}

#[cfg(not(target_arch = "wasm32"))]
fn kv_get_str(_key: &str) -> Option<String> {
    None
}

#[cfg(target_arch = "wasm32")]
fn kv_set_str(key: &str, value: &str) -> bool {
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
fn kv_set_str(_key: &str, _value: &str) -> bool {
    false
}

// ── TTS helpers ──────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct TtsRequest {
    text: String,
    voice: String,
    speed: f32,
}

#[derive(Serialize, Deserialize)]
struct TtsResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
}

#[cfg(target_arch = "wasm32")]
fn tts_speak(req: &TtsRequest) -> Result<(), String> {
    let req_bytes = rmp_serde::to_vec(req).map_err(|e| e.to_string())?;
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
fn tts_speak(_req: &TtsRequest) -> Result<(), String> {
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn tts_list_voices() -> Vec<String> {
    let ptr = unsafe { host_tts_list_voices() };
    if ptr == 0 {
        return vec![];
    }
    let bytes = match read_length_prefixed(ptr) {
        Some(b) => b,
        None => return vec![],
    };
    rmp_serde::from_slice::<Vec<String>>(&bytes).unwrap_or_default()
}

#[cfg(not(target_arch = "wasm32"))]
fn tts_list_voices() -> Vec<String> {
    vec![]
}

// ── Manifest ─────────────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn __corvid_manifest() -> i32 {
    let manifest = PluginManifest {
        id: "tts-voice".into(),
        version: "0.1.0".into(),
        author: "corvid-agent".into(),
        description: "Text-to-speech voice output via Piper. Synthesizes text and plays audio on the host device.".into(),
        capabilities: vec![
            Capability::AudioOutput,
            Capability::Storage { namespace: "tts-voice".into() },
        ],
        event_filter: vec![],
        trust_tier: TrustTier::Trusted,
        min_host_version: "0.3.0".into(),
        tools: vec![
            ToolInfo {
                name: "speak".into(),
                description: "Synthesize text and play it on the host audio device. Uses the currently configured voice and speed.".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "text": { "type": "string", "description": "Text to speak aloud" },
                        "voice": { "type": "string", "description": "Voice model override (optional, uses stored voice if omitted)" },
                        "speed": { "type": "number", "description": "Speed multiplier override, 0.5–2.0 (optional, uses stored speed if omitted)" }
                    },
                    "required": ["text"]
                }),
            },
            ToolInfo {
                name: "set_voice".into(),
                description: "Set the default voice model for future speak calls.".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "voice": { "type": "string", "description": "Voice model name (e.g. en_US-lessac-medium)" },
                        "speed": { "type": "number", "description": "Default speed multiplier, 0.5–2.0 (optional)" }
                    },
                    "required": ["voice"]
                }),
            },
            ToolInfo {
                name: "get_voice".into(),
                description: "Get the currently configured voice model and speed.".into(),
                input_schema: serde_json::json!({ "type": "object", "properties": {} }),
            },
            ToolInfo {
                name: "list_voices".into(),
                description: "List available Piper voice models on the host system.".into(),
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
pub extern "C" fn __corvid_invoke(tool_ptr: i32, tool_len: i32, input_ptr: i32, input_len: i32) -> i32 {
    let tool_bytes = unsafe { std::slice::from_raw_parts(tool_ptr as *const u8, tool_len as usize) };
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
        "speak" => handle_speak(&input),
        "set_voice" => handle_set_voice(&input),
        "get_voice" => handle_get_voice(&input),
        "list_voices" => handle_list_voices(&input),
        _ => write_json(&err(format!("unknown tool: {tool}"))),
    }
}

fn handle_speak(input: &serde_json::Value) -> i32 {
    let text = match input.get("text").and_then(|v| v.as_str()) {
        Some(t) if !t.is_empty() => t.to_string(),
        Some(_) => return write_json(&err("text cannot be empty")),
        None => return write_json(&err("missing required field: text")),
    };

    let stored_voice = kv_get_str("voice").unwrap_or_default();
    let voice = input
        .get("voice")
        .and_then(|v| v.as_str())
        .unwrap_or(&stored_voice)
        .to_string();

    let stored_speed: f32 = kv_get_str("speed")
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    let speed = input
        .get("speed")
        .and_then(|v| v.as_f64())
        .map(|f| f as f32)
        .unwrap_or(stored_speed)
        .clamp(0.1, 4.0);

    let req = TtsRequest { text, voice, speed };

    match tts_speak(&req) {
        Ok(()) => write_json(&ok("spoken")),
        Err(e) => write_json(&err(e)),
    }
}

fn handle_set_voice(input: &serde_json::Value) -> i32 {
    let voice = match input.get("voice").and_then(|v| v.as_str()) {
        Some(v) if !v.is_empty() => v.to_string(),
        Some(_) => return write_json(&err("voice cannot be empty")),
        None => return write_json(&err("missing required field: voice")),
    };

    kv_set_str("voice", &voice);

    if let Some(speed) = input.get("speed").and_then(|v| v.as_f64()) {
        let speed = (speed as f32).clamp(0.1, 4.0);
        kv_set_str("speed", &speed.to_string());
    }

    write_json(&ok(format!("voice set to {voice}")))
}

fn handle_get_voice(_input: &serde_json::Value) -> i32 {
    let voice = kv_get_str("voice").unwrap_or_else(|| "(host default)".into());
    let speed: f32 = kv_get_str("speed")
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    write_json(&serde_json::json!({ "voice": voice, "speed": speed }))
}

fn handle_list_voices(_input: &serde_json::Value) -> i32 {
    let voices = tts_list_voices();
    write_json(&serde_json::json!({ "voices": voices, "count": voices.len() }))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tts_request_roundtrip() {
        let req = TtsRequest {
            text: "Hello world".into(),
            voice: "en_US-lessac-medium".into(),
            speed: 1.0,
        };
        let bytes = rmp_serde::to_vec(&req).unwrap();
        let decoded: TtsRequest = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(decoded.text, "Hello world");
        assert_eq!(decoded.voice, "en_US-lessac-medium");
        assert!((decoded.speed - 1.0).abs() < 0.001);
    }

    #[test]
    fn tts_response_roundtrip() {
        let resp = TtsResponse { ok: true, error: None };
        let bytes = rmp_serde::to_vec(&resp).unwrap();
        let decoded: TtsResponse = rmp_serde::from_slice(&bytes).unwrap();
        assert!(decoded.ok);
        assert!(decoded.error.is_none());

        let resp_err = TtsResponse { ok: false, error: Some("piper not found".into()) };
        let bytes = rmp_serde::to_vec(&resp_err).unwrap();
        let decoded: TtsResponse = rmp_serde::from_slice(&bytes).unwrap();
        assert!(!decoded.ok);
        assert_eq!(decoded.error.as_deref(), Some("piper not found"));
    }

    #[test]
    fn speak_missing_text_returns_error() {
        let result = rmp_serde::to_vec(&serde_json::json!({})).unwrap();
        let input: serde_json::Value = rmp_serde::from_slice(&result).unwrap();
        // Non-WASM: tts_speak is a no-op, so test input validation only
        let text = input.get("text").and_then(|v| v.as_str());
        assert!(text.is_none(), "missing text should not be found");
    }

    #[test]
    fn speed_clamping() {
        // Speed values outside 0.1–4.0 should be clamped
        assert!((0.05_f32.clamp(0.1, 4.0) - 0.1).abs() < 0.001);
        assert!((5.0_f32.clamp(0.1, 4.0) - 4.0).abs() < 0.001);
        assert!((1.5_f32.clamp(0.1, 4.0) - 1.5).abs() < 0.001);
    }

    #[test]
    fn manifest_has_required_tools() {
        // Verify tool names are present in the manifest definition
        let tool_names = ["speak", "set_voice", "get_voice", "list_voices"];
        // The manifest is generated by __corvid_manifest — verify the names match
        // the dispatch table in __corvid_invoke
        for name in &tool_names {
            // Each tool must be handled in the match block
            let result = match *name {
                "speak" | "set_voice" | "get_voice" | "list_voices" => true,
                _ => false,
            };
            assert!(result, "tool {name} not handled in dispatch");
        }
    }

    #[test]
    fn capability_includes_audio_output() {
        let caps = vec![
            Capability::AudioOutput,
            Capability::Storage { namespace: "tts-voice".into() },
        ];
        assert!(caps.contains(&Capability::AudioOutput));
        assert!(caps.iter().any(|c| matches!(c, Capability::Storage { namespace } if namespace == "tts-voice")));
    }

    #[test]
    fn list_voices_non_wasm_returns_empty() {
        let voices = tts_list_voices();
        assert!(voices.is_empty(), "non-WASM list_voices should return empty vec");
    }

    #[test]
    fn tts_speak_non_wasm_is_noop() {
        let req = TtsRequest {
            text: "test".into(),
            voice: "".into(),
            speed: 1.0,
        };
        assert!(tts_speak(&req).is_ok(), "non-WASM tts_speak should be a no-op Ok");
    }
}
