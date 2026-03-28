//! Wasmtime engine configuration with AOT compilation cache.

use std::path::Path;

use anyhow::Result;
use wasmtime::{Config, Engine};

/// Builds a configured Wasmtime engine with AOT cache support.
///
/// Cache directory defaults to `~/.corvid/cache/plugins/<agent-id>/`.
/// Cache key: `(wasm_hash, compiler_version, cpu_features)`.
///
/// - First load: ~150ms per plugin (compile + cache)
/// - Cached load: ~5ms per plugin
pub fn build_engine(cache_dir: &Path) -> Result<Engine> {
    let mut config = Config::new();

    // Enable fuel-based metering for instruction limits
    config.consume_fuel(true);

    // Enable Cranelift compiler optimizations
    config.cranelift_opt_level(wasmtime::OptLevel::Speed);

    // Enable AOT cache if directory exists (or can be created)
    if std::fs::create_dir_all(cache_dir).is_ok() {
        // Wasmtime's built-in cache uses a TOML config.
        // We write a minimal cache config pointing to our directory.
        let cache_config_path = cache_dir.join("wasmtime-cache.toml");
        if !cache_config_path.exists() {
            let config_content = format!(
                "[cache]\n\
                 enabled = true\n\
                 directory = \"{}\"\n",
                cache_dir.display()
            );
            let _ = std::fs::write(&cache_config_path, config_content);
        }

        if let Err(e) = config.cache_config_load(&cache_config_path) {
            tracing::warn!("AOT cache unavailable (falling back to uncached): {e}");
        }
    }

    // WASI: we handle linking manually per-plugin — do NOT use default WASI
    // This is critical for the capability model.

    Engine::new(&config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_builds_successfully() {
        let dir = tempfile::tempdir().unwrap();
        let engine = build_engine(dir.path()).unwrap();
        // Verify engine was created — fuel metering is configured internally
        let _ = engine;
    }

    #[test]
    fn engine_builds_without_cache() {
        // Point to a non-writable path — should still build (no cache)
        let engine = build_engine(Path::new("/nonexistent/cache/path"));
        assert!(engine.is_ok());
    }
}
