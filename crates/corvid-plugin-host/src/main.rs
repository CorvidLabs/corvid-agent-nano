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
use corvid_plugin_host::registry::PluginRegistry;

#[derive(Parser, Debug)]
#[command(name = "corvid-plugin-host", about = "WASM plugin host for corvid-agent")]
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
}

async fn handle_request(
    state: &ServerState,
    req: JsonRpcRequest,
) -> JsonRpcResponse {
    let result = match req.method.as_str() {
        "plugin.list" => {
            let manifests = state.registry.list_manifests().await;
            Ok(serde_json::to_value(manifests).unwrap_or_default())
        }
        "plugin.load" => {
            handle_load(state, &req.params).await
        }
        "plugin.unload" => {
            handle_unload(state, &req.params).await
        }
        "plugin.reload" => {
            handle_reload(state, &req.params).await
        }
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

    let loaded = corvid_plugin_host::loader::load_plugin(&state.engine, &wasm_bytes, tier)
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

    let loaded = corvid_plugin_host::loader::load_plugin(&state.engine, &wasm_bytes, tier)
        .map_err(|e| format!("load failed: {e}"))?;

    state
        .registry
        .reload(id, loaded)
        .await
        .map_err(|e| format!("reload failed: {e}"))?;

    tracing::info!(plugin_id = %id, "plugin reloaded");
    Ok(serde_json::json!({ "ok": true }))
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
        .map(|p| PathBuf::from(p))
        .unwrap_or_else(|| data_dir.join("plugins.sock"));

    let cache_dir = cli
        .cache_dir
        .map(|p| PathBuf::from(p))
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

    let state = Arc::new(ServerState {
        registry: PluginRegistry::new(),
        engine,
        start_time: Instant::now(),
    });

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
