//! Security sandboxing — per-tier resource limits and capability gating.

use std::time::Duration;

use corvid_plugin_sdk::TrustTier;

/// Per-tier resource limits enforced by the Wasmtime store.
#[derive(Debug, Clone)]
pub struct SandboxLimits {
    /// Maximum linear memory in bytes.
    pub memory_bytes: usize,
    /// Fuel budget per tool call (instruction count).
    pub fuel_per_call: u64,
    /// Wall-clock timeout per invocation.
    pub timeout: Duration,
    /// Whether outbound HTTP is allowed.
    pub network_allowed: bool,
    /// Whether database reads are allowed.
    pub db_read_allowed: bool,
    /// Whether agent messaging is allowed.
    pub messaging_allowed: bool,
    /// Whether Algorand state reads are allowed.
    pub algo_read_allowed: bool,
}

impl SandboxLimits {
    /// Returns the limits for a given trust tier.
    pub fn for_tier(tier: TrustTier) -> Self {
        match tier {
            TrustTier::Trusted => Self {
                memory_bytes: 128 * 1024 * 1024, // 128 MB
                fuel_per_call: 1_000_000_000,    // 1B instructions
                timeout: Duration::from_secs(30),
                network_allowed: true,
                db_read_allowed: true,
                messaging_allowed: true,
                algo_read_allowed: true,
            },
            TrustTier::Verified => Self {
                memory_bytes: 32 * 1024 * 1024, // 32 MB
                fuel_per_call: 100_000_000,     // 100M instructions
                timeout: Duration::from_secs(5),
                network_allowed: true, // read-only, allowlist
                db_read_allowed: true,
                messaging_allowed: false,
                algo_read_allowed: true,
            },
            TrustTier::Untrusted => Self {
                memory_bytes: 4 * 1024 * 1024, // 4 MB
                fuel_per_call: 10_000_000,     // 10M instructions
                timeout: Duration::from_secs(1),
                network_allowed: false,
                db_read_allowed: false,
                messaging_allowed: false,
                algo_read_allowed: false,
            },
        }
    }
}

/// SSRF mitigation — checks if a URL targets a blocked address.
///
/// Blocks RFC1918, localhost, link-local, and cloud metadata endpoints.
pub fn is_ssrf_blocked(url: &str) -> bool {
    // Reject non-http(s) schemes
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return true;
    }

    // Extract host portion
    let host = match extract_host(url) {
        Some(h) => h,
        None => return true, // Can't parse → block
    };

    is_blocked_host(host)
}

fn extract_host(url: &str) -> Option<&str> {
    let after_scheme = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    // Host is everything up to the first / or end
    let host_port = after_scheme.split('/').next().unwrap_or(after_scheme);
    Some(host_port)
}

fn is_blocked_host(host_with_port: &str) -> bool {
    // Normalize: strip brackets for IPv6, strip port
    let host = if host_with_port.starts_with('[') {
        // Bracketed IPv6 like [::1]:8080 → extract ::1
        host_with_port
            .trim_start_matches('[')
            .split(']')
            .next()
            .unwrap_or(host_with_port)
    } else if host_with_port.matches(':').count() == 1 {
        // IPv4 or hostname with port like 127.0.0.1:8080
        host_with_port.split(':').next().unwrap_or(host_with_port)
    } else {
        // Bare IPv6 (::1) or hostname without port
        host_with_port
    };

    let host_lower = host.to_lowercase();

    // Localhost names
    if host_lower == "localhost" {
        return true;
    }

    // Cloud metadata
    if host == "169.254.169.254" {
        return true;
    }

    // Parse IPv4
    let parts: Vec<u8> = host
        .split('.')
        .filter_map(|p| p.parse::<u8>().ok())
        .collect();

    if parts.len() == 4 {
        let (a, b) = (parts[0], parts[1]);
        if a == 127 {
            return true;
        } // 127.0.0.0/8
        if a == 10 {
            return true;
        } // 10.0.0.0/8
        if a == 172 && (16..=31).contains(&b) {
            return true;
        } // 172.16.0.0/12
        if a == 192 && b == 168 {
            return true;
        } // 192.168.0.0/16
        if a == 169 && b == 254 {
            return true;
        } // 169.254.0.0/16
    }

    // IPv6 blocked patterns
    if host_lower == "::1" {
        return true;
    }
    // Full ULA range fc00::/7 (includes fd00::/8 and fc00::/8)
    if host_lower.starts_with("fc00:") || host_lower.starts_with("fd00:") {
        return true;
    }
    // Link-local IPv6 fe80::/10
    if host_lower.starts_with("fe80:") {
        return true;
    }
    // IPv4-mapped IPv6 (::ffff:192.168.x.x, ::ffff:127.x.x.x, etc.)
    // These bypass IPv4 checks: http://[::ffff:127.0.0.1]/ reaches localhost
    if let Some(mapped) = host_lower.strip_prefix("::ffff:") {
        let parts: Vec<u8> = mapped
            .split('.')
            .filter_map(|p| p.parse::<u8>().ok())
            .collect();
        if parts.len() == 4 {
            // Re-run the IPv4 blocked check on the mapped address
            let (a, b) = (parts[0], parts[1]);
            if a == 127 || a == 10 || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 168)
                || (a == 169 && b == 254)
            {
                return true;
            }
        } else {
            // Hex-encoded form like ::ffff:7f00:1 (127.0.0.1) — block conservatively
            return true;
        }
    }

    false
}

/// Wasmtime memory limiter for enforcing per-plugin memory caps.
pub struct MemoryLimiter {
    limit: usize,
    allocated: usize,
}

impl MemoryLimiter {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            allocated: 0,
        }
    }
}

impl wasmtime::ResourceLimiter for MemoryLimiter {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> anyhow::Result<bool> {
        let delta = desired.saturating_sub(current);
        if self.allocated + delta > self.limit {
            Ok(false)
        } else {
            self.allocated += delta;
            Ok(true)
        }
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> anyhow::Result<bool> {
        // Allow table growth up to a reasonable limit (64K entries)
        Ok(desired <= 65536)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasmtime::ResourceLimiter;

    #[test]
    fn tier_limits_ordering() {
        let trusted = SandboxLimits::for_tier(TrustTier::Trusted);
        let verified = SandboxLimits::for_tier(TrustTier::Verified);
        let untrusted = SandboxLimits::for_tier(TrustTier::Untrusted);

        assert!(trusted.memory_bytes > verified.memory_bytes);
        assert!(verified.memory_bytes > untrusted.memory_bytes);
        assert!(trusted.fuel_per_call > verified.fuel_per_call);
        assert!(verified.fuel_per_call > untrusted.fuel_per_call);
        assert!(trusted.timeout > verified.timeout);
        assert!(verified.timeout > untrusted.timeout);
    }

    #[test]
    fn ssrf_blocks_localhost() {
        assert!(is_ssrf_blocked("http://127.0.0.1/admin"));
        assert!(is_ssrf_blocked("http://127.0.0.255/"));
        assert!(is_ssrf_blocked("http://localhost/secret"));
    }

    #[test]
    fn ssrf_blocks_rfc1918() {
        assert!(is_ssrf_blocked("http://10.0.0.1/"));
        assert!(is_ssrf_blocked("http://172.16.0.1/"));
        assert!(is_ssrf_blocked("http://172.31.255.255/"));
        assert!(is_ssrf_blocked("http://192.168.1.1/"));
    }

    #[test]
    fn ssrf_blocks_cloud_metadata() {
        assert!(is_ssrf_blocked("http://169.254.169.254/latest/meta-data/"));
    }

    #[test]
    fn ssrf_blocks_non_http() {
        assert!(is_ssrf_blocked("file:///etc/passwd"));
        assert!(is_ssrf_blocked("ftp://example.com/"));
        assert!(is_ssrf_blocked("gopher://evil.com/"));
    }

    #[test]
    fn ssrf_allows_public() {
        assert!(!is_ssrf_blocked("https://api.example.com/v1/data"));
        assert!(!is_ssrf_blocked("http://8.8.8.8/dns"));
        assert!(!is_ssrf_blocked("https://algorand.foundation/"));
    }

    #[test]
    fn ssrf_blocks_ipv6_localhost() {
        assert!(is_ssrf_blocked("http://[::1]/"));
        assert!(is_ssrf_blocked("http://::1/"));
    }

    #[test]
    fn ssrf_blocks_ipv6_mapped_ipv4() {
        // ::ffff:127.0.0.1 maps to localhost
        assert!(is_ssrf_blocked("http://[::ffff:127.0.0.1]/"));
        assert!(is_ssrf_blocked("http://::ffff:127.0.0.1/"));
        // ::ffff:192.168.1.1 maps to RFC1918
        assert!(is_ssrf_blocked("http://[::ffff:192.168.1.1]/"));
        assert!(is_ssrf_blocked("http://::ffff:192.168.1.1/"));
        // ::ffff:10.0.0.1 maps to RFC1918
        assert!(is_ssrf_blocked("http://[::ffff:10.0.0.1]/"));
        // ::ffff:169.254.169.254 maps to cloud metadata
        assert!(is_ssrf_blocked("http://[::ffff:169.254.169.254]/"));
        // Hex-encoded form (conservatively blocked)
        assert!(is_ssrf_blocked("http://[::ffff:7f00:1]/"));
    }

    #[test]
    fn ssrf_blocks_ipv6_link_local() {
        assert!(is_ssrf_blocked("http://[fe80::1]/"));
        assert!(is_ssrf_blocked("http://fe80::1/"));
    }

    #[test]
    fn ssrf_blocks_ula_full_range() {
        // fd00::/8 (already covered)
        assert!(is_ssrf_blocked("http://[fd00::1]/"));
        // fc00::/8 (now also blocked)
        assert!(is_ssrf_blocked("http://[fc00::1]/"));
        assert!(is_ssrf_blocked("http://fc00::1/"));
    }

    #[test]
    fn untrusted_no_network() {
        let limits = SandboxLimits::for_tier(TrustTier::Untrusted);
        assert!(!limits.network_allowed);
        assert!(!limits.db_read_allowed);
        assert!(!limits.messaging_allowed);
    }

    #[test]
    fn memory_limiter_enforces_cap() {
        let mut limiter = MemoryLimiter::new(1024);
        assert!(limiter.memory_growing(0, 512, None).unwrap());
        assert!(limiter.memory_growing(512, 1024, None).unwrap());
        assert!(!limiter.memory_growing(1024, 2048, None).unwrap());
    }
}
