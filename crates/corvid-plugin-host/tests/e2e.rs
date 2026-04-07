//! End-to-end tests for the plugin host.
//!
//! These tests compile the hello-world plugin to wasm32-unknown-unknown,
//! load it through the full pipeline, and exercise tool invocation,
//! registry lifecycle, signature verification, and sandbox enforcement.

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, OnceLock};

use corvid_plugin_host::host_functions::storage::StorageBackend;
use corvid_plugin_host::invoke::{invoke_tool, InvokeContext};
use corvid_plugin_host::loader::{load_plugin, verify_signature};
use corvid_plugin_host::registry::PluginRegistry;
use corvid_plugin_sdk::TrustTier;

static WASM_BYTES: OnceLock<Vec<u8>> = OnceLock::new();

/// Build the hello-world plugin once for all tests and return the WASM bytes.
fn wasm_bytes() -> Vec<u8> {
    WASM_BYTES
        .get_or_init(|| {
            let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .to_path_buf();

            let plugin_dir = workspace_root.join("plugins/hello-world");

            let output = Command::new("cargo")
                .arg("build")
                .arg("--target")
                .arg("wasm32-unknown-unknown")
                .arg("--release")
                .current_dir(&plugin_dir)
                .output()
                .expect("failed to run cargo build for hello-world plugin");

            assert!(
                output.status.success(),
                "hello-world plugin build failed:\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );

            let wasm_path = plugin_dir
                .join("target/wasm32-unknown-unknown/release/hello_world_plugin.wasm");

            assert!(
                wasm_path.exists(),
                "WASM binary not found at {}",
                wasm_path.display()
            );

            std::fs::read(&wasm_path).expect("failed to read hello-world WASM binary")
        })
        .clone()
}

fn test_engine() -> wasmtime::Engine {
    let tmp = tempfile::tempdir().unwrap();
    corvid_plugin_host::build_engine(tmp.path()).unwrap()
}

fn test_invoke_ctx() -> InvokeContext {
    InvokeContext {
        storage: Arc::new(StorageBackend::new()),
        algo: None,
        messaging: None,
        db: None,
        fs: None,
    }
}

// ── Full Pipeline: Load → Invoke ───────────────────────────────────────

#[test]
fn load_hello_world_plugin() {
    let engine = test_engine();
    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    let loaded = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Untrusted)
        .expect("load_plugin should succeed");

    assert_eq!(loaded.manifest.id, "hello-world");
    assert_eq!(loaded.manifest.version, "0.1.0");
    assert_eq!(loaded.manifest.author, "corvid");
    assert_eq!(loaded.manifest.tools.len(), 2);
    assert_eq!(loaded.manifest.tools[0].name, "hello");
    assert_eq!(loaded.manifest.tools[1].name, "echo");
    assert!(loaded.manifest.capabilities.is_empty());
    assert!(loaded.manifest.dependencies.is_empty());
    assert_eq!(loaded.tier, TrustTier::Untrusted);
}

#[test]
fn invoke_hello_tool_with_name() {
    let engine = test_engine();
    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    let loaded = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Untrusted).unwrap();
    let ctx = test_invoke_ctx();

    let result = invoke_tool(
        &engine,
        &loaded.module,
        &loaded.manifest.id,
        &loaded.manifest.capabilities,
        &loaded.limits,
        &ctx,
        "hello",
        &serde_json::json!({"name": "Algorand"}),
    )
    .expect("invoke_tool should succeed");

    assert_eq!(result["greeting"], "Hello, Algorand!");
}

#[test]
fn invoke_hello_tool_default_name() {
    let engine = test_engine();
    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    let loaded = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Untrusted).unwrap();
    let ctx = test_invoke_ctx();

    let result = invoke_tool(
        &engine,
        &loaded.module,
        &loaded.manifest.id,
        &loaded.manifest.capabilities,
        &loaded.limits,
        &ctx,
        "hello",
        &serde_json::json!({}),
    )
    .unwrap();

    assert_eq!(result["greeting"], "Hello, World!");
}

#[test]
fn invoke_echo_tool() {
    let engine = test_engine();
    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    let loaded = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Untrusted).unwrap();
    let ctx = test_invoke_ctx();

    let input = serde_json::json!({
        "message": "round-trip test",
        "number": 42,
        "nested": {"a": true}
    });

    let result = invoke_tool(
        &engine,
        &loaded.module,
        &loaded.manifest.id,
        &loaded.manifest.capabilities,
        &loaded.limits,
        &ctx,
        "echo",
        &input,
    )
    .unwrap();

    assert_eq!(result, input);
}

#[test]
fn invoke_unknown_tool_returns_error() {
    let engine = test_engine();
    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    let loaded = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Untrusted).unwrap();
    let ctx = test_invoke_ctx();

    let result = invoke_tool(
        &engine,
        &loaded.module,
        &loaded.manifest.id,
        &loaded.manifest.capabilities,
        &loaded.limits,
        &ctx,
        "nonexistent",
        &serde_json::json!({}),
    )
    .unwrap();

    assert!(result["error"].as_str().unwrap().contains("unknown tool"));
}

#[test]
fn invoke_hello_multiple_times() {
    let engine = test_engine();
    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    let loaded = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Untrusted).unwrap();
    let ctx = test_invoke_ctx();

    // Each invocation gets its own WASM store — verify isolation
    for name in &["Alice", "Bob", "Charlie", "Diana", "Eve"] {
        let result = invoke_tool(
            &engine,
            &loaded.module,
            &loaded.manifest.id,
            &loaded.manifest.capabilities,
            &loaded.limits,
            &ctx,
            "hello",
            &serde_json::json!({"name": name}),
        )
        .unwrap();

        assert_eq!(result["greeting"], format!("Hello, {name}!"));
    }
}

// ── Sandbox Enforcement ────────────────────────────────────────────────

#[test]
fn load_at_all_trust_tiers() {
    let engine = test_engine();
    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    // Untrusted — should load fine (no capabilities required)
    let loaded = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Untrusted);
    assert!(loaded.is_ok());

    // Verified — should load fine (no sig required for non-Trusted)
    let loaded = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Verified);
    assert!(loaded.is_ok());

    // Trusted — should fail without signature
    let loaded = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Trusted);
    match loaded {
        Ok(_) => panic!("expected signature error for Trusted tier without sig"),
        Err(e) => {
            let err = format!("{e}");
            assert!(err.contains("signature"), "expected signature error, got: {err}");
        }
    }
}

#[test]
fn sandbox_limits_applied_per_tier() {
    let engine = test_engine();
    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    let untrusted = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Untrusted).unwrap();
    let verified = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Verified).unwrap();

    // Verify tier-specific limits are assigned
    assert_eq!(untrusted.limits.memory_bytes, 4 * 1024 * 1024);
    assert_eq!(untrusted.limits.fuel_per_call, 10_000_000);
    assert!(!untrusted.limits.network_allowed);
    assert!(!untrusted.limits.db_read_allowed);

    assert_eq!(verified.limits.memory_bytes, 32 * 1024 * 1024);
    assert_eq!(verified.limits.fuel_per_call, 100_000_000);
    assert!(verified.limits.network_allowed);
    assert!(verified.limits.db_read_allowed);
}

#[test]
fn invoke_works_with_verified_tier() {
    let engine = test_engine();
    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    let loaded = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Verified).unwrap();
    let ctx = test_invoke_ctx();

    let result = invoke_tool(
        &engine,
        &loaded.module,
        &loaded.manifest.id,
        &loaded.manifest.capabilities,
        &loaded.limits,
        &ctx,
        "hello",
        &serde_json::json!({"name": "Verified"}),
    )
    .unwrap();

    assert_eq!(result["greeting"], "Hello, Verified!");
}

// ── Registry Lifecycle ─────────────────────────────────────────────────

#[tokio::test]
async fn registry_register_and_get() {
    let engine = test_engine();
    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    let loaded = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Untrusted).unwrap();
    let registry = PluginRegistry::new();

    assert!(registry.is_empty().await);

    registry.register(loaded).await.unwrap();

    assert_eq!(registry.len().await, 1);
    assert!(!registry.is_empty().await);

    let slot = registry.get("hello-world").await;
    assert!(slot.is_some());

    let slot = slot.unwrap();
    assert!(slot.is_active());
    assert!(!slot.is_draining());
    assert_eq!(slot.manifest.id, "hello-world");
    assert_eq!(slot.state_str(), "active");
}

#[tokio::test]
async fn registry_duplicate_register_fails() {
    let engine = test_engine();
    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    let loaded1 = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Untrusted).unwrap();
    let loaded2 = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Untrusted).unwrap();
    let registry = PluginRegistry::new();

    registry.register(loaded1).await.unwrap();

    let err = registry.register(loaded2).await;
    assert!(err.is_err());
    assert!(format!("{}", err.unwrap_err()).contains("already registered"));
}

#[tokio::test]
async fn registry_unload() {
    let engine = test_engine();
    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    let loaded = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Untrusted).unwrap();
    let registry = PluginRegistry::new();

    registry.register(loaded).await.unwrap();
    assert_eq!(registry.len().await, 1);

    registry.unload("hello-world").await.unwrap();
    assert_eq!(registry.len().await, 0);
    assert!(registry.get("hello-world").await.is_none());
}

#[tokio::test]
async fn registry_unload_nonexistent_fails() {
    let registry = PluginRegistry::new();
    let err = registry.unload("nonexistent").await;
    assert!(err.is_err());
    assert!(format!("{}", err.unwrap_err()).contains("not found"));
}

#[tokio::test]
async fn registry_hot_reload() {
    let engine = test_engine();
    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    let loaded = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Untrusted).unwrap();
    let new_loaded = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Untrusted).unwrap();
    let registry = PluginRegistry::new();

    registry.register(loaded).await.unwrap();

    // Hot-reload with same binary (verifies the drain → swap → activate cycle)
    registry.reload("hello-world", new_loaded).await.unwrap();

    // Plugin should still be active after reload
    let slot = registry.get("hello-world").await.unwrap();
    assert!(slot.is_active());
    assert_eq!(slot.state_str(), "active");
}

#[tokio::test]
async fn registry_reload_nonexistent_fails() {
    let engine = test_engine();
    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    let loaded = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Untrusted).unwrap();
    let registry = PluginRegistry::new();

    let err = registry.reload("nonexistent", loaded).await;
    assert!(err.is_err());
}

#[tokio::test]
async fn registry_list_manifests() {
    let engine = test_engine();
    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    let loaded = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Untrusted).unwrap();
    let registry = PluginRegistry::new();

    registry.register(loaded).await.unwrap();

    let manifests = registry.list_manifests().await;
    assert_eq!(manifests.len(), 1);
    assert_eq!(manifests[0].id, "hello-world");
    assert_eq!(manifests[0].version, "0.1.0");
    assert_eq!(manifests[0].tools.len(), 2);
}

#[tokio::test]
async fn registry_health_status() {
    let engine = test_engine();
    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    let loaded = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Untrusted).unwrap();
    let registry = PluginRegistry::new();

    registry.register(loaded).await.unwrap();

    let status = registry.health_status().await;
    assert_eq!(status.len(), 1);
    assert_eq!(status["hello-world"], "active");
}

#[tokio::test]
async fn registry_call_guard_lifecycle() {
    let engine = test_engine();
    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    let loaded = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Untrusted).unwrap();
    let registry = PluginRegistry::new();

    registry.register(loaded).await.unwrap();
    let slot = registry.get("hello-world").await.unwrap();

    // Acquire a call guard
    let guard = slot.try_acquire();
    assert!(guard.is_some());
    assert_eq!(
        slot.active_calls
            .load(std::sync::atomic::Ordering::Acquire),
        1
    );

    // Drop guard — active_calls should decrement
    drop(guard);
    assert_eq!(
        slot.active_calls
            .load(std::sync::atomic::Ordering::Acquire),
        0
    );
}

// ── Invoke Through Registry ────────────────────────────────────────────

#[tokio::test]
async fn registry_load_and_invoke() {
    let engine = test_engine();
    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    let loaded = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Untrusted).unwrap();
    let registry = PluginRegistry::new();

    let caps = loaded.manifest.capabilities.clone();
    let limits = loaded.limits.clone();
    let id = loaded.manifest.id.clone();
    registry.register(loaded).await.unwrap();

    let slot = registry.get("hello-world").await.unwrap();
    let _guard = slot.try_acquire().unwrap();

    let module = slot.module.read().await;
    let ctx = test_invoke_ctx();

    let result = invoke_tool(
        &engine,
        &module,
        &id,
        &caps,
        &limits,
        &ctx,
        "hello",
        &serde_json::json!({"name": "Registry"}),
    )
    .unwrap();

    assert_eq!(result["greeting"], "Hello, Registry!");
}

// ── Signature Verification E2E ─────────────────────────────────────────

#[test]
fn signed_plugin_loads_as_trusted() {
    use ed25519_dalek::{Signer, SigningKey};

    let engine = test_engine();
    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    // Set up trusted keys directory
    let trusted_keys_dir = tmp.path().join("trusted-keys");
    std::fs::create_dir_all(&trusted_keys_dir).unwrap();

    // Generate signing key and register as trusted
    let signing_key = SigningKey::from_bytes(&[42u8; 32]);
    let verifying_key = signing_key.verifying_key();
    let pubkey_hex = hex::encode(verifying_key.as_bytes());
    std::fs::write(
        trusted_keys_dir.join("test-publisher.pub"),
        &pubkey_hex,
    )
    .unwrap();

    // Sign the WASM bytes
    let signature = signing_key.sign(&bytes);
    let sig_data = format!("{}\n{}\n", pubkey_hex, hex::encode(signature.to_bytes()));

    // Load as Trusted with valid signature
    let loaded = load_plugin(
        &engine,
        &bytes,
        Some(sig_data.as_bytes()),
        &trusted_keys_dir,
        TrustTier::Trusted,
    )
    .expect("signed plugin should load as Trusted");

    assert_eq!(loaded.manifest.id, "hello-world");
    assert_eq!(loaded.tier, TrustTier::Trusted);
    assert_eq!(loaded.limits.memory_bytes, 128 * 1024 * 1024);
    assert_eq!(loaded.limits.fuel_per_call, 1_000_000_000);
    assert!(loaded.limits.network_allowed);
}

#[test]
fn invoke_signed_trusted_plugin() {
    use ed25519_dalek::{Signer, SigningKey};

    let engine = test_engine();
    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    let trusted_keys_dir = tmp.path().join("trusted-keys");
    std::fs::create_dir_all(&trusted_keys_dir).unwrap();

    let signing_key = SigningKey::from_bytes(&[42u8; 32]);
    let verifying_key = signing_key.verifying_key();
    let pubkey_hex = hex::encode(verifying_key.as_bytes());
    std::fs::write(trusted_keys_dir.join("test.pub"), &pubkey_hex).unwrap();

    let signature = signing_key.sign(&bytes);
    let sig_data = format!("{}\n{}\n", pubkey_hex, hex::encode(signature.to_bytes()));

    let loaded = load_plugin(
        &engine,
        &bytes,
        Some(sig_data.as_bytes()),
        &trusted_keys_dir,
        TrustTier::Trusted,
    )
    .unwrap();

    let ctx = test_invoke_ctx();
    let result = invoke_tool(
        &engine,
        &loaded.module,
        &loaded.manifest.id,
        &loaded.manifest.capabilities,
        &loaded.limits,
        &ctx,
        "hello",
        &serde_json::json!({"name": "Trusted"}),
    )
    .unwrap();

    assert_eq!(result["greeting"], "Hello, Trusted!");
}

#[test]
fn tampered_wasm_fails_signature() {
    use ed25519_dalek::{Signer, SigningKey};

    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    let trusted_keys_dir = tmp.path().join("trusted-keys");
    std::fs::create_dir_all(&trusted_keys_dir).unwrap();

    let signing_key = SigningKey::from_bytes(&[42u8; 32]);
    let verifying_key = signing_key.verifying_key();
    let pubkey_hex = hex::encode(verifying_key.as_bytes());
    std::fs::write(trusted_keys_dir.join("test.pub"), &pubkey_hex).unwrap();

    // Sign the original bytes
    let signature = signing_key.sign(&bytes);
    let sig_data = format!("{}\n{}\n", pubkey_hex, hex::encode(signature.to_bytes()));

    // Tamper with the WASM bytes
    let mut tampered = bytes.clone();
    if let Some(last) = tampered.last_mut() {
        *last ^= 0xFF;
    }

    // Verify that signature check fails on tampered bytes
    let result = verify_signature(
        &tampered,
        Some(sig_data.as_bytes()),
        &trusted_keys_dir,
        TrustTier::Trusted,
    );

    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("verification failed"),
        "expected verification failure, got: {err}"
    );
}

#[test]
fn untrusted_key_rejected_for_trusted_tier() {
    use ed25519_dalek::{Signer, SigningKey};

    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    let trusted_keys_dir = tmp.path().join("trusted-keys");
    std::fs::create_dir_all(&trusted_keys_dir).unwrap();
    // Empty trusted-keys dir — no keys registered

    let signing_key = SigningKey::from_bytes(&[42u8; 32]);
    let verifying_key = signing_key.verifying_key();
    let pubkey_hex = hex::encode(verifying_key.as_bytes());

    let signature = signing_key.sign(&bytes);
    let sig_data = format!("{}\n{}\n", pubkey_hex, hex::encode(signature.to_bytes()));

    let result = verify_signature(
        &bytes,
        Some(sig_data.as_bytes()),
        &trusted_keys_dir,
        TrustTier::Trusted,
    );

    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("not in the trusted registry"));
}

// ── Dependency Checks ──────────────────────────────────────────────────

#[tokio::test]
async fn dependency_check_passes_when_satisfied() {
    let engine = test_engine();
    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    let loaded = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Untrusted).unwrap();
    let registry = PluginRegistry::new();

    // hello-world has no dependencies, so check should pass
    registry
        .check_dependencies(&loaded.manifest)
        .await
        .unwrap();
}

#[tokio::test]
async fn dependency_check_fails_when_missing() {
    let registry = PluginRegistry::new();

    let manifest = corvid_plugin_sdk::PluginManifest {
        id: "dependent-plugin".into(),
        version: "1.0.0".into(),
        author: "test".into(),
        description: "test".into(),
        capabilities: vec![],
        event_filter: vec![],
        trust_tier: TrustTier::Untrusted,
        min_host_version: "0.1.0".into(),
        tools: vec![],
        dependencies: vec!["missing-dep".into()],
    };

    let err = registry.check_dependencies(&manifest).await;
    assert!(err.is_err());
    assert!(format!("{}", err.unwrap_err()).contains("missing-dep"));
}

// ── Topological Order ──────────────────────────────────────────────────

#[test]
fn topological_order_with_real_manifests() {
    let engine = test_engine();
    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    let loaded = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Untrusted).unwrap();

    // Single plugin with no deps — trivial topological sort
    let order = PluginRegistry::topological_order(&[loaded.manifest]).unwrap();
    assert_eq!(order, vec!["hello-world"]);
}

// ── Edge Cases ─────────────────────────────────────────────────────────

#[test]
fn invoke_with_large_input() {
    let engine = test_engine();
    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    let loaded = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Untrusted).unwrap();
    let ctx = test_invoke_ctx();

    // Echo a larger payload to stress-test memory handling
    let big_string = "x".repeat(100_000);
    let input = serde_json::json!({"data": big_string});

    let result = invoke_tool(
        &engine,
        &loaded.module,
        &loaded.manifest.id,
        &loaded.manifest.capabilities,
        &loaded.limits,
        &ctx,
        "echo",
        &input,
    )
    .unwrap();

    assert_eq!(result["data"].as_str().unwrap().len(), 100_000);
}

#[test]
fn invoke_with_unicode_input() {
    let engine = test_engine();
    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    let loaded = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Untrusted).unwrap();
    let ctx = test_invoke_ctx();

    let result = invoke_tool(
        &engine,
        &loaded.module,
        &loaded.manifest.id,
        &loaded.manifest.capabilities,
        &loaded.limits,
        &ctx,
        "hello",
        &serde_json::json!({"name": "世界 🌍"}),
    )
    .unwrap();

    assert_eq!(result["greeting"], "Hello, 世界 🌍!");
}

#[test]
fn invoke_with_empty_json_object() {
    let engine = test_engine();
    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    let loaded = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Untrusted).unwrap();
    let ctx = test_invoke_ctx();

    let result = invoke_tool(
        &engine,
        &loaded.module,
        &loaded.manifest.id,
        &loaded.manifest.capabilities,
        &loaded.limits,
        &ctx,
        "echo",
        &serde_json::json!({}),
    )
    .unwrap();

    assert_eq!(result, serde_json::json!({}));
}

#[test]
fn invoke_with_nested_json() {
    let engine = test_engine();
    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    let loaded = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Untrusted).unwrap();
    let ctx = test_invoke_ctx();

    let input = serde_json::json!({
        "level1": {
            "level2": {
                "level3": {
                    "value": [1, 2, 3, null, true, false]
                }
            }
        }
    });

    let result = invoke_tool(
        &engine,
        &loaded.module,
        &loaded.manifest.id,
        &loaded.manifest.capabilities,
        &loaded.limits,
        &ctx,
        "echo",
        &input,
    )
    .unwrap();

    assert_eq!(result, input);
}

// ── Concurrent Invocations ─────────────────────────────────────────────

#[tokio::test]
async fn concurrent_invocations() {
    let engine = Arc::new(test_engine());
    let bytes = wasm_bytes();
    let tmp = tempfile::tempdir().unwrap();

    let loaded = load_plugin(&engine, &bytes, None, tmp.path(), TrustTier::Untrusted).unwrap();
    let module = Arc::new(loaded.module);
    let caps = Arc::new(loaded.manifest.capabilities);
    let limits = Arc::new(loaded.limits);
    let id = loaded.manifest.id.clone();

    let mut handles = Vec::new();

    for i in 0..10 {
        let engine = Arc::clone(&engine);
        let module = Arc::clone(&module);
        let caps = Arc::clone(&caps);
        let limits = Arc::clone(&limits);
        let id = id.clone();

        handles.push(tokio::task::spawn_blocking(move || {
            let ctx = test_invoke_ctx();
            let name = format!("Worker-{i}");
            let result = invoke_tool(
                &engine,
                &module,
                &id,
                &caps,
                &limits,
                &ctx,
                "hello",
                &serde_json::json!({"name": name}),
            )
            .unwrap();

            assert_eq!(result["greeting"], format!("Hello, Worker-{i}!"));
        }));
    }

    for handle in handles {
        handle.await.unwrap();
    }
}
