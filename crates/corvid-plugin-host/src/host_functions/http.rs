//! Host function: allowlisted outbound HTTP with SSRF mitigation.

use wasmtime::Linker;

use crate::loader::PluginState;
use crate::sandbox::is_ssrf_blocked;
use crate::wasm_mem;

/// Validates a URL against an allowlist and SSRF rules.
///
/// Returns true if the request should be allowed.
pub fn validate_url(url: &str, allowlist: &[String]) -> bool {
    // SSRF check first
    if is_ssrf_blocked(url) {
        return false;
    }

    // Extract host from URL for allowlist check
    let host = match extract_host_from_url(url) {
        Some(h) => h,
        None => return false,
    };

    // Check against allowlist.
    //
    // Subdomain matching is only applied when the allowlist entry itself
    // contains at least one dot (e.g. "example.com").  This prevents a bare
    // TLD entry such as "com" from accidentally matching every ".com" domain.
    allowlist.iter().any(|allowed| {
        host == *allowed || (allowed.contains('.') && host.ends_with(&format!(".{allowed}")))
    })
}

fn extract_host_from_url(url: &str) -> Option<String> {
    let after_scheme = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    let host_port = after_scheme.split('/').next()?;
    // Strip port
    Some(host_port.split(':').next().unwrap_or(host_port).to_string())
}

/// MessagePack-serialized HTTP response written back to WASM memory.
#[derive(serde::Serialize)]
struct HttpResponse {
    status: u16,
    body: Vec<u8>,
}

/// MessagePack-serialized HTTP error written back to WASM memory.
#[derive(serde::Serialize)]
struct HttpError {
    status: u16,
    error: String,
}

/// Execute an HTTP GET request, returning msgpack-serialized response.
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

/// Execute an HTTP POST request, returning msgpack-serialized response.
fn do_http_post(url: &str, request_body: &[u8]) -> Vec<u8> {
    match ureq::post(url)
        .header("Content-Type", "application/octet-stream")
        .send(request_body)
    {
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

            let request_body = match wasm_mem::read_bytes(&mut caller, body_ptr, body_len) {
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

            let response = do_http_post(&url, &request_body);
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
        // Even if somehow allowlisted, SSRF targets are blocked
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
        // A single-label entry like "com" must NOT act as a wildcard for all
        // .com domains — it should only match the exact hostname "com".
        let list = vec!["com".into()];
        assert!(!validate_url("https://evil.com/steal", &list));
        assert!(!validate_url("https://attacker.com/", &list));
    }

    #[test]
    fn allowlist_requires_dot_for_subdomain_match() {
        // Confirm that a proper multi-label entry still allows subdomains
        let list = vec!["example.com".into()];
        assert!(validate_url("https://api.example.com/v1", &list));
        // but a non-matching domain is still blocked
        assert!(!validate_url("https://notexample.com/", &list));
    }
}
