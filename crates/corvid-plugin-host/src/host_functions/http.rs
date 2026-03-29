//! Host function: allowlisted outbound HTTP with SSRF mitigation.

use wasmtime::Linker;

use crate::loader::PluginState;
use crate::sandbox::is_ssrf_blocked;

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

    // Check against allowlist
    allowlist
        .iter()
        .any(|allowed| host == *allowed || host.ends_with(&format!(".{allowed}")))
}

fn extract_host_from_url(url: &str) -> Option<String> {
    let after_scheme = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    let host_port = after_scheme.split('/').next()?;
    // Strip port
    Some(host_port.split(':').next().unwrap_or(host_port).to_string())
}

/// Link HTTP host functions into the WASM linker.
pub fn link(linker: &mut Linker<PluginState>) -> anyhow::Result<()> {
    // host_http_get(url_ptr, url_len) -> ptr to msgpack response
    linker.func_wrap(
        "env",
        "host_http_get",
        |_caller: wasmtime::Caller<'_, PluginState>, _url_ptr: i32, _url_len: i32| -> i32 {
            // Full implementation will:
            // 1. Read URL from WASM memory
            // 2. Validate against allowlist + SSRF
            // 3. Make HTTP GET request
            // 4. Write msgpack response to WASM memory
            // 5. Return pointer to response
            0 // placeholder
        },
    )?;

    // host_http_post(url_ptr, url_len, body_ptr, body_len) -> ptr
    linker.func_wrap(
        "env",
        "host_http_post",
        |_caller: wasmtime::Caller<'_, PluginState>,
         _url_ptr: i32,
         _url_len: i32,
         _body_ptr: i32,
         _body_len: i32|
         -> i32 {
            0 // placeholder
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
}
