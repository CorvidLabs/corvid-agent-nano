//! Host function: allowlisted outbound HTTP with SSRF mitigation.
//!
//! `host_http_post` accepts a msgpack-serialized `HttpPostRequest` as its body,
//! which lets plugins supply custom headers (required for LLM API auth). If
//! deserialization fails the raw bytes are sent as-is (backward compat).

use wasmtime::Linker;

use crate::loader::PluginState;
use crate::sandbox::is_ssrf_blocked;
use crate::wasm_mem;

/// Structured POST request body plugin can pass to `host_http_post`.
///
/// Plugins serialize this as msgpack and pass the bytes as the body parameter.
/// Headers are `(name, value)` pairs; Content-Type defaults to
/// `application/octet-stream` if not provided.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct HttpPostRequest {
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Validates a URL against an allowlist and SSRF rules.
pub fn validate_url(url: &str, allowlist: &[String]) -> bool {
    if is_ssrf_blocked(url) {
        return false;
    }
    let host = match extract_host_from_url(url) {
        Some(h) => h,
        None => return false,
    };
    allowlist.iter().any(|allowed| {
        host == *allowed || (allowed.contains('.') && host.ends_with(&format!(".{allowed}")))
    })
}

fn extract_host_from_url(url: &str) -> Option<String> {
    let after_scheme = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    let host_port = after_scheme.split('/').next()?;
    Some(host_port.split(':').next().unwrap_or(host_port).to_string())
}

/// Msgpack-serialized HTTP response written back to WASM memory.
#[derive(serde::Serialize)]
struct HttpResponse {
    status: u16,
    body: Vec<u8>,
}

/// Msgpack-serialized HTTP error written back to WASM memory.
#[derive(serde::Serialize)]
struct HttpError {
    status: u16,
    error: String,
}

fn do_http_get(url: &str) -> Vec<u8> {
    match ureq::get(url).call() {
        Ok(response) => {
            let status: u16 = response.status().into();
            let body = response.into_body().read_to_vec().unwrap_or_default();
            rmp_serde::to_vec(&HttpResponse { status, body }).unwrap_or_default()
        }
        Err(e) => rmp_serde::to_vec(&HttpError {
            status: 0,
            error: e.to_string(),
        })
        .unwrap_or_default(),
    }
}

/// Execute an HTTP POST. Body bytes are tried as msgpack `HttpPostRequest` first;
/// if that fails they are sent raw as `application/octet-stream`.
fn do_http_post(url: &str, body_bytes: &[u8]) -> Vec<u8> {
    let (headers, raw_body): (Vec<(String, String)>, Vec<u8>) =
        if let Ok(req) = rmp_serde::from_slice::<HttpPostRequest>(body_bytes) {
            (req.headers, req.body)
        } else {
            (vec![], body_bytes.to_vec())
        };

    let mut builder = ureq::post(url);

    let mut has_content_type = false;
    for (name, value) in &headers {
        if name.to_lowercase() == "content-type" {
            has_content_type = true;
        }
        builder = builder.header(name.as_str(), value.as_str());
    }
    if !has_content_type {
        builder = builder.header("Content-Type", "application/octet-stream");
    }

    match builder.send(&raw_body[..]) {
        Ok(response) => {
            let status: u16 = response.status().into();
            let body = response.into_body().read_to_vec().unwrap_or_default();
            rmp_serde::to_vec(&HttpResponse { status, body }).unwrap_or_default()
        }
        Err(e) => rmp_serde::to_vec(&HttpError {
            status: 0,
            error: e.to_string(),
        })
        .unwrap_or_default(),
    }
}

/// Link HTTP host functions into the WASM linker.
pub fn link(linker: &mut Linker<PluginState>) -> anyhow::Result<()> {
    // host_http_get(url_ptr, url_len) -> ptr to length-prefixed msgpack response
    linker.func_wrap(
        "env",
        "host_http_get",
        |mut caller: wasmtime::Caller<'_, PluginState>, url_ptr: i32, url_len: i32| -> i32 {
            let url = match wasm_mem::read_str(&mut caller, url_ptr, url_len) {
                Some(u) => u,
                None => {
                    tracing::warn!("host_http_get: failed to read URL from WASM memory");
                    return 0;
                }
            };

            let allowlist = caller.data().http_allowlist.clone();
            if !validate_url(&url, &allowlist) {
                tracing::warn!(url = %url, "host_http_get: URL blocked by allowlist/SSRF rules");
                let err = rmp_serde::to_vec(&HttpError {
                    status: 0,
                    error: "URL blocked by security policy".into(),
                })
                .unwrap_or_default();
                return wasm_mem::write_response(&mut caller, &err);
            }

            let response = do_http_get(&url);
            wasm_mem::write_response(&mut caller, &response)
        },
    )?;

    // host_http_post(url_ptr, url_len, body_ptr, body_len) -> ptr
    //
    // Body bytes: try msgpack HttpPostRequest { headers, body } first.
    // Falls back to raw bytes + application/octet-stream if deserialization fails.
    linker.func_wrap(
        "env",
        "host_http_post",
        |mut caller: wasmtime::Caller<'_, PluginState>,
         url_ptr: i32,
         url_len: i32,
         body_ptr: i32,
         body_len: i32|
         -> i32 {
            let url = match wasm_mem::read_str(&mut caller, url_ptr, url_len) {
                Some(u) => u,
                None => {
                    tracing::warn!("host_http_post: failed to read URL from WASM memory");
                    return 0;
                }
            };

            let body_bytes = match wasm_mem::read_bytes(&mut caller, body_ptr, body_len) {
                Some(b) => b,
                None => {
                    tracing::warn!("host_http_post: failed to read body from WASM memory");
                    return 0;
                }
            };

            let allowlist = caller.data().http_allowlist.clone();
            if !validate_url(&url, &allowlist) {
                tracing::warn!(url = %url, "host_http_post: URL blocked by allowlist/SSRF rules");
                let err = rmp_serde::to_vec(&HttpError {
                    status: 0,
                    error: "URL blocked by security policy".into(),
                })
                .unwrap_or_default();
                return wasm_mem::write_response(&mut caller, &err);
            }

            let response = do_http_post(&url, &body_bytes);
            wasm_mem::write_response(&mut caller, &response)
        },
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_exact_match() {
        let list = vec!["api.example.com".into()];
        assert!(validate_url("https://api.example.com/v1/data", &list));
        assert!(!validate_url("https://evil.com/v1/data", &list));
    }

    #[test]
    fn allowlist_subdomain_match() {
        let list = vec!["example.com".into()];
        assert!(validate_url("https://api.example.com/v1", &list));
        assert!(validate_url("https://example.com/v1", &list));
        assert!(!validate_url("https://notexample.com/v1", &list));
    }

    #[test]
    fn ssrf_blocked_even_if_allowlisted() {
        let list = vec!["127.0.0.1".into()];
        assert!(!validate_url("http://127.0.0.1/admin", &list));
    }

    #[test]
    fn non_http_blocked() {
        let list = vec!["example.com".into()];
        assert!(!validate_url("file:///etc/passwd", &list));
    }

    #[test]
    fn empty_allowlist_blocks_all() {
        assert!(!validate_url("https://example.com/", &[]));
    }

    #[test]
    fn bare_tld_in_allowlist_does_not_match_all_subdomains() {
        let list = vec!["com".into()];
        assert!(!validate_url("https://evil.com/steal", &list));
        assert!(!validate_url("https://attacker.com/", &list));
    }

    #[test]
    fn allowlist_requires_dot_for_subdomain_match() {
        let list = vec!["example.com".into()];
        assert!(validate_url("https://api.example.com/v1", &list));
        assert!(!validate_url("https://notexample.com/", &list));
    }

    #[test]
    fn http_post_request_msgpack_roundtrip() {
        let req = HttpPostRequest {
            headers: vec![
                ("Content-Type".into(), "application/json".into()),
                ("x-api-key".into(), "sk-test-123".into()),
            ],
            body: b"{\"hello\": \"world\"}".to_vec(),
        };
        let packed = rmp_serde::to_vec(&req).unwrap();
        let unpacked: HttpPostRequest = rmp_serde::from_slice(&packed).unwrap();
        assert_eq!(unpacked.headers.len(), 2);
        assert_eq!(unpacked.headers[0].0, "Content-Type");
        assert_eq!(unpacked.body, b"{\"hello\": \"world\"}");
    }

    #[test]
    fn http_post_request_fallback_on_raw_bytes() {
        // Raw bytes that are not valid msgpack HttpPostRequest
        let raw = b"plain text body";
        // Should deserialize to Err and fall back
        let result = rmp_serde::from_slice::<HttpPostRequest>(raw);
        assert!(
            result.is_err(),
            "raw bytes should fail msgpack deserialization"
        );
    }
}
