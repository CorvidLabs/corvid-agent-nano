//! Plugin loading pipeline — ABI check, signature, manifest, instantiation.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use corvid_plugin_sdk::{PluginManifest, TrustTier, ABI_MIN_COMPATIBLE, ABI_VERSION};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use wasmtime::{Engine, Instance, Linker, Module, Store};

use crate::host_functions::algo::AlgoBackend;
use crate::host_functions::db::DbBackend;
use crate::host_functions::fs::FsBackend;
use crate::host_functions::messaging::MessagingBackend;
use crate::host_functions::storage::StorageBackend;
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
///
/// `storage` and `http_allowlist` are `None` during manifest extraction
/// (host functions are never linked in that context) and `Some` during
/// actual plugin execution.
pub struct PluginState {
    pub limiter: MemoryLimiter,
    pub plugin_id: String,
    pub storage: Option<Arc<StorageBackend>>,
    pub http_allowlist: Vec<String>,
    pub algo: Option<Arc<AlgoBackend>>,
    pub messaging: Option<Arc<MessagingBackend>>,
    pub db: Option<Arc<DbBackend>>,
    pub fs: Option<Arc<FsBackend>>,
    /// Target filter pattern from the AgentMessage capability (glob-style).
    pub message_target_filter: Option<String>,
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

    // Validate dependency IDs
    let id_re2 = regex::Regex::new(r"^[a-z][a-z0-9-]{0,49}$").unwrap();
    for dep in &m.dependencies {
        if !id_re2.is_match(dep) {
            return Err(LoadError::ManifestInvalid(format!(
                "dependency ID '{}' does not match ^[a-z][a-z0-9-]{{0,49}}$",
                dep
            )));
        }
        if dep == &m.id {
            return Err(LoadError::ManifestInvalid(format!(
                "plugin '{}' cannot depend on itself",
                m.id
            )));
        }
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

/// Parse a `.sig` file into (public key, signature).
///
/// Format: two lines of hex — first line is the 32-byte Ed25519 public key,
/// second line is the 64-byte Ed25519 signature over the WASM bytes.
pub fn parse_sig_file(sig_data: &[u8]) -> Result<(VerifyingKey, Signature), LoadError> {
    let text = std::str::from_utf8(sig_data)
        .map_err(|_| LoadError::SignatureInvalid("sig file is not valid UTF-8".into()))?;

    let mut lines = text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'));

    let pubkey_hex = lines
        .next()
        .ok_or_else(|| LoadError::SignatureInvalid("sig file missing public key line".into()))?
        .trim();

    let sig_hex = lines
        .next()
        .ok_or_else(|| LoadError::SignatureInvalid("sig file missing signature line".into()))?
        .trim();

    let pubkey_bytes: [u8; 32] = hex::decode(pubkey_hex)
        .map_err(|e| LoadError::SignatureInvalid(format!("invalid public key hex: {e}")))?
        .try_into()
        .map_err(|_| LoadError::SignatureInvalid("public key must be 32 bytes".into()))?;

    let sig_bytes: [u8; 64] = hex::decode(sig_hex)
        .map_err(|e| LoadError::SignatureInvalid(format!("invalid signature hex: {e}")))?
        .try_into()
        .map_err(|_| LoadError::SignatureInvalid("signature must be 64 bytes".into()))?;

    let verifying_key = VerifyingKey::from_bytes(&pubkey_bytes)
        .map_err(|e| LoadError::SignatureInvalid(format!("invalid Ed25519 public key: {e}")))?;

    let signature = Signature::from_bytes(&sig_bytes);

    Ok((verifying_key, signature))
}

/// Check if a public key is present in the trusted keys directory.
///
/// Trusted keys are stored as `{data_dir}/trusted-keys/{name}.pub` files,
/// each containing the hex-encoded 32-byte Ed25519 public key on the first line.
pub fn is_key_trusted(key: &VerifyingKey, trusted_keys_dir: &Path) -> Result<bool, LoadError> {
    let target_hex = hex::encode(key.as_bytes());

    let entries = match std::fs::read_dir(trusted_keys_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(LoadError::SignatureInvalid(format!(
                "trusted keys directory not found: {}",
                trusted_keys_dir.display()
            )));
        }
        Err(e) => {
            return Err(LoadError::SignatureInvalid(format!(
                "failed to read trusted keys directory: {e}"
            )));
        }
    };

    for entry in entries {
        let entry = entry.map_err(|e| {
            LoadError::SignatureInvalid(format!("failed to read trusted key entry: {e}"))
        })?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("pub") {
            continue;
        }
        if let Ok(contents) = std::fs::read_to_string(&path) {
            let stored_hex = contents.lines().next().unwrap_or("").trim();
            if stored_hex == target_hex {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

/// Step 2: Verify Ed25519 signature (Trusted tier only).
///
/// `sig_data`: contents of the detached `.sig` file (hex pubkey + hex signature).
/// `trusted_keys_dir`: path to `{data_dir}/trusted-keys/` containing `.pub` files.
pub fn verify_signature(
    wasm_bytes: &[u8],
    sig_data: Option<&[u8]>,
    trusted_keys_dir: &Path,
    tier: TrustTier,
) -> Result<(), LoadError> {
    if tier != TrustTier::Trusted {
        return Ok(());
    }

    let sig_data = sig_data.ok_or_else(|| {
        LoadError::SignatureInvalid(
            "Ed25519 signature required for Trusted tier — no .sig data provided".into(),
        )
    })?;

    let (verifying_key, signature) = parse_sig_file(sig_data)?;

    // Verify the signature over the raw WASM bytes
    verifying_key
        .verify(wasm_bytes, &signature)
        .map_err(|e| LoadError::SignatureInvalid(format!("Ed25519 verification failed: {e}")))?;

    // Check the signing key is in the trusted registry
    if !is_key_trusted(&verifying_key, trusted_keys_dir)? {
        return Err(LoadError::SignatureInvalid(format!(
            "signature is valid but public key {} is not in the trusted registry",
            hex::encode(verifying_key.as_bytes())
        )));
    }

    tracing::info!(
        pubkey = %hex::encode(verifying_key.as_bytes()),
        "Ed25519 signature verified for Trusted plugin"
    );
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
///
/// `sig_data`: contents of the `.sig` file, required for Trusted tier.
/// `trusted_keys_dir`: path to `{data_dir}/trusted-keys/`.
pub fn load_plugin(
    engine: &Engine,
    wasm_bytes: &[u8],
    sig_data: Option<&[u8]>,
    trusted_keys_dir: &Path,
    tier: TrustTier,
) -> Result<LoadedPlugin> {
    let limits = SandboxLimits::for_tier(tier);
    let module = Module::new(engine, wasm_bytes).context("failed to compile WASM module")?;

    // Step 2: Signature (before manifest for Trusted)
    verify_signature(wasm_bytes, sig_data, trusted_keys_dir, tier)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // Create a minimal store + instance for ABI/manifest extraction
    let mut store = Store::new(
        engine,
        PluginState {
            limiter: MemoryLimiter::new(limits.memory_bytes),
            plugin_id: String::new(),
            storage: None,
            http_allowlist: Vec::new(),
            algo: None,
            messaging: None,
            db: None,
            fs: None,
            message_target_filter: None,
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
    use ed25519_dalek::SigningKey;

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
            tools: vec![],
            dependencies: vec![],
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
            tools: vec![],
            dependencies: vec![],
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
            tools: vec![],
            dependencies: vec![],
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
            tools: vec![],
            dependencies: vec![],
        };
        let err = validate_manifest(&m).unwrap_err();
        assert!(matches!(err, LoadError::ManifestInvalid(_)));
    }

    #[test]
    fn signature_skipped_for_non_trusted() {
        let dummy_dir = std::env::temp_dir().join("corvid-test-no-keys");
        assert!(verify_signature(&[], None, &dummy_dir, TrustTier::Verified).is_ok());
        assert!(verify_signature(&[], None, &dummy_dir, TrustTier::Untrusted).is_ok());
    }

    #[test]
    fn signature_required_for_trusted() {
        let dummy_dir = std::env::temp_dir().join("corvid-test-no-keys");
        let err = verify_signature(&[], None, &dummy_dir, TrustTier::Trusted).unwrap_err();
        assert!(matches!(err, LoadError::SignatureInvalid(_)));
    }

    #[test]
    fn valid_signature_with_trusted_key() {
        use ed25519_dalek::Signer;

        let tmp = tempfile::tempdir().unwrap();
        let trusted_keys_dir = tmp.path().join("trusted-keys");
        std::fs::create_dir_all(&trusted_keys_dir).unwrap();

        // Generate a signing key
        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let verifying_key = signing_key.verifying_key();

        // Register the public key as trusted
        let pubkey_hex = hex::encode(verifying_key.as_bytes());
        std::fs::write(trusted_keys_dir.join("test-publisher.pub"), &pubkey_hex).unwrap();

        // Sign some WASM bytes
        let wasm_bytes = b"fake wasm module bytes for testing";
        let signature = signing_key.sign(wasm_bytes);

        // Create the .sig file content
        let sig_data = format!("{}\n{}\n", pubkey_hex, hex::encode(signature.to_bytes()));

        assert!(verify_signature(
            wasm_bytes,
            Some(sig_data.as_bytes()),
            &trusted_keys_dir,
            TrustTier::Trusted,
        )
        .is_ok());
    }

    #[test]
    fn valid_signature_but_untrusted_key_rejected() {
        use ed25519_dalek::Signer;

        let tmp = tempfile::tempdir().unwrap();
        let trusted_keys_dir = tmp.path().join("trusted-keys");
        std::fs::create_dir_all(&trusted_keys_dir).unwrap();
        // Empty trusted-keys dir — no keys registered

        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let pubkey_hex = hex::encode(verifying_key.as_bytes());

        let wasm_bytes = b"fake wasm module bytes";
        let signature = signing_key.sign(wasm_bytes);
        let sig_data = format!("{}\n{}\n", pubkey_hex, hex::encode(signature.to_bytes()));

        let err = verify_signature(
            wasm_bytes,
            Some(sig_data.as_bytes()),
            &trusted_keys_dir,
            TrustTier::Trusted,
        )
        .unwrap_err();

        assert!(matches!(err, LoadError::SignatureInvalid(_)));
        assert!(format!("{err}").contains("not in the trusted registry"));
    }

    #[test]
    fn invalid_signature_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let trusted_keys_dir = tmp.path().join("trusted-keys");
        std::fs::create_dir_all(&trusted_keys_dir).unwrap();

        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let pubkey_hex = hex::encode(verifying_key.as_bytes());

        // Register key as trusted
        std::fs::write(trusted_keys_dir.join("test.pub"), &pubkey_hex).unwrap();

        // Create a bad signature (all zeros)
        let bad_sig = hex::encode([0u8; 64]);
        let sig_data = format!("{}\n{}\n", pubkey_hex, bad_sig);

        let err = verify_signature(
            b"wasm bytes",
            Some(sig_data.as_bytes()),
            &trusted_keys_dir,
            TrustTier::Trusted,
        )
        .unwrap_err();

        assert!(matches!(err, LoadError::SignatureInvalid(_)));
        assert!(format!("{err}").contains("verification failed"));
    }

    #[test]
    fn parse_sig_file_with_comments() {
        use ed25519_dalek::Signer;

        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let pubkey_hex = hex::encode(verifying_key.as_bytes());

        let wasm = b"test data";
        let sig = signing_key.sign(wasm);
        let sig_hex = hex::encode(sig.to_bytes());

        // Format with comments
        let sig_file = format!("# Signed by CorvidLabs\n{pubkey_hex}\n{sig_hex}\n");
        let (key, _) = parse_sig_file(sig_file.as_bytes()).unwrap();
        assert_eq!(key, verifying_key);
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
            tools: vec![],
            dependencies: vec![],
        }
    }
}
