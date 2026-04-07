//! MCP server (Model Context Protocol) over stdio and health check HTTP endpoint.
//!
//! `can mcp` starts a JSON-RPC 2.0 MCP server on stdin/stdout so AI agents
//! (Claude Code, Cursor, etc.) can drive nano interactively.
//!
//! `serve_health` starts a minimal HTTP server for Docker/systemd health checks.

use std::time::Instant;

use anyhow::Result;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{info, warn};

use crate::{contacts::ContactStore, load_identity, storage::SqliteMessageCache, wallet, Network};

// ---------------------------------------------------------------------------
// Health check HTTP endpoint
// ---------------------------------------------------------------------------

/// Serve a minimal HTTP health endpoint on `port`.
///
/// `GET /health` returns JSON:
/// ```json
/// {"status":"healthy","network":"localnet","uptime_secs":42,...}
/// ```
pub async fn serve_health(
    port: u16,
    network: String,
    address: String,
    algod_url: String,
    indexer_url: String,
    hub_url: Option<String>,
    start_time: Instant,
) -> Result<()> {
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpListener;

    let listener = TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
    info!(port = port, "health check server listening");

    loop {
        let (mut stream, peer) = listener.accept().await?;

        let network = network.clone();
        let address = address.clone();
        let algod_url = algod_url.clone();
        let indexer_url = indexer_url.clone();
        let hub_url = hub_url.clone();

        tokio::spawn(async move {
            // Read request (just enough to parse the path)
            let mut buf = [0u8; 1024];
            let n = match stream.read(&mut buf).await {
                Ok(n) => n,
                Err(e) => {
                    warn!(peer = %peer, error = %e, "health: read error");
                    return;
                }
            };

            let request = String::from_utf8_lossy(&buf[..n]);
            let first_line = request.lines().next().unwrap_or("");

            // Only respond to GET /health and GET /
            let (status_code, body) = if first_line.starts_with("GET /health")
                || first_line.starts_with("GET / ")
                || first_line == "GET /"
            {
                let uptime = start_time.elapsed().as_secs();
                let body = json!({
                    "status": "healthy",
                    "network": network,
                    "address": address,
                    "uptime_secs": uptime,
                    "algod_url": algod_url,
                    "indexer_url": indexer_url,
                    "hub_url": hub_url,
                });
                (200u16, body.to_string())
            } else {
                let body = json!({"status": "not found"});
                (404u16, body.to_string())
            };

            let status_text = if status_code == 200 {
                "OK"
            } else {
                "Not Found"
            };
            let response = format!(
                "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                status_code,
                status_text,
                body.len(),
                body
            );

            if let Err(e) = stream.write_all(response.as_bytes()).await {
                warn!(peer = %peer, error = %e, "health: write error");
            }
        });
    }
}

// ---------------------------------------------------------------------------
// MCP server over stdio
// ---------------------------------------------------------------------------

/// MCP JSON-RPC 2.0 request.
#[derive(Debug)]
struct McpRequest {
    id: Option<Value>,
    method: String,
    params: Value,
}

impl McpRequest {
    fn parse(line: &str) -> Option<Self> {
        let v: Value = serde_json::from_str(line).ok()?;
        let method = v.get("method")?.as_str()?.to_string();
        let id = v.get("id").cloned();
        let params = v
            .get("params")
            .cloned()
            .unwrap_or(Value::Object(Default::default()));
        Some(McpRequest { id, method, params })
    }
}

/// Write a JSON-RPC response to stdout.
async fn write_response(
    writer: &mut (impl AsyncWriteExt + Unpin),
    id: Option<Value>,
    result: Value,
) -> Result<()> {
    let resp = json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    });
    let mut line = serde_json::to_string(&resp)?;
    line.push('\n');
    writer.write_all(line.as_bytes()).await?;
    writer.flush().await?;
    Ok(())
}

/// Write a JSON-RPC error response to stdout.
async fn write_error(
    writer: &mut (impl AsyncWriteExt + Unpin),
    id: Option<Value>,
    code: i32,
    message: &str,
) -> Result<()> {
    let resp = json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
        },
    });
    let mut line = serde_json::to_string(&resp)?;
    line.push('\n');
    writer.write_all(line.as_bytes()).await?;
    writer.flush().await?;
    Ok(())
}

/// The list of MCP tools exposed by `can mcp`.
fn tool_list() -> Value {
    json!({
        "tools": [
            {
                "name": "send_message",
                "description": "Send an encrypted AlgoChat message to a contact or Algorand address",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "to": {
                            "type": "string",
                            "description": "Contact name or Algorand address"
                        },
                        "message": {
                            "type": "string",
                            "description": "Message text to send"
                        }
                    },
                    "required": ["to", "message"]
                }
            },
            {
                "name": "list_contacts",
                "description": "List all PSK contacts",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            },
            {
                "name": "add_contact",
                "description": "Add a new PSK contact",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "Contact name"},
                        "address": {"type": "string", "description": "Algorand address"},
                        "psk": {"type": "string", "description": "Pre-shared key (hex or base64)"}
                    },
                    "required": ["name", "address", "psk"]
                }
            },
            {
                "name": "remove_contact",
                "description": "Remove a contact by name",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "description": "Contact name to remove"}
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "list_messages",
                "description": "List recent messages from the local inbox",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of messages to return (default 20)"
                        },
                        "from": {
                            "type": "string",
                            "description": "Filter by sender contact name or address"
                        }
                    }
                }
            },
            {
                "name": "get_status",
                "description": "Get agent status: network, address, and contact count",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            }
        ]
    })
}

/// Shared context passed to tool handlers.
struct McpContext {
    data_dir: String,
    network: Network,
    algod_url: String,
    algod_token: String,
    indexer_url: String,
    indexer_token: String,
    seed: [u8; 32],
    address: String,
    hub_url: String,
}

/// Handle a `tools/call` request.
async fn handle_tool_call(ctx: &McpContext, name: &str, args: &Value) -> Value {
    match name {
        "list_contacts" => tool_list_contacts(ctx),
        "add_contact" => tool_add_contact(ctx, args),
        "remove_contact" => tool_remove_contact(ctx, args),
        "list_messages" => tool_list_messages(ctx, args),
        "get_status" => tool_get_status(ctx),
        "send_message" => tool_send_message(ctx, args).await,
        _ => json!({
            "content": [{"type": "text", "text": format!("Unknown tool: {}", name)}],
            "isError": true
        }),
    }
}

fn tool_list_contacts(ctx: &McpContext) -> Value {
    let path = std::path::Path::new(&ctx.data_dir).join("contacts.db");
    if !path.exists() {
        return json!({
            "content": [{"type": "text", "text": "No contacts found."}]
        });
    }
    match ContactStore::open(&path) {
        Ok(store) => match store.list() {
            Ok(contacts) if contacts.is_empty() => json!({
                "content": [{"type": "text", "text": "No contacts."}]
            }),
            Ok(contacts) => {
                let list: Vec<Value> = contacts
                    .iter()
                    .map(|c| json!({"name": c.name, "address": c.address, "added_at": c.added_at}))
                    .collect();
                json!({
                    "content": [{
                        "type": "text",
                        "text": serde_json::to_string_pretty(&list).unwrap_or_default()
                    }]
                })
            }
            Err(e) => json!({
                "content": [{"type": "text", "text": format!("Error: {}", e)}],
                "isError": true
            }),
        },
        Err(e) => json!({
            "content": [{"type": "text", "text": format!("Failed to open contacts: {}", e)}],
            "isError": true
        }),
    }
}

fn tool_add_contact(ctx: &McpContext, args: &Value) -> Value {
    let name = match args.get("name").and_then(|v| v.as_str()) {
        Some(n) => n.to_string(),
        None => {
            return json!({"content": [{"type": "text", "text": "Missing 'name'"}], "isError": true})
        }
    };
    let address = match args.get("address").and_then(|v| v.as_str()) {
        Some(a) => a.to_string(),
        None => {
            return json!({"content": [{"type": "text", "text": "Missing 'address'"}], "isError": true})
        }
    };
    let psk_str = match args.get("psk").and_then(|v| v.as_str()) {
        Some(p) => p.to_string(),
        None => {
            return json!({"content": [{"type": "text", "text": "Missing 'psk'"}], "isError": true})
        }
    };

    if let Err(e) = wallet::decode_address(&address) {
        return json!({"content": [{"type": "text", "text": format!("Invalid address: {}", e)}], "isError": true});
    }

    let psk_bytes = match crate::contacts::parse_psk(&psk_str) {
        Ok(b) => b,
        Err(e) => {
            return json!({"content": [{"type": "text", "text": format!("Invalid PSK: {}", e)}], "isError": true})
        }
    };

    let data_path = std::path::Path::new(&ctx.data_dir);
    if let Err(e) = std::fs::create_dir_all(data_path) {
        return json!({"content": [{"type": "text", "text": format!("Failed to create data dir: {}", e)}], "isError": true});
    }

    let path = data_path.join("contacts.db");
    match ContactStore::open(&path) {
        Ok(store) => match store.add(&name, &address, &psk_bytes) {
            Ok(()) => json!({
                "content": [{"type": "text", "text": format!("Added contact: {} ({})", name, address)}]
            }),
            Err(e) => {
                json!({"content": [{"type": "text", "text": format!("Failed to add: {}", e)}], "isError": true})
            }
        },
        Err(e) => {
            json!({"content": [{"type": "text", "text": format!("Failed to open contacts: {}", e)}], "isError": true})
        }
    }
}

fn tool_remove_contact(ctx: &McpContext, args: &Value) -> Value {
    let name = match args.get("name").and_then(|v| v.as_str()) {
        Some(n) => n.to_string(),
        None => {
            return json!({"content": [{"type": "text", "text": "Missing 'name'"}], "isError": true})
        }
    };
    let path = std::path::Path::new(&ctx.data_dir).join("contacts.db");
    match ContactStore::open(&path) {
        Ok(store) => match store.remove(&name) {
            Ok(true) => {
                json!({"content": [{"type": "text", "text": format!("Removed contact: {}", name)}]})
            }
            Ok(false) => {
                json!({"content": [{"type": "text", "text": format!("Contact '{}' not found", name)}]})
            }
            Err(e) => {
                json!({"content": [{"type": "text", "text": format!("Error: {}", e)}], "isError": true})
            }
        },
        Err(e) => {
            json!({"content": [{"type": "text", "text": format!("Failed to open contacts: {}", e)}], "isError": true})
        }
    }
}

fn tool_list_messages(ctx: &McpContext, args: &Value) -> Value {
    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as i64;
    let from_filter = args
        .get("from")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let data_path = std::path::Path::new(&ctx.data_dir);
    let messages_db = data_path.join("messages.db");
    if !messages_db.exists() {
        return json!({"content": [{"type": "text", "text": "No messages cached yet."}]});
    }

    // Resolve contact name to address if needed
    let from_address = if let Some(ref from_str) = from_filter {
        let contacts_path = data_path.join("contacts.db");
        if contacts_path.exists() {
            if let Ok(store) = ContactStore::open(&contacts_path) {
                if let Ok(Some(contact)) = store.get(from_str) {
                    Some(contact.address)
                } else {
                    Some(from_str.clone())
                }
            } else {
                Some(from_str.clone())
            }
        } else {
            Some(from_str.clone())
        }
    } else {
        None
    };

    let conn = match rusqlite::Connection::open(&messages_db) {
        Ok(c) => c,
        Err(e) => {
            return json!({"content": [{"type": "text", "text": format!("Failed to open messages.db: {}", e)}], "isError": true})
        }
    };

    let query = if from_address.is_some() {
        "SELECT sender, content, timestamp_secs, direction FROM messages \
         WHERE participant = ?1 ORDER BY timestamp_secs DESC LIMIT ?2"
    } else {
        "SELECT sender, content, timestamp_secs, direction FROM messages \
         ORDER BY timestamp_secs DESC LIMIT ?1"
    };

    let rows_result: rusqlite::Result<Vec<(String, String, i64, String)>> = if let Some(ref addr) =
        from_address
    {
        let mut stmt = match conn.prepare(query) {
            Ok(s) => s,
            Err(e) => {
                return json!({"content": [{"type": "text", "text": format!("Query error: {}", e)}], "isError": true})
            }
        };
        stmt.query_map(rusqlite::params![addr, limit], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .and_then(|rows| rows.collect())
    } else {
        let mut stmt = match conn.prepare(query) {
            Ok(s) => s,
            Err(e) => {
                return json!({"content": [{"type": "text", "text": format!("Query error: {}", e)}], "isError": true})
            }
        };
        stmt.query_map(rusqlite::params![limit], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .and_then(|rows| rows.collect())
    };

    match rows_result {
        Ok(rows) if rows.is_empty() => {
            json!({"content": [{"type": "text", "text": "No messages found."}]})
        }
        Ok(rows) => {
            let list: Vec<Value> = rows
                .iter()
                .map(|(sender, content, ts, dir)| {
                    json!({
                        "from": sender,
                        "content": content,
                        "timestamp": ts,
                        "direction": dir,
                    })
                })
                .collect();
            json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&list).unwrap_or_default()
                }]
            })
        }
        Err(e) => {
            json!({"content": [{"type": "text", "text": format!("Query failed: {}", e)}], "isError": true})
        }
    }
}

fn tool_get_status(ctx: &McpContext) -> Value {
    let contact_count = {
        let path = std::path::Path::new(&ctx.data_dir).join("contacts.db");
        if path.exists() {
            ContactStore::open(&path)
                .ok()
                .and_then(|s| s.count().ok())
                .unwrap_or(0)
        } else {
            0
        }
    };

    json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&json!({
                "network": ctx.network.to_string(),
                "address": ctx.address,
                "algod_url": ctx.algod_url,
                "hub_url": ctx.hub_url,
                "contacts": contact_count,
            })).unwrap_or_default()
        }]
    })
}

async fn tool_send_message(ctx: &McpContext, args: &Value) -> Value {
    let to = match args.get("to").and_then(|v| v.as_str()) {
        Some(t) => t.to_string(),
        None => {
            return json!({"content": [{"type": "text", "text": "Missing 'to'"}], "isError": true})
        }
    };
    let message = match args.get("message").and_then(|v| v.as_str()) {
        Some(m) => m.to_string(),
        None => {
            return json!({"content": [{"type": "text", "text": "Missing 'message'"}], "isError": true})
        }
    };

    // Resolve recipient address (contact name or raw address)
    let recipient_address = {
        let contacts_path = std::path::Path::new(&ctx.data_dir).join("contacts.db");
        let mut resolved = None;
        if contacts_path.exists() {
            if let Ok(store) = ContactStore::open(&contacts_path) {
                if let Ok(Some(contact)) = store.get(&to) {
                    resolved = Some(contact.address.clone());
                }
            }
        }
        // If not found as contact name, treat as raw address
        resolved.unwrap_or_else(|| to.clone())
    };

    // Validate address
    if let Err(e) = wallet::decode_address(&recipient_address) {
        return json!({"content": [{"type": "text", "text": format!("Invalid address '{}': {}", recipient_address, e)}], "isError": true});
    }

    use crate::algorand::{HttpAlgodClient, HttpIndexerClient};
    use crate::storage::SqliteKeyStorage;
    use algochat::{AlgoChat, AlgoChatConfig, AlgorandConfig};
    use ed25519_dalek::SigningKey;

    let algo_config = AlgorandConfig::new(&ctx.algod_url, &ctx.algod_token)
        .with_indexer(&ctx.indexer_url, &ctx.indexer_token);
    let config = AlgoChatConfig::new(algo_config);

    let data_path = std::path::Path::new(&ctx.data_dir);
    let key_storage = match SqliteKeyStorage::open(data_path.join("keys.db")) {
        Ok(s) => s,
        Err(e) => {
            return json!({"content": [{"type": "text", "text": format!("Failed to open key storage: {}", e)}], "isError": true})
        }
    };
    let message_cache = match SqliteMessageCache::open(data_path.join("messages.db")) {
        Ok(c) => c,
        Err(e) => {
            return json!({"content": [{"type": "text", "text": format!("Failed to open message cache: {}", e)}], "isError": true})
        }
    };

    let algod = HttpAlgodClient::new(&ctx.algod_url, &ctx.algod_token);
    let indexer = HttpIndexerClient::new(&ctx.indexer_url, &ctx.indexer_token);

    let client = match AlgoChat::from_seed(
        &ctx.seed,
        &ctx.address,
        config,
        algod,
        indexer,
        key_storage,
        message_cache,
    )
    .await
    {
        Ok(c) => c,
        Err(e) => {
            return json!({"content": [{"type": "text", "text": format!("Failed to initialize AlgoChat: {}", e)}], "isError": true})
        }
    };

    // Register PSK contacts
    let contacts_path = data_path.join("contacts.db");
    if contacts_path.exists() {
        if let Ok(store) = ContactStore::open(&contacts_path) {
            if let Ok(contacts) = store.list() {
                for contact in &contacts {
                    let mut psk = [0u8; 32];
                    psk.copy_from_slice(&contact.psk);
                    let _ = client
                        .add_psk_contact(&contact.address, &psk, Some(contact.name.clone()))
                        .await;
                }
            }
        }
    }

    // Build a separate algod client for transaction submission
    let algod_for_tx = HttpAlgodClient::new(&ctx.algod_url, &ctx.algod_token);
    let signing_key = SigningKey::from_bytes(&ctx.seed);

    match crate::agent::send_reply(
        &client,
        &algod_for_tx,
        &ctx.address,
        &recipient_address,
        &message,
        &signing_key,
    )
    .await
    {
        Ok(txid) => json!({
            "content": [{
                "type": "text",
                "text": format!("Message sent to {}. Transaction ID: {}", recipient_address, txid)
            }]
        }),
        Err(e) => {
            json!({"content": [{"type": "text", "text": format!("Failed to send: {}", e)}], "isError": true})
        }
    }
}

// ---------------------------------------------------------------------------
// MCP server entry point
// ---------------------------------------------------------------------------

/// Run the MCP server over stdin/stdout.
#[allow(clippy::too_many_arguments)]
pub async fn cmd_mcp(
    network: Network,
    algod_url: Option<String>,
    algod_token: Option<String>,
    indexer_url: Option<String>,
    indexer_token: Option<String>,
    seed_hex: Option<String>,
    address: Option<String>,
    password: Option<String>,
    hub_url: String,
    data_dir: &str,
) -> Result<()> {
    let net = network.defaults();
    let algod_url = algod_url.unwrap_or(net.algod_url);
    let algod_token = algod_token.unwrap_or(net.algod_token);
    let indexer_url = indexer_url.unwrap_or(net.indexer_url);
    let indexer_token = indexer_token.unwrap_or(net.indexer_token);

    let (seed, agent_address) = load_identity(
        seed_hex.as_deref(),
        address.as_deref(),
        password.as_deref(),
        data_dir,
    )?;

    let ctx = McpContext {
        data_dir: data_dir.to_string(),
        network,
        algod_url,
        algod_token,
        indexer_url,
        indexer_token,
        seed,
        address: agent_address.clone(),
        hub_url,
    };

    info!(address = %agent_address, "MCP server starting on stdio");

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin).lines();
    let mut writer = stdout;

    while let Ok(Some(line)) = reader.next_line().await {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let req = match McpRequest::parse(&line) {
            Some(r) => r,
            None => {
                let _ = write_error(&mut writer, None, -32700, "Parse error").await;
                continue;
            }
        };

        info!(method = %req.method, "MCP request");

        match req.method.as_str() {
            "initialize" => {
                let result = json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": "corvid-agent-nano",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                });
                let _ = write_response(&mut writer, req.id, result).await;
            }

            "notifications/initialized" => {
                // No response needed for notifications
            }

            "tools/list" => {
                let _ = write_response(&mut writer, req.id, tool_list()).await;
            }

            "tools/call" => {
                let tool_name = req
                    .params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let tool_args = req
                    .params
                    .get("arguments")
                    .cloned()
                    .unwrap_or(Value::Object(Default::default()));

                if tool_name.is_empty() {
                    let _ = write_error(&mut writer, req.id, -32602, "Missing tool name").await;
                    continue;
                }

                let result = handle_tool_call(&ctx, tool_name, &tool_args).await;
                let _ = write_response(&mut writer, req.id, result).await;
            }

            "ping" => {
                let _ = write_response(&mut writer, req.id, json!({})).await;
            }

            other => {
                warn!(method = %other, "MCP: unsupported method");
                let _ = write_error(
                    &mut writer,
                    req.id,
                    -32601,
                    &format!("Method not found: {}", other),
                )
                .await;
            }
        }
    }

    info!("MCP server exiting (stdin closed)");
    Ok(())
}
