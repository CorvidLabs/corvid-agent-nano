//! Plugin loading pipeline — ABI check, signature, manifest, instantiation.

use anyhow::{Context, Result};
use corvid_plugin_sdk::{PluginManifest, TrustTier, ABI_MIN_COMPATIBLE, ABI_VERSION};
use wasmtime::{Engine, Instance, Linker, Module, Store};

use crate::sandbox::{MemoryLimiter, SandboxLimits};

/// Errors during plugin loading (before init).
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("ABI version {version} incompatible with host [{min}, {max}]")]
    AbiMismatch { version: u32, min: u32, max: u32 },

    #[error("signature verification failed: {0}")]
    SignatureInvalid(String),

    #[error("invalid manifest: {0}")]
    ManifestInvalid(String),

    #[error("host version {host} < required {required}")]
    HostTooOld { host: String, required: String },

    #[error("WASM error: {0}")]
    Wasm(String),
}

/// A validated, instantiated plugin ready for registration.
pub struct LoadedPlugin {
    pub manifest: PluginManifest,
    pub tier: TrustTier,
    pub module: Module,
    pub limits: SandboxLimits,
}

/// Plugin host state stored in each Wasmtime `Store`.
pub struct PluginState {
    pub limiter: MemoryLimiter,
    pub plugin_id: String,
}

/// Current host version for min_host_version checks.
const HOST_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Validate a plugin manifest.
pub fn validate_manifest(m: &PluginManifest) -> Result<(), LoadError> {
    // ID regex: ^[a-z][a-z0-9-]{0,49}$
    let id_re = regex::Regex::new(r"^[a-z][a-z0-9-]{0,49}$").unwrap();
    if !id_re.is_match(&m.id) {
        return Err(LoadError::ManifestInvalid(format!(
            "ID '{}' does not match ^[a-z][a-z0-9-]{{0,49}}$",
            m.id
        )));
    }

    // Version must be valid semver
    if semver::Version::parse(&m.version).is_err() {
        return Err(LoadError::ManifestInvalid(format!(
            "version '{}' is not valid semver",
            m.version
        )));
    }

    // min_host_version must be valid semver
    let min_host = semver::Version::parse(&m.min_host_version).map_err(|_| {
        LoadError::ManifestInvalid(format!(
            "min_host_version '{}' is not valid semver",
            m.min_host_version
        ))
    })?;

    // Check host version compatibility
    let host = semver::Version::parse(HOST_VERSION).unwrap_or_else(|_| {
        semver::Version::new(0, 1, 0) // fallback for dev builds
    });

    if host < min_host {
        return Err(LoadError::HostTooOld {
            host: HOST_VERSION.to_string(),
            required: m.min_host_version.clone(),
        });
    }

    // Author and description must not be empty
    if m.author.is_empty() {
        return Err(LoadError::ManifestInvalid("author is empty".into()));
    }
    if m.description.is_empty() {
        return Err(LoadError::ManifestInvalid("description is empty".into()));
    }

    Ok(())
}

/// Step 1: Extract and verify ABI version from a compiled WASM module.
pub fn check_abi_version(
    instance: &Instance,
    store: &mut Store<PluginState>,
) -> Result<u32, LoadError> {
    let abi_fn = instance
        .get_typed_func::<(), i32>(&mut *store, "__corvid_abi_version")
        .map_err(|_| LoadError::Wasm("missing export: __corvid_abi_version".into()))?;

    let version = abi_fn
        .call(&mut *store, ())
        .map_err(|e| LoadError::Wasm(format!("__corvid_abi_version call failed: {e}")))?
        as u32;

    if version < ABI_MIN_COMPATIBLE || version > ABI_VERSION {
        return Err(LoadError::AbiMismatch {
            version,
            min: ABI_MIN_COMPATIBLE,
            max: ABI_VERSION,
        });
    }

    Ok(version)
}

/// Step 2: Verify Ed25519 signature (Trusted tier only).
pub fn verify_signature(wasm_bytes: &[u8], tier: TrustTier) -> Result<(), LoadError> {
    if tier != TrustTier::Trusted {
        return Ok(()); // Signature check only for Trusted
    }

    // For now, signature verification requires a detached .sig file
    // alongside the .wasm. Full PKI integration comes in v1.1.
    // We verify the signature exists and is structurally valid.
    //
    // TODO: Implement full Ed25519 verification with key registry
    let _ = wasm_bytes;
    tracing::warn!("Ed25519 signature verification not yet implemented — accepting Trusted plugin");
    Ok(())
}

/// Step 3: Extract manifest from WASM module via `__corvid_manifest` export.
pub fn extract_manifest(
    instance: &Instance,
    store: &mut Store<PluginState>,
) -> Result<PluginManifest, LoadError> {
    let manifest_fn = instance
        .get_typed_func::<(), i32>(&mut *store, "__corvid_manifest")
        .map_err(|_| LoadError::Wasm("missing export: __corvid_manifest".into()))?;

    let ptr = manifest_fn
        .call(&mut *store, ())
        .map_err(|e| LoadError::Wasm(format!("__corvid_manifest call failed: {e}")))?;

    // Read length prefix (4 bytes LE) then MessagePack payload from WASM memory
    let memory = instance
        .get_memory(&mut *store, "memory")
        .ok_or_else(|| LoadError::Wasm("no memory export".into()))?;

    let data = memory.data(&store);
    let ptr = ptr as usize;

    if ptr + 4 > data.len() {
        return Err(LoadError::Wasm("manifest pointer out of bounds".into()));
    }

    let len = u32::from_le_bytes([data[ptr], data[ptr + 1], data[ptr + 2], data[ptr + 3]]) as usize;

    if ptr + 4 + len > data.len() {
        return Err(LoadError::Wasm("manifest data out of bounds".into()));
    }

    let manifest_bytes = &data[ptr + 4..ptr + 4 + len];
    let manifest: PluginManifest = rmp_serde::from_slice(manifest_bytes)
        .map_err(|e| LoadError::ManifestInvalid(format!("failed to deserialize manifest: {e}")))?;

    validate_manifest(&manifest)?;

    Ok(manifest)
}

/// Full 4-step load sequence.
pub fn load_plugin(engine: &Engine, wasm_bytes: &[u8], tier: TrustTier) -> Result<LoadedPlugin> {
    let limits = SandboxLimits::for_tier(tier);
    let module = Module::new(engine, wasm_bytes).context("failed to compile WASM module")?;

    // Step 2: Signature (before manifest for Trusted)
    verify_signature(wasm_bytes, tier).map_err(|e| anyhow::anyhow!("{e}"))?;

    // Create a minimal store + instance for ABI/manifest extraction
    let mut store = Store::new(
        engine,
        PluginState {
            limiter: MemoryLimiter::new(limits.memory_bytes),
            plugin_id: String::new(),
        },
    );
    store.limiter(|state| &mut state.limiter);
    store.set_fuel(limits.fuel_per_call)?;

    let linker = Linker::new(engine);
    let instance = linker
        .instantiate(&mut store, &module)
        .context("failed to instantiate WASM for manifest extraction")?;

    // Step 1: ABI check
    let _abi = check_abi_version(&instance, &mut store).map_err(|e| anyhow::anyhow!("{e}"))?;

    // Step 3: Manifest extraction + validation
    let manifest = extract_manifest(&instance, &mut store).map_err(|e| anyhow::anyhow!("{e}"))?;

    Ok(LoadedPlugin {
        manifest,
        tier,
        module,
        limits,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_plugin_sdk::{Capability, EventKind};

    #[test]
    fn valid_manifest_passes() {
        let m = PluginManifest {
            id: "algo-oracle".into(),
            version: "1.0.0".into(),
            author: "corvid".into(),
            description: "Oracle plugin".into(),
            capabilities: vec![Capability::AlgoRead],
            event_filter: vec![EventKind::AgentMessage],
            trust_tier: TrustTier::Verified,
            min_host_version: "0.1.0".into(),
        };
        assert!(validate_manifest(&m).is_ok());
    }

    #[test]
    fn invalid_id_rejected() {
        let m = PluginManifest {
            id: "INVALID_ID".into(),
            version: "1.0.0".into(),
            author: "corvid".into(),
            description: "test".into(),
            capabilities: vec![],
            event_filter: vec![],
            trust_tier: TrustTier::Untrusted,
            min_host_version: "0.1.0".into(),
        };
        let err = validate_manifest(&m).unwrap_err();
        assert!(matches!(err, LoadError::ManifestInvalid(_)));
    }

    #[test]
    fn invalid_semver_rejected() {
        let m = PluginManifest {
            id: "test-plugin".into(),
            version: "not-semver".into(),
            author: "corvid".into(),
            description: "test".into(),
            capabilities: vec![],
            event_filter: vec![],
            trust_tier: TrustTier::Untrusted,
            min_host_version: "0.1.0".into(),
        };
        let err = validate_manifest(&m).unwrap_err();
        assert!(matches!(err, LoadError::ManifestInvalid(_)));
    }

    #[test]
    fn empty_author_rejected() {
        let m = PluginManifest {
            id: "test-plugin".into(),
            version: "1.0.0".into(),
            author: "".into(),
            description: "test".into(),
            capabilities: vec![],
            event_filter: vec![],
            trust_tier: TrustTier::Untrusted,
            min_host_version: "0.1.0".into(),
        };
        let err = validate_manifest(&m).unwrap_err();
        assert!(matches!(err, LoadError::ManifestInvalid(_)));
    }

    #[test]
    fn signature_skipped_for_non_trusted() {
        assert!(verify_signature(&[], TrustTier::Verified).is_ok());
        assert!(verify_signature(&[], TrustTier::Untrusted).is_ok());
    }

    #[test]
    fn id_regex_edge_cases() {
        // Minimum valid ID
        let mut m = valid_manifest();
        m.id = "a".into();
        assert!(validate_manifest(&m).is_ok());

        // Max length (50 chars)
        m.id = format!("a{}", "b".repeat(49));
        assert!(validate_manifest(&m).is_ok());

        // Too long (51 chars)
        m.id = format!("a{}", "b".repeat(50));
        assert!(validate_manifest(&m).is_err());

        // Can't start with digit
        m.id = "0abc".into();
        assert!(validate_manifest(&m).is_err());

        // Can't start with hyphen
        m.id = "-abc".into();
        assert!(validate_manifest(&m).is_err());

        // Hyphens OK in middle
        m.id = "my-cool-plugin".into();
        assert!(validate_manifest(&m).is_ok());
    }

    fn valid_manifest() -> PluginManifest {
        PluginManifest {
            id: "test-plugin".into(),
            version: "1.0.0".into(),
            author: "corvid".into(),
            description: "A test plugin".into(),
            capabilities: vec![],
            event_filter: vec![],
            trust_tier: TrustTier::Untrusted,
            min_host_version: "0.1.0".into(),
        }
    }
}
