//! Raw host function declarations for WASM plugins.
//!
//! These are the `extern "C"` imports provided by the plugin host.
//! All functions use MessagePack serialization across the WASM boundary.
//! Return values are pointers to MessagePack-encoded response buffers
//! in WASM linear memory.
//!
//! Plugin authors should prefer the high-level service traits instead
//! of calling these directly. The `corvid-plugin-macros` crate generates
//! wrapper implementations that call through these functions.

#[cfg(target_arch = "wasm32")]
extern "C" {
    /// Scoped key-value read.
    pub fn host_kv_get(key_ptr: i32, key_len: i32) -> i32;

    /// Scoped key-value write.
    pub fn host_kv_set(key_ptr: i32, key_len: i32, val_ptr: i32, val_len: i32) -> i32;

    /// Allowlisted HTTP GET.
    pub fn host_http_get(url_ptr: i32, url_len: i32) -> i32;

    /// Allowlisted HTTP POST.
    pub fn host_http_post(url_ptr: i32, url_len: i32, body_ptr: i32, body_len: i32) -> i32;

    /// Read-only SQL query.
    pub fn host_db_query(sql_ptr: i32, sql_len: i32) -> i32;

    /// Sandboxed file read.
    pub fn host_fs_read(path_ptr: i32, path_len: i32) -> i32;

    /// Algorand app state read.
    pub fn host_algo_state(app_id: i64, key_ptr: i32, key_len: i32) -> i32;

    /// Agent message send.
    pub fn host_send_message(target_ptr: i32, target_len: i32, msg_ptr: i32, msg_len: i32) -> i32;

    /// LLM chat — send messages to the host-managed LLM and receive a response.
    ///
    /// `req_ptr`/`req_len`: msgpack-encoded `LlmRequest` (messages + optional system prompt).
    /// Returns a pointer to a length-prefixed msgpack-encoded `LlmResponse`.
    pub fn host_llm_chat(req_ptr: i32, req_len: i32) -> i32;

    /// TTS speak — synthesize text and play it on the host audio device.
    ///
    /// `req_ptr`/`req_len`: msgpack-encoded `TtsRequest` (text, voice, speed).
    /// Returns a pointer to a length-prefixed msgpack-encoded `TtsResponse`.
    /// Blocks until playback completes (or the backend queues it).
    pub fn host_tts_speak(req_ptr: i32, req_len: i32) -> i32;

    /// TTS list voices — enumerate available voice models on the host.
    ///
    /// Returns a pointer to a length-prefixed msgpack-encoded `Vec<String>`.
    pub fn host_tts_list_voices() -> i32;
}
