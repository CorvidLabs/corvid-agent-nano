//! Host function: TTS audio output via Piper (or mock backend for testing).
//!
//! Config env vars:
//!
//! | Env Var                  | Default           | Description                              |
//! |--------------------------|-------------------|------------------------------------------|
//! | CORVID_TTS_BACKEND       | piper             | piper | mock                             |
//! | CORVID_PIPER_BINARY      | piper             | Path to the piper binary                 |
//! | CORVID_PIPER_DATA_DIR    | ~/.local/share/piper | Directory containing .onnx voice models |
//! | CORVID_PIPER_VOICE       | en_US-lessac-medium | Default voice model name               |
//!
//! Piper is invoked as a subprocess: `piper --model <voice>.onnx --output-raw`
//! with text piped to stdin. Raw 16-bit PCM output is played via the system
//! audio player (`afplay` on macOS, `aplay` on Linux).

use corvid_plugin_sdk::service::{TtsRequest, TtsResponse};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use wasmtime::Linker;

use crate::loader::PluginState;
use crate::wasm_mem;

// ── Backend ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum TtsBackendKind {
    Piper,
    Mock,
}

#[derive(Debug, Clone)]
pub struct TtsBackend {
    pub kind: TtsBackendKind,
    pub piper_binary: PathBuf,
    pub data_dir: PathBuf,
    pub default_voice: String,
}

impl TtsBackend {
    pub fn from_config(
        backend_str: &str,
        piper_binary: PathBuf,
        data_dir: PathBuf,
        default_voice: String,
    ) -> Self {
        let kind = match backend_str.to_lowercase().as_str() {
            "mock" => TtsBackendKind::Mock,
            _ => TtsBackendKind::Piper,
        };
        Self {
            kind,
            piper_binary,
            data_dir,
            default_voice,
        }
    }

    pub fn from_env() -> Self {
        let backend_str = std::env::var("CORVID_TTS_BACKEND").unwrap_or_else(|_| "piper".into());
        let piper_binary = std::env::var("CORVID_PIPER_BINARY")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("piper"));
        let data_dir = std::env::var("CORVID_PIPER_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                dirs::data_local_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("piper")
            });
        let default_voice =
            std::env::var("CORVID_PIPER_VOICE").unwrap_or_else(|_| "en_US-lessac-medium".into());
        Self::from_config(&backend_str, piper_binary, data_dir, default_voice)
    }

    /// List available .onnx voice models in the data directory.
    pub fn list_voices(&self) -> Vec<String> {
        if self.kind == TtsBackendKind::Mock {
            return vec!["mock-voice".into()];
        }
        let Ok(entries) = std::fs::read_dir(&self.data_dir) else {
            return vec![];
        };
        let mut voices: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.ends_with(".onnx") {
                    Some(name.trim_end_matches(".onnx").to_owned())
                } else {
                    None
                }
            })
            .collect();
        voices.sort();
        voices
    }

    /// Synthesize `text` and play it. Blocks until playback completes.
    pub fn speak(&self, req: &TtsRequest) -> Result<(), String> {
        if self.kind == TtsBackendKind::Mock {
            tracing::info!("TTS mock: {:?}", req.text);
            return Ok(());
        }

        let voice = if req.voice.is_empty() {
            &self.default_voice
        } else {
            &req.voice
        };

        // Resolve model path: try exact path first, then data_dir/<voice>.onnx
        let model_path = {
            let p = PathBuf::from(voice);
            if p.is_absolute() && p.exists() {
                p
            } else {
                let candidate = self.data_dir.join(format!("{voice}.onnx"));
                if !candidate.exists() {
                    return Err(format!(
                        "voice model not found: {} (looked in {})",
                        voice,
                        self.data_dir.display()
                    ));
                }
                candidate
            }
        };

        // Speed is passed as --length-scale (inverse of speed: 1.0/speed)
        let length_scale = if req.speed > 0.0 {
            format!("{:.3}", 1.0 / req.speed)
        } else {
            "1.000".into()
        };

        // Run piper: stdin=text, stdout=raw 16kHz mono s16le PCM
        let mut piper = Command::new(&self.piper_binary)
            .args([
                "--model",
                &model_path.to_string_lossy(),
                "--length-scale",
                &length_scale,
                "--output-raw",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("failed to spawn piper: {e} (is piper installed?)"))?;

        piper
            .stdin
            .as_mut()
            .unwrap()
            .write_all(req.text.as_bytes())
            .map_err(|e| format!("failed to write to piper stdin: {e}"))?;

        let output = piper
            .wait_with_output()
            .map_err(|e| format!("piper wait failed: {e}"))?;

        if !output.status.success() {
            return Err(format!(
                "piper exited with status {}",
                output.status.code().unwrap_or(-1)
            ));
        }

        play_pcm_audio(&output.stdout)
    }
}

/// Play raw 16kHz mono s16le PCM via the system audio player.
fn play_pcm_audio(pcm: &[u8]) -> Result<(), String> {
    if pcm.is_empty() {
        return Err("TTS produced no audio output".into());
    }

    // Write to a temp file then play it — avoids platform-specific raw PCM
    // streaming complexity while keeping latency acceptable for typical utterances.
    let tmp = tempfile::Builder::new()
        .suffix(".wav")
        .tempfile()
        .map_err(|e| format!("failed to create temp file: {e}"))?;

    write_wav(tmp.path(), pcm, 16000, 1, 16).map_err(|e| format!("failed to write WAV: {e}"))?;

    let player = if cfg!(target_os = "macos") {
        "afplay"
    } else {
        "aplay"
    };

    let status = Command::new(player)
        .arg(tmp.path())
        .status()
        .map_err(|e| format!("failed to run {player}: {e}"))?;

    if !status.success() {
        return Err(format!(
            "{player} exited with status {}",
            status.code().unwrap_or(-1)
        ));
    }

    Ok(())
}

/// Write a minimal WAV file wrapping raw PCM data.
fn write_wav(
    path: &std::path::Path,
    pcm: &[u8],
    sample_rate: u32,
    channels: u16,
    bits_per_sample: u16,
) -> std::io::Result<()> {
    use std::io::Write;

    let data_len = pcm.len() as u32;
    let byte_rate = sample_rate * channels as u32 * bits_per_sample as u32 / 8;
    let block_align = channels * bits_per_sample / 8;
    let chunk_size = 36 + data_len;

    let mut f = std::fs::File::create(path)?;

    // RIFF header
    f.write_all(b"RIFF")?;
    f.write_all(&chunk_size.to_le_bytes())?;
    f.write_all(b"WAVE")?;

    // fmt chunk
    f.write_all(b"fmt ")?;
    f.write_all(&16u32.to_le_bytes())?; // chunk size
    f.write_all(&1u16.to_le_bytes())?; // PCM format
    f.write_all(&channels.to_le_bytes())?;
    f.write_all(&sample_rate.to_le_bytes())?;
    f.write_all(&byte_rate.to_le_bytes())?;
    f.write_all(&block_align.to_le_bytes())?;
    f.write_all(&bits_per_sample.to_le_bytes())?;

    // data chunk
    f.write_all(b"data")?;
    f.write_all(&data_len.to_le_bytes())?;
    f.write_all(pcm)?;

    Ok(())
}

// ── WASM host function linkage ───────────────────────────────────────────────

fn write_tts_response(caller: &mut wasmtime::Caller<'_, PluginState>, resp: TtsResponse) -> i32 {
    let bytes = match rmp_serde::to_vec(&resp) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("host_tts_speak: failed to serialize response: {e}");
            return 0;
        }
    };
    wasm_mem::write_response(caller, &bytes)
}

fn write_voices_response(
    caller: &mut wasmtime::Caller<'_, PluginState>,
    voices: Vec<String>,
) -> i32 {
    let bytes = match rmp_serde::to_vec(&voices) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("host_tts_list_voices: failed to serialize: {e}");
            return 0;
        }
    };
    wasm_mem::write_response(caller, &bytes)
}

pub fn link(linker: &mut Linker<PluginState>) -> anyhow::Result<()> {
    // host_tts_speak: synthesize and play text
    linker.func_wrap(
        "env",
        "host_tts_speak",
        |mut caller: wasmtime::Caller<'_, PluginState>, req_ptr: i32, req_len: i32| -> i32 {
            let req_bytes = match wasm_mem::read_bytes(&mut caller, req_ptr, req_len) {
                Some(b) => b,
                None => {
                    tracing::warn!("host_tts_speak: failed to read request from WASM memory");
                    return write_tts_response(
                        &mut caller,
                        TtsResponse {
                            ok: false,
                            error: Some("failed to read request".into()),
                        },
                    );
                }
            };

            let req: TtsRequest = match rmp_serde::from_slice(&req_bytes) {
                Ok(r) => r,
                Err(e) => {
                    return write_tts_response(
                        &mut caller,
                        TtsResponse {
                            ok: false,
                            error: Some(format!("invalid request: {e}")),
                        },
                    );
                }
            };

            let backend = match caller.data().tts.as_ref() {
                Some(b) => b.clone(),
                None => {
                    return write_tts_response(
                        &mut caller,
                        TtsResponse {
                            ok: false,
                            error: Some("AudioOutput capability not configured".into()),
                        },
                    );
                }
            };

            match backend.speak(&req) {
                Ok(()) => write_tts_response(
                    &mut caller,
                    TtsResponse {
                        ok: true,
                        error: None,
                    },
                ),
                Err(e) => {
                    tracing::warn!("host_tts_speak: playback failed: {e}");
                    write_tts_response(
                        &mut caller,
                        TtsResponse {
                            ok: false,
                            error: Some(e),
                        },
                    )
                }
            }
        },
    )?;

    // host_tts_list_voices: enumerate available voices
    linker.func_wrap(
        "env",
        "host_tts_list_voices",
        |mut caller: wasmtime::Caller<'_, PluginState>| -> i32 {
            let voices = caller
                .data()
                .tts
                .as_ref()
                .map(|b| b.list_voices())
                .unwrap_or_default();
            write_voices_response(&mut caller, voices)
        },
    )?;

    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_backend() -> TtsBackend {
        TtsBackend::from_config(
            "mock",
            PathBuf::from("piper"),
            PathBuf::from("/tmp"),
            "en_US-lessac-medium".into(),
        )
    }

    #[test]
    fn mock_speak_succeeds() {
        let backend = mock_backend();
        let req = TtsRequest {
            text: "Hello from mock TTS".into(),
            voice: String::new(),
            speed: 1.0,
        };
        assert!(backend.speak(&req).is_ok());
    }

    #[test]
    fn mock_list_voices() {
        let backend = mock_backend();
        let voices = backend.list_voices();
        assert_eq!(voices, vec!["mock-voice"]);
    }

    #[test]
    fn from_config_piper() {
        let b = TtsBackend::from_config(
            "piper",
            PathBuf::from("piper"),
            PathBuf::from("/data"),
            "en_GB-alan-low".into(),
        );
        assert_eq!(b.kind, TtsBackendKind::Piper);
        assert_eq!(b.default_voice, "en_GB-alan-low");
    }

    #[test]
    fn from_config_mock() {
        let b = TtsBackend::from_config(
            "mock",
            PathBuf::from("piper"),
            PathBuf::from("/data"),
            "x".into(),
        );
        assert_eq!(b.kind, TtsBackendKind::Mock);
    }

    #[test]
    fn tts_request_roundtrip() {
        let req = TtsRequest {
            text: "Test utterance".into(),
            voice: "en_US-lessac-medium".into(),
            speed: 1.25,
        };
        let packed = rmp_serde::to_vec(&req).unwrap();
        let unpacked: TtsRequest = rmp_serde::from_slice(&packed).unwrap();
        assert_eq!(unpacked.text, "Test utterance");
        assert_eq!(unpacked.voice, "en_US-lessac-medium");
        assert!((unpacked.speed - 1.25).abs() < 0.001);
    }

    #[test]
    fn tts_response_roundtrip() {
        let resp = TtsResponse {
            ok: true,
            error: None,
        };
        let packed = rmp_serde::to_vec(&resp).unwrap();
        let unpacked: TtsResponse = rmp_serde::from_slice(&packed).unwrap();
        assert!(unpacked.ok);
        assert!(unpacked.error.is_none());
    }

    #[test]
    fn write_wav_produces_valid_header() {
        let pcm: Vec<u8> = vec![0u8; 3200]; // 0.1s of silence at 16kHz mono s16le
        let tmp = tempfile::NamedTempFile::new().unwrap();
        write_wav(tmp.path(), &pcm, 16000, 1, 16).unwrap();
        let data = std::fs::read(tmp.path()).unwrap();
        assert_eq!(&data[0..4], b"RIFF");
        assert_eq!(&data[8..12], b"WAVE");
        assert_eq!(&data[12..16], b"fmt ");
        assert_eq!(&data[36..40], b"data");
        let data_len = u32::from_le_bytes([data[40], data[41], data[42], data[43]]);
        assert_eq!(data_len as usize, pcm.len());
    }

    #[test]
    fn speak_empty_text_mock() {
        let backend = mock_backend();
        let req = TtsRequest {
            text: "".into(),
            voice: "".into(),
            speed: 1.0,
        };
        assert!(backend.speak(&req).is_ok(), "mock should handle empty text");
    }
}
