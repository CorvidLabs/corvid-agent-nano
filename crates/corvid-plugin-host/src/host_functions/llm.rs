//! Host function: LLM chat via host-managed provider.
//!
//! The host reads provider config from environment variables so API keys
//! never enter WASM memory:
//!
//! | Env Var              | Default          | Description                    |
//! |----------------------|------------------|--------------------------------|
//! | CORVID_LLM_PROVIDER  | claude           | claude | openai | ollama        |
//! | CORVID_LLM_ENDPOINT  | (provider default)| Base URL override              |
//! | CORVID_LLM_API_KEY   | —                | API key (not needed for Ollama)|
//! | CORVID_LLM_MODEL     | (provider default)| Model name                     |

use corvid_plugin_sdk::service::{LlmRequest, LlmResponse};
use serde::{Deserialize, Serialize};
use wasmtime::Linker;

use crate::loader::PluginState;
use crate::wasm_mem;

/// Supported LLM providers.
#[derive(Debug, Clone, PartialEq)]
pub enum LlmProvider {
    Claude,
    OpenAi,
    Ollama,
}

/// Host-side LLM backend — reads config from env vars at construction time.
#[derive(Debug, Clone)]
pub struct LlmBackend {
    pub provider: LlmProvider,
    pub endpoint: String,
    pub api_key: String,
    pub model: String,
}

impl LlmBackend {
    /// Build from explicit config values. Returns `None` if config is missing
    /// required values (e.g., no API key for Claude/OpenAI).
    pub fn from_config(
        provider_str: &str,
        api_key: String,
        endpoint_override: Option<String>,
        model_override: Option<String>,
    ) -> Option<Self> {
        let provider = match provider_str.to_lowercase().as_str() {
            "openai" => LlmProvider::OpenAi,
            "ollama" => LlmProvider::Ollama,
            _ => LlmProvider::Claude,
        };

        if api_key.is_empty() && provider != LlmProvider::Ollama {
            tracing::warn!("LlmChat: no API key provided — capability will be unavailable");
            return None;
        }

        let endpoint = endpoint_override.unwrap_or_else(|| match provider {
            LlmProvider::Claude => "https://api.anthropic.com/v1/messages".into(),
            LlmProvider::OpenAi => "https://api.openai.com/v1/chat/completions".into(),
            LlmProvider::Ollama => "http://localhost:11434/api/chat".into(),
        });

        let model = model_override.unwrap_or_else(|| match provider {
            LlmProvider::Claude => "claude-haiku-4-5-20251001".into(),
            LlmProvider::OpenAi => "gpt-4o-mini".into(),
            LlmProvider::Ollama => "llama3".into(),
        });

        Some(Self {
            provider,
            endpoint,
            api_key,
            model,
        })
    }

    /// Build from environment variables. Returns `None` if config is missing
    /// required values (e.g., no API key for Claude/OpenAI).
    pub fn from_env() -> Option<Self> {
        let provider_str = std::env::var("CORVID_LLM_PROVIDER")
            .unwrap_or_else(|_| "claude".into());
        let api_key = std::env::var("CORVID_LLM_API_KEY").unwrap_or_default();
        let endpoint = std::env::var("CORVID_LLM_ENDPOINT").ok();
        let model = std::env::var("CORVID_LLM_MODEL").ok();
        Self::from_config(&provider_str, api_key, endpoint, model)
    }

    /// Call the LLM and return the response text.
    pub fn chat(&self, req: &LlmRequest) -> Result<String, String> {
        match self.provider {
            LlmProvider::Claude => self.chat_claude(req),
            LlmProvider::OpenAi => self.chat_openai(req),
            LlmProvider::Ollama => self.chat_ollama(req),
        }
    }

    fn chat_claude(&self, req: &LlmRequest) -> Result<String, String> {
        #[derive(Serialize)]
        struct ClaudeRequest<'a> {
            model: &'a str,
            max_tokens: u32,
            #[serde(skip_serializing_if = "str::is_empty")]
            system: &'a str,
            messages: Vec<ClaudeMessage<'a>>,
        }

        #[derive(Serialize)]
        struct ClaudeMessage<'a> {
            role: &'a str,
            content: &'a str,
        }

        #[derive(Deserialize)]
        struct ClaudeResponse {
            content: Vec<ClaudeContent>,
        }

        #[derive(Deserialize)]
        struct ClaudeContent {
            text: String,
        }

        let messages: Vec<ClaudeMessage> = req
            .messages
            .iter()
            .filter(|m| m.role != "system")
            .map(|m| ClaudeMessage {
                role: &m.role,
                content: &m.content,
            })
            .collect();

        let body = ClaudeRequest {
            model: &self.model,
            max_tokens: 1024,
            system: &req.system,
            messages,
        };

        let body_bytes = serde_json::to_vec(&body).map_err(|e| e.to_string())?;

        let response = ureq::post(&self.endpoint)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .send(&body_bytes[..])
            .map_err(|e| format!("HTTP error: {e}"))?;

        let body_bytes = response
            .into_body()
            .read_to_vec()
            .map_err(|e| format!("read error: {e}"))?;

        let resp: ClaudeResponse =
            serde_json::from_slice(&body_bytes).map_err(|e| format!("parse error: {e}"))?;

        resp.content
            .into_iter()
            .next()
            .map(|c| c.text)
            .ok_or_else(|| "empty response from Claude".into())
    }

    fn chat_openai(&self, req: &LlmRequest) -> Result<String, String> {
        #[derive(Serialize)]
        struct OaiRequest<'a> {
            model: &'a str,
            messages: Vec<OaiMessage>,
        }

        #[derive(Serialize)]
        struct OaiMessage {
            role: String,
            content: String,
        }

        #[derive(Deserialize)]
        struct OaiResponse {
            choices: Vec<OaiChoice>,
        }

        #[derive(Deserialize)]
        struct OaiChoice {
            message: OaiMsg,
        }

        #[derive(Deserialize)]
        struct OaiMsg {
            content: String,
        }

        let mut messages: Vec<OaiMessage> = Vec::new();

        if !req.system.is_empty() {
            messages.push(OaiMessage {
                role: "system".into(),
                content: req.system.clone(),
            });
        }

        for m in &req.messages {
            messages.push(OaiMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            });
        }

        let body = OaiRequest {
            model: &self.model,
            messages,
        };

        let body_bytes = serde_json::to_vec(&body).map_err(|e| e.to_string())?;

        let response = ureq::post(&self.endpoint)
            .header("Authorization", &format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .send(&body_bytes[..])
            .map_err(|e| format!("HTTP error: {e}"))?;

        let resp_bytes = response
            .into_body()
            .read_to_vec()
            .map_err(|e| format!("read error: {e}"))?;

        let resp: OaiResponse =
            serde_json::from_slice(&resp_bytes).map_err(|e| format!("parse error: {e}"))?;

        resp.choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .ok_or_else(|| "empty response from OpenAI".into())
    }

    fn chat_ollama(&self, req: &LlmRequest) -> Result<String, String> {
        #[derive(Serialize)]
        struct OllamaRequest<'a> {
            model: &'a str,
            messages: Vec<OllamaMessage>,
            stream: bool,
        }

        #[derive(Serialize)]
        struct OllamaMessage {
            role: String,
            content: String,
        }

        #[derive(Deserialize)]
        struct OllamaResponse {
            message: OllamaMsg,
        }

        #[derive(Deserialize)]
        struct OllamaMsg {
            content: String,
        }

        let mut messages: Vec<OllamaMessage> = Vec::new();

        if !req.system.is_empty() {
            messages.push(OllamaMessage {
                role: "system".into(),
                content: req.system.clone(),
            });
        }

        for m in &req.messages {
            messages.push(OllamaMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            });
        }

        let body = OllamaRequest {
            model: &self.model,
            messages,
            stream: false,
        };

        let body_bytes = serde_json::to_vec(&body).map_err(|e| e.to_string())?;

        let response = ureq::post(&self.endpoint)
            .header("Content-Type", "application/json")
            .send(&body_bytes[..])
            .map_err(|e| format!("HTTP error: {e}"))?;

        let resp_bytes = response
            .into_body()
            .read_to_vec()
            .map_err(|e| format!("read error: {e}"))?;

        let resp: OllamaResponse =
            serde_json::from_slice(&resp_bytes).map_err(|e| format!("parse error: {e}"))?;

        Ok(resp.message.content)
    }
}

/// Encode an `LlmResponse` as msgpack and return a pointer to the
/// length-prefixed buffer in WASM memory. Returns 0 on allocation failure.
fn write_llm_response(caller: &mut wasmtime::Caller<'_, PluginState>, resp: LlmResponse) -> i32 {
    let bytes = match rmp_serde::to_vec(&resp) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!("host_llm_chat: failed to serialize response: {e}");
            return 0;
        }
    };
    wasm_mem::write_response(caller, &bytes)
}

/// Link the `host_llm_chat` host function into the WASM linker.
pub fn link(linker: &mut Linker<PluginState>) -> anyhow::Result<()> {
    linker.func_wrap(
        "env",
        "host_llm_chat",
        |mut caller: wasmtime::Caller<'_, PluginState>, req_ptr: i32, req_len: i32| -> i32 {
            let req_bytes = match wasm_mem::read_bytes(&mut caller, req_ptr, req_len) {
                Some(b) => b,
                None => {
                    tracing::warn!("host_llm_chat: failed to read request from WASM memory");
                    return write_llm_response(
                        &mut caller,
                        LlmResponse {
                            content: String::new(),
                            error: Some("failed to read request".into()),
                        },
                    );
                }
            };

            let req: LlmRequest = match rmp_serde::from_slice(&req_bytes) {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("host_llm_chat: failed to deserialize request: {e}");
                    return write_llm_response(
                        &mut caller,
                        LlmResponse {
                            content: String::new(),
                            error: Some(format!("invalid request: {e}")),
                        },
                    );
                }
            };

            let backend = match caller.data().llm.as_ref() {
                Some(b) => b.clone(),
                None => {
                    return write_llm_response(
                        &mut caller,
                        LlmResponse {
                            content: String::new(),
                            error: Some(
                                "LlmChat capability not configured (check CORVID_LLM_API_KEY)"
                                    .into(),
                            ),
                        },
                    );
                }
            };

            match backend.chat(&req) {
                Ok(content) => write_llm_response(
                    &mut caller,
                    LlmResponse {
                        content,
                        error: None,
                    },
                ),
                Err(e) => {
                    tracing::warn!("host_llm_chat: LLM call failed: {e}");
                    write_llm_response(
                        &mut caller,
                        LlmResponse {
                            content: String::new(),
                            error: Some(e),
                        },
                    )
                }
            }
        },
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_from_env_no_key() {
        let backend = LlmBackend::from_config("claude", String::new(), None, None);
        assert!(backend.is_none(), "should return None without API key");
    }

    #[test]
    fn backend_from_env_ollama_no_key_needed() {
        let backend = LlmBackend::from_config("ollama", String::new(), None, None);
        assert!(backend.is_some(), "Ollama does not require an API key");
        let b = backend.unwrap();
        assert_eq!(b.provider, LlmProvider::Ollama);
        assert!(b.endpoint.contains("11434"));
    }

    #[test]
    fn backend_from_env_claude_with_key() {
        let backend =
            LlmBackend::from_config("claude", "test-key".into(), None, None).unwrap();
        assert_eq!(backend.provider, LlmProvider::Claude);
        assert_eq!(backend.api_key, "test-key");
        assert!(backend.endpoint.contains("anthropic.com"));
    }

    #[test]
    fn backend_from_env_custom_endpoint() {
        let backend = LlmBackend::from_config(
            "openai",
            "test-key".into(),
            Some("https://my-proxy.example.com/v1/chat".into()),
            None,
        )
        .unwrap();
        assert_eq!(backend.endpoint, "https://my-proxy.example.com/v1/chat");
    }

    #[test]
    fn llm_message_roundtrip() {
        use corvid_plugin_sdk::LlmMessage;
        let req = LlmRequest {
            messages: vec![LlmMessage {
                role: "user".into(),
                content: "Hello!".into(),
            }],
            system: "You are helpful.".into(),
        };
        let packed = rmp_serde::to_vec(&req).unwrap();
        let unpacked: LlmRequest = rmp_serde::from_slice(&packed).unwrap();
        assert_eq!(unpacked.messages.len(), 1);
        assert_eq!(unpacked.system, "You are helpful.");
    }
}
