//! Plugin Host binary — Unix domain socket JSON-RPC server.
//!
//! Runs as a sidecar process alongside the corvid-agent TypeScript server.
//! Socket path: `{data_dir}/plugins.sock`

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

use corvid_plugin_host::engine::build_engine;
use corvid_plugin_host::host_functions::storage::StorageBackend;
use corvid_plugin_host::invoke::InvokeContext;
use corvid_plugin_host::loader;
use corvid_plugin_host::registry::PluginRegistry;

#[derive(Parser, Debug)]
#[command(
    name = "corvid-plugin-host",
    about = "WASM plugin host for corvid-agent"
)]
struct Cli {
    /// Base directory for socket, cache, plugin storage.
    #[arg(long, default_value = "~/.corvid")]
    data_dir: String,

    /// Unix domain socket path.
    #[arg(long)]
    socket_path: Option<String>,

    /// Wasmtime AOT cache directory.
    #[arg(long)]
    cache_dir: Option<String>,

    /// Agent identity for cache isolation.
    #[arg(long)]
    agent_id: String,

    /// Log level (RUST_LOG filter).
    #[arg(long, default_value = "info")]
    log_level: String,
}

/// JSON-RPC request envelope.
#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    method: String,
    params: serde_json::Value,
    id: Option<serde_json::Value>,
}

/// JSON-RPC response envelope.
#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    result: Option<serde_json::Value>,
    error: Option<String>,
    id: Option<serde_json::Value>,
}

/// Shared server state.
struct ServerState {
    registry: PluginRegistry,
    engine: wasmtime::Engine,
    start_time: Instant,
    data_dir: PathBuf,
    invoke_ctx: InvokeContext,
}

async fn handle_request(state: &ServerState, req: JsonRpcRequest) -> JsonRpcResponse {
    let result = match req.method.as_str() {
        "plugin.list" => {
            let manifests = state.registry.list_manifests().await;
            Ok(serde_json::json!({ "plugins": manifests }))
        }
        "plugin.load" => handle_load(state, &req.params).await,
        "plugin.unload" => handle_unload(state, &req.params).await,
        "plugin.reload" => handle_reload(state, &req.params).await,
        "plugin.invoke" => handle_invoke(state, &req.params).await,
        "plugin.event" => handle_event(state, &req.params).await,
        "plugin.tools" => {
            let resp = corvid_plugin_host::discovery::list_tools(
                &state.registry,
                req.params.get("id").and_then(|v| v.as_str()),
            )
            .await;
            Ok(serde_json::to_value(resp).unwrap_or_default())
        }
        "health.check" => {
            let status = state.registry.health_status().await;
            let uptime_ms = state.start_time.elapsed().as_millis() as u64;
            Ok(serde_json::json!({
                "plugins": status,
                "uptime_ms": uptime_ms,
            }))
        }
        _ => Err(format!("unknown method: {}", req.method)),
    };

    match result {
        Ok(value) => JsonRpcResponse {
            result: Some(value),
            error: None,
            id: req.id,
        },
        Err(err) => JsonRpcResponse {
            result: None,
            error: Some(err),
            id: req.id,
        },
    }
}

async fn handle_load(
    state: &ServerState,
    params: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let path = params
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or("missing 'path' parameter")?;

    let tier_str = params
        .get("tier")
        .and_then(|v| v.as_str())
        .unwrap_or("untrusted");

    let tier = match tier_str {
        "trusted" | "Trusted" => corvid_plugin_sdk::TrustTier::Trusted,
        "verified" | "Verified" => corvid_plugin_sdk::TrustTier::Verified,
        _ => corvid_plugin_sdk::TrustTier::Untrusted,
    };

    let wasm_bytes = std::fs::read(path).map_err(|e| format!("failed to read WASM: {e}"))?;

    // Read detached .sig file if it exists alongside the .wasm
    let sig_path = format!("{path}.sig");
    let sig_data = std::fs::read(&sig_path).ok();
    let trusted_keys_dir = state.data_dir.join("trusted-keys");

    let loaded = corvid_plugin_host::loader::load_plugin(
        &state.engine,
        &wasm_bytes,
        sig_data.as_deref(),
        &trusted_keys_dir,
        tier,
    )
    .map_err(|e| format!("load failed: {e}"))?;

    let plugin_id = loaded.manifest.id.clone();
    state
        .registry
        .register(loaded)
        .await
        .map_err(|e| format!("registration failed: {e}"))?;

    tracing::info!(plugin_id = %plugin_id, tier = ?tier, "plugin loaded");
    Ok(serde_json::json!({ "ok": true, "id": plugin_id }))
}

async fn handle_unload(
    state: &ServerState,
    params: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let id = params
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("missing 'id' parameter")?;

    state
        .registry
        .unload(id)
        .await
        .map_err(|e| format!("unload failed: {e}"))?;

    tracing::info!(plugin_id = %id, "plugin unloaded");
    Ok(serde_json::json!({ "ok": true }))
}

async fn handle_reload(
    state: &ServerState,
    params: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let id = params
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("missing 'id' parameter")?;

    let path = params
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or("missing 'path' parameter")?;

    let tier_str = params
        .get("tier")
        .and_then(|v| v.as_str())
        .unwrap_or("untrusted");

    let tier = match tier_str {
        "trusted" | "Trusted" => corvid_plugin_sdk::TrustTier::Trusted,
        "verified" | "Verified" => corvid_plugin_sdk::TrustTier::Verified,
        _ => corvid_plugin_sdk::TrustTier::Untrusted,
    };

    let wasm_bytes = std::fs::read(path).map_err(|e| format!("failed to read WASM: {e}"))?;

    let sig_path = format!("{path}.sig");
    let sig_data = std::fs::read(&sig_path).ok();
    let trusted_keys_dir = state.data_dir.join("trusted-keys");

    let loaded = corvid_plugin_host::loader::load_plugin(
        &state.engine,
        &wasm_bytes,
        sig_data.as_deref(),
        &trusted_keys_dir,
        tier,
    )
    .map_err(|e| format!("load failed: {e}"))?;

    state
        .registry
        .reload(id, loaded)
        .await
        .map_err(|e| format!("reload failed: {e}"))?;

    tracing::info!(plugin_id = %id, "plugin reloaded");
    Ok(serde_json::json!({ "ok": true }))
}

async fn handle_invoke(
    state: &ServerState,
    params: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let plugin_id = params
        .get("plugin_id")
        .or_else(|| params.get("pluginId"))
        .and_then(|v| v.as_str())
        .ok_or("missing 'plugin_id' parameter")?;

    let tool = params
        .get("tool")
        .and_then(|v| v.as_str())
        .ok_or("missing 'tool' parameter")?;

    // Input can be raw JSON or base64-encoded msgpack (from PluginBridge).
    // The PluginBridge sends base64(msgpack({pluginId, tool, input})) — we
    // extract the nested `input` field.
    let input = match params.get("input").and_then(|v| v.as_str()) {
        Some(b64) => {
            use base64::Engine as _;
            match base64::engine::general_purpose::STANDARD.decode(b64) {
                Ok(bytes) => {
                    let decoded: serde_json::Value =
                        rmp_serde::from_slice(&bytes).unwrap_or_default();
                    // Extract nested input from PluginBridge envelope
                    decoded.get("input").cloned().unwrap_or(decoded)
                }
                Err(_) => serde_json::Value::String(b64.to_string()),
            }
        }
        _ => params
            .get("input")
            .cloned()
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
    };

    let slot = state
        .registry
        .get(plugin_id)
        .await
        .ok_or_else(|| format!("plugin '{}' not found", plugin_id))?;

    if !slot.is_active() {
        return Err(format!("plugin '{}' is not active", plugin_id));
    }

    let _guard = slot
        .try_acquire()
        .ok_or_else(|| format!("plugin '{}' is draining", plugin_id))?;

    let module = slot.module.read().await.clone();
    let limits = slot.limits.clone();
    let capabilities = slot.manifest.capabilities.clone();

    // Run the WASM invocation on a blocking thread (it's synchronous)
    let engine = state.engine.clone();
    let invoke_ctx_storage = Arc::clone(&state.invoke_ctx.storage);
    let invoke_ctx_algo = state.invoke_ctx.algo.as_ref().map(Arc::clone);
    let invoke_ctx_messaging = state.invoke_ctx.messaging.as_ref().map(Arc::clone);
    let plugin_id_owned = plugin_id.to_string();
    let tool_owned = tool.to_string();

    // Extract timeout before the move closure consumes `limits`.
    let timeout_duration = limits.timeout;

    let blocking_fut = tokio::task::spawn_blocking(move || {
        let ctx = InvokeContext {
            storage: invoke_ctx_storage,
            algo: invoke_ctx_algo,
            messaging: invoke_ctx_messaging,
        };

        let handle = std::thread::spawn(move || {
            corvid_plugin_host::invoke::invoke_tool(
                &engine,
                &module,
                &plugin_id_owned,
                &capabilities,
                &limits,
                &ctx,
                &tool_owned,
                &input,
            )
        });

        match handle.join() {
            Ok(result) => result,
            Err(_) => Err(anyhow::anyhow!("plugin panicked during invocation")),
        }
    });

    // Enforce wall-clock timeout. The Wasmtime fuel budget handles
    // instruction-count limits; this guards against blocking I/O or
    // pathological host-function loops that escape fuel accounting.
    let result = tokio::time::timeout(timeout_duration, blocking_fut)
        .await
        .map_err(|_| {
            format!(
                "plugin '{}' timed out after {:?}",
                plugin_id, timeout_duration
            )
        })?
        .map_err(|e| format!("spawn_blocking failed: {e}"))?;

    match result {
        Ok(value) => {
            tracing::info!(plugin_id = %plugin_id, tool = %tool, "tool invoked");
            Ok(serde_json::json!({ "result": value }))
        }
        Err(e) => {
            tracing::warn!(plugin_id = %plugin_id, tool = %tool, error = %e, "tool invocation failed");
            Err(format!("invocation failed: {e}"))
        }
    }
}

async fn handle_event(
    state: &ServerState,
    params: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let event_value = params.get("event").ok_or("missing 'event' parameter")?;

    let event: corvid_plugin_sdk::error::PluginEvent =
        serde_json::from_value(event_value.clone()).map_err(|e| format!("invalid event: {e}"))?;

    let event_kind = event.kind();
    let manifests = state.registry.list_manifests().await;
    let mut dispatched = 0u32;
    let mut errors = Vec::new();

    for manifest in &manifests {
        if !manifest.event_filter.contains(&event_kind) {
            continue;
        }

        let slot = match state.registry.get(&manifest.id).await {
            Some(s) => s,
            None => continue,
        };

        if !slot.is_active() {
            continue;
        }

        let _guard = match slot.try_acquire() {
            Some(g) => g,
            None => continue,
        };

        let module = slot.module.read().await.clone();
        let limits = slot.limits.clone();
        let capabilities = manifest.capabilities.clone();
        let plugin_id = manifest.id.clone();
        let engine = state.engine.clone();
        let ctx_storage = Arc::clone(&state.invoke_ctx.storage);
        let ctx_algo = state.invoke_ctx.algo.as_ref().map(Arc::clone);
        let ctx_messaging = state.invoke_ctx.messaging.as_ref().map(Arc::clone);
        let event_clone = event.clone();

        let result = tokio::task::spawn_blocking(move || {
            let ctx = InvokeContext {
                storage: ctx_storage,
                algo: ctx_algo,
                messaging: ctx_messaging,
            };
            corvid_plugin_host::invoke::dispatch_event_to_plugin(
                &engine,
                &module,
                &plugin_id,
                &capabilities,
                &limits,
                &ctx,
                &event_clone,
            )
        })
        .await;

        match result {
            Ok(Ok(status)) => {
                tracing::info!(
                    plugin_id = %manifest.id,
                    event = %event_kind,
                    status,
                    "event dispatched"
                );
                dispatched += 1;
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    plugin_id = %manifest.id,
                    event = %event_kind,
                    error = %e,
                    "event dispatch failed"
                );
                errors.push(format!("{}: {e}", manifest.id));
            }
            Err(e) => {
                errors.push(format!("{}: spawn failed: {e}", manifest.id));
            }
        }
    }

    Ok(serde_json::json!({
        "ok": true,
        "dispatched": dispatched,
        "errors": errors,
    }))
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&cli.log_level)),
        )
        .init();

    // Resolve paths
    let data_dir = shellexpand(&cli.data_dir);
    let socket_path = cli
        .socket_path
        .map(PathBuf::from)
        .unwrap_or_else(|| data_dir.join("plugins.sock"));

    let cache_dir = cli
        .cache_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| data_dir.join("cache").join("plugins").join(&cli.agent_id));

    // Check for existing socket (another instance running)
    if socket_path.exists() {
        // Try to connect — if it succeeds, another host is running
        if tokio::net::UnixStream::connect(&socket_path).await.is_ok() {
            bail!(
                "socket {} already in use — another plugin host is running",
                socket_path.display()
            );
        }
        // Stale socket file — remove it
        std::fs::remove_file(&socket_path)?;
    }

    // Build Wasmtime engine
    let engine = build_engine(&cache_dir)?;

    // Create shared backends for plugin invocations
    let invoke_ctx = InvokeContext {
        storage: Arc::new(StorageBackend::new()),
        algo: None,      // Set to Some(...) when algod client is configured
        messaging: None, // Set to Some(...) when messaging is configured
    };

    let state = Arc::new(ServerState {
        registry: PluginRegistry::new(),
        engine,
        start_time: Instant::now(),
        data_dir: data_dir.clone(),
        invoke_ctx,
    });

    // Auto-load plugins from {data_dir}/plugins/*.wasm
    let plugins_dir = data_dir.join("plugins");
    if plugins_dir.is_dir() {
        let mut count = 0u32;
        if let Ok(entries) = std::fs::read_dir(&plugins_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "wasm") {
                    let path_str = path.display().to_string();
                    match std::fs::read(&path) {
                        Ok(wasm_bytes) => {
                            let sig_path = format!("{path_str}.sig");
                            let sig_data = std::fs::read(&sig_path).ok();
                            let trusted_keys_dir = data_dir.join("trusted-keys");

                            match loader::load_plugin(
                                &state.engine,
                                &wasm_bytes,
                                sig_data.as_deref(),
                                &trusted_keys_dir,
                                corvid_plugin_sdk::TrustTier::Untrusted,
                            ) {
                                Ok(loaded) => {
                                    let id = loaded.manifest.id.clone();
                                    match state.registry.register(loaded).await {
                                        Ok(()) => {
                                            tracing::info!(plugin = %id, path = %path_str, "auto-loaded plugin");
                                            count += 1;
                                        }
                                        Err(e) => {
                                            tracing::warn!(path = %path_str, error = %e, "failed to register plugin");
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(path = %path_str, error = %e, "failed to load plugin");
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!(path = %path_str, error = %e, "failed to read WASM file");
                        }
                    }
                }
            }
        }
        if count > 0 {
            tracing::info!(count, "auto-loaded plugins from {}", plugins_dir.display());
        }
    }

    // Bind Unix socket
    let listener = UnixListener::bind(&socket_path)?;
    tracing::info!(
        socket = %socket_path.display(),
        agent_id = %cli.agent_id,
        "plugin host started"
    );

    // Accept connections
    loop {
        let (stream, _) = listener.accept().await?;
        let state = Arc::clone(&state);

        tokio::spawn(async move {
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut line = String::new();

            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break, // EOF
                    Ok(_) => {
                        let req: JsonRpcRequest = match serde_json::from_str(&line) {
                            Ok(r) => r,
                            Err(e) => {
                                let resp = JsonRpcResponse {
                                    result: None,
                                    error: Some(format!("invalid JSON-RPC: {e}")),
                                    id: None,
                                };
                                let _ = writer
                                    .write_all(
                                        format!("{}\n", serde_json::to_string(&resp).unwrap())
                                            .as_bytes(),
                                    )
                                    .await;
                                continue;
                            }
                        };

                        let resp = handle_request(&state, req).await;
                        let _ = writer
                            .write_all(
                                format!("{}\n", serde_json::to_string(&resp).unwrap()).as_bytes(),
                            )
                            .await;
                    }
                    Err(e) => {
                        tracing::error!("socket read error: {e}");
                        break;
                    }
                }
            }
        });
    }
}

/// Expand ~ in paths.
fn shellexpand(path: &str) -> PathBuf {
    if path.starts_with('~') {
        if let Some(home) = dirs_home() {
            return PathBuf::from(path.replacen('~', &home, 1));
        }
    }
    PathBuf::from(path)
}

fn dirs_home() -> Option<String> {
    std::env::var("HOME").ok()
}
