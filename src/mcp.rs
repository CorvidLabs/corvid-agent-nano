//! MCP (Model Context Protocol) server — exposes nano's capabilities as MCP
//! tools over stdio so that AI agents (Claude Code, Cursor, etc.) can drive
//! AlgoChat programmatically.
//!
//! Usage:
//! ```
//! can mcp --network testnet
//! ```
//!
//! Then in `settings.json`:
//! ```json
//! { "mcpServers": { "nano": { "command": "./can", "args": ["mcp", "--network", "testnet"] } } }
//! ```

use std::sync::Arc;

use algochat::{AlgoChat, AlgoChatConfig, AlgorandConfig};
use anyhow::Result;
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{debug, info, warn};

use crate::algorand::{HttpAlgodClient, HttpIndexerClient};
use crate::contacts::ContactStore;
use crate::storage::{SqliteKeyStorage, SqliteMessageCache};

// ---------------------------------------------------------------------------
// Concrete AlgoChat client type
// ---------------------------------------------------------------------------

type NanoAlgoChat =
    AlgoChat<HttpAlgodClient, HttpIndexerClient, SqliteKeyStorage, SqliteMessageCache>;

// ---------------------------------------------------------------------------
// Runtime context shared across all tool calls
// ---------------------------------------------------------------------------

pub struct McpContext {
    pub network: String,
    pub agent_address: String,
    pub data_dir: String,
    pub client: Arc<NanoAlgoChat>,
    pub algod: Arc<HttpAlgodClient>,
    pub signing_key: SigningKey,
}

// ---------------------------------------------------------------------------
// JSON-RPC 2.0 types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    method: String,
    #[serde(default)]
    params: Value,
    /// Present on requests; absent on notifications.
    id: Option<Value>,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
    id: Value,
}

#[derive(Debug, Serialize)]
struct RpcError {
    code: i32,
    message: String,
}

impl JsonRpcResponse {
    fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            result: Some(result),
            error: None,
            id,
        }
    }

    fn err(id: Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
            }),
            id,
        }
    }

    fn internal_err(id: Value, e: impl std::fmt::Display) -> Self {
        Self::err(id, -32603, format!("Internal error: {e}"))
    }

    fn invalid_params(id: Value, msg: impl Into<String>) -> Self {
        Self::err(id, -32602, msg)
    }
}

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

fn tools_list() -> Value {
    json!({
        "tools": [
            {
                "name": "send_message",
                "description": "Send an encrypted AlgoChat message to a contact or address",
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
                        "name": {
                            "type": "string",
                            "description": "Contact name"
                        },
                        "address": {
                            "type": "string",
                            "description": "Algorand address"
                        },
                        "psk": {
                            "type": "string",
                            "description": "Pre-shared key (hex or base64, 32 bytes)"
                        }
                    },
                    "required": ["name", "address", "psk"]
                }
            },
            {
                "name": "remove_contact",
                "description": "Remove a PSK contact by name",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Contact name"
                        }
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "list_messages",
                "description": "List recent messages from the local cache",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of messages to return (default 20)"
                        },
                        "contact": {
                            "type": "string",
                            "description": "Filter by contact name or address"
                        }
                    }
                }
            },
            {
                "name": "get_status",
                "description": "Get agent status including network, address, and balance",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            },
            {
                "name": "get_balance",
                "description": "Check the ALGO balance of the agent wallet",
                "inputSchema": {
                    "type": "object",
                    "properties": {}
                }
            }
        ]
    })
}

// ---------------------------------------------------------------------------
// Tool handlers
// ---------------------------------------------------------------------------

async fn handle_send_message(ctx: &McpContext, params: &Value) -> Result<Value, String> {
    let to = params["to"].as_str().ok_or("missing 'to'")?;
    let message = params["message"].as_str().ok_or("missing 'message'")?;

    // Resolve contact name → address if needed
    let contacts_path = std::path::Path::new(&ctx.data_dir).join("contacts.db");
    let recipient = if contacts_path.exists() {
        if let Ok(store) = ContactStore::open(&contacts_path) {
            if let Ok(Some(contact)) = store.get(to) {
                contact.address
            } else {
                to.to_string()
            }
        } else {
            to.to_string()
        }
    } else {
        to.to_string()
    };

    let txid = crate::agent::send_reply(
        &ctx.client,
        &*ctx.algod,
        &ctx.agent_address,
        &recipient,
        message,
        &ctx.signing_key,
    )
    .await
    .map_err(|e| format!("{e}"))?;

    Ok(json!({
        "content": [{
            "type": "text",
            "text": format!("Message sent. Transaction ID: {}", txid)
        }]
    }))
}

fn handle_list_contacts(ctx: &McpContext) -> Result<Value, String> {
    let contacts_path = std::path::Path::new(&ctx.data_dir).join("contacts.db");
    if !contacts_path.exists() {
        return Ok(json!({
            "content": [{"type": "text", "text": "No contacts configured."}]
        }));
    }

    let store = ContactStore::open(&contacts_path).map_err(|e| format!("{e}"))?;
    let contacts = store.list().map_err(|e| format!("{e}"))?;

    if contacts.is_empty() {
        return Ok(json!({
            "content": [{"type": "text", "text": "No contacts."}]
        }));
    }

    let list: Vec<Value> = contacts
        .iter()
        .map(|c| json!({"name": c.name, "address": c.address, "added_at": c.added_at}))
        .collect();

    Ok(json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&list).unwrap_or_default()
        }]
    }))
}

fn handle_add_contact(ctx: &McpContext, params: &Value) -> Result<Value, String> {
    let name = params["name"].as_str().ok_or("missing 'name'")?;
    let address = params["address"].as_str().ok_or("missing 'address'")?;
    let psk = params["psk"].as_str().ok_or("missing 'psk'")?;

    // Validate address
    crate::wallet::decode_address(address).map_err(|e| format!("Invalid address: {e}"))?;
    let psk_bytes = crate::contacts::parse_psk(psk).map_err(|e| format!("Invalid PSK: {e}"))?;

    let contacts_path = std::path::Path::new(&ctx.data_dir).join("contacts.db");
    let store = ContactStore::open(&contacts_path).map_err(|e| format!("{e}"))?;
    store
        .upsert(name, address, &psk_bytes)
        .map_err(|e| format!("{e}"))?;

    Ok(json!({
        "content": [{
            "type": "text",
            "text": format!("Added contact: {} ({})", name, address)
        }]
    }))
}

fn handle_remove_contact(ctx: &McpContext, params: &Value) -> Result<Value, String> {
    let name = params["name"].as_str().ok_or("missing 'name'")?;

    let contacts_path = std::path::Path::new(&ctx.data_dir).join("contacts.db");
    if !contacts_path.exists() {
        return Ok(json!({
            "content": [{"type": "text", "text": "No contacts configured."}]
        }));
    }

    let store = ContactStore::open(&contacts_path).map_err(|e| format!("{e}"))?;
    let removed = store.remove(name).map_err(|e| format!("{e}"))?;

    Ok(json!({
        "content": [{
            "type": "text",
            "text": if removed {
                format!("Removed contact: {}", name)
            } else {
                format!("Contact '{}' not found.", name)
            }
        }]
    }))
}

fn handle_list_messages(ctx: &McpContext, params: &Value) -> Result<Value, String> {
    use rusqlite::Connection;

    let data_path = std::path::Path::new(&ctx.data_dir);
    let messages_db = data_path.join("messages.db");

    if !messages_db.exists() {
        return Ok(json!({
            "content": [{"type": "text", "text": "No messages in cache. Run `can run` first."}]
        }));
    }

    let limit = params["limit"].as_u64().unwrap_or(20) as i64;
    let contact_filter = params["contact"].as_str();

    let conn = Connection::open(&messages_db).map_err(|e| format!("{e}"))?;

    // Optionally resolve contact name → address
    let addr_filter: Option<String> = if let Some(filter) = contact_filter {
        let contacts_path = data_path.join("contacts.db");
        if contacts_path.exists() {
            if let Ok(store) = ContactStore::open(&contacts_path) {
                if let Ok(Some(c)) = store.get(filter) {
                    Some(c.address)
                } else {
                    Some(filter.to_string())
                }
            } else {
                Some(filter.to_string())
            }
        } else {
            Some(filter.to_string())
        }
    } else {
        None
    };

    let (sql, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
        if let Some(ref addr) = addr_filter {
            (
                "SELECT sender, recipient, content, timestamp_secs, direction \
                 FROM messages WHERE participant = ?1 \
                 ORDER BY timestamp_secs DESC LIMIT ?2"
                    .to_string(),
                vec![
                    Box::new(addr.clone()) as Box<dyn rusqlite::types::ToSql>,
                    Box::new(limit),
                ],
            )
        } else {
            (
                "SELECT sender, recipient, content, timestamp_secs, direction \
                 FROM messages ORDER BY timestamp_secs DESC LIMIT ?1"
                    .to_string(),
                vec![Box::new(limit) as Box<dyn rusqlite::types::ToSql>],
            )
        };

    let mut stmt = conn.prepare(&sql).map_err(|e| format!("{e}"))?;
    let refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|p| &**p).collect();

    let rows = stmt
        .query_map(refs.as_slice(), |row| {
            Ok(json!({
                "sender": row.get::<_, String>(0)?,
                "recipient": row.get::<_, String>(1)?,
                "content": row.get::<_, String>(2)?,
                "timestamp_secs": row.get::<_, i64>(3)?,
                "direction": row.get::<_, String>(4)?
            }))
        })
        .map_err(|e| format!("{e}"))?;

    let mut messages: Vec<Value> = rows.filter_map(|r| r.ok()).collect();
    messages.reverse(); // show oldest first

    let text = serde_json::to_string_pretty(&messages).unwrap_or_default();
    Ok(json!({
        "content": [{"type": "text", "text": text}]
    }))
}

async fn handle_get_status(ctx: &McpContext) -> Result<Value, String> {
    use algochat::AlgodClient;

    let balance_info = ctx.algod.get_account_info(&ctx.agent_address).await;
    let (balance_algo, min_algo, algod_ok) = match balance_info {
        Ok(info) => (
            info.amount as f64 / 1_000_000.0,
            info.min_balance as f64 / 1_000_000.0,
            true,
        ),
        Err(_) => (0.0, 0.0, false),
    };

    let contacts_path = std::path::Path::new(&ctx.data_dir).join("contacts.db");
    let contact_count = if contacts_path.exists() {
        ContactStore::open(&contacts_path)
            .and_then(|s| s.count())
            .unwrap_or(0)
    } else {
        0
    };

    let status = json!({
        "network": ctx.network,
        "address": ctx.agent_address,
        "algod_connected": algod_ok,
        "balance_algo": balance_algo,
        "min_balance_algo": min_algo,
        "contact_count": contact_count
    });

    Ok(json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&status).unwrap_or_default()
        }]
    }))
}

async fn handle_get_balance(ctx: &McpContext) -> Result<Value, String> {
    use algochat::AlgodClient;

    let info = ctx
        .algod
        .get_account_info(&ctx.agent_address)
        .await
        .map_err(|e| format!("Failed to fetch balance: {e}"))?;

    let algo = info.amount as f64 / 1_000_000.0;
    let min_algo = info.min_balance as f64 / 1_000_000.0;

    Ok(json!({
        "content": [{
            "type": "text",
            "text": format!(
                "Balance: {:.6} ALGO (minimum required: {:.6} ALGO)\nAddress: {}",
                algo, min_algo, ctx.agent_address
            )
        }]
    }))
}

// ---------------------------------------------------------------------------
// Request dispatcher
// ---------------------------------------------------------------------------

async fn dispatch_tool(ctx: &McpContext, name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "send_message" => handle_send_message(ctx, args).await,
        "list_contacts" => handle_list_contacts(ctx),
        "add_contact" => handle_add_contact(ctx, args),
        "remove_contact" => handle_remove_contact(ctx, args),
        "list_messages" => handle_list_messages(ctx, args),
        "get_status" => handle_get_status(ctx).await,
        "get_balance" => handle_get_balance(ctx).await,
        _ => Err(format!("Unknown tool: {name}")),
    }
}

async fn handle_request(ctx: &McpContext, line: &str) -> Option<JsonRpcResponse> {
    let req: JsonRpcRequest = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "failed to parse JSON-RPC request");
            return Some(JsonRpcResponse::err(
                Value::Null,
                -32700,
                format!("Parse error: {e}"),
            ));
        }
    };

    debug!(method = %req.method, "MCP request");

    // Notifications (no id) don't get a response
    let id = match req.id {
        Some(id) => id,
        None => {
            debug!(method = %req.method, "notification — no response");
            return None;
        }
    };

    match req.method.as_str() {
        "initialize" => Some(JsonRpcResponse::ok(
            id,
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "corvid-agent-nano",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        )),

        "tools/list" => Some(JsonRpcResponse::ok(id, tools_list())),

        "tools/call" => {
            let tool_name = match req.params["name"].as_str() {
                Some(n) => n.to_string(),
                None => {
                    return Some(JsonRpcResponse::invalid_params(id, "missing 'name'"));
                }
            };
            let args = &req.params["arguments"];

            match dispatch_tool(ctx, &tool_name, args).await {
                Ok(result) => Some(JsonRpcResponse::ok(id, result)),
                Err(msg) => Some(JsonRpcResponse::internal_err(id, msg)),
            }
        }

        "ping" => Some(JsonRpcResponse::ok(id, json!({}))),

        method => {
            warn!(method, "unknown MCP method");
            Some(JsonRpcResponse::err(
                id,
                -32601,
                format!("Method not found: {method}"),
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// MCP server entry point
// ---------------------------------------------------------------------------

/// Run the MCP server over stdio until EOF.
pub async fn run_mcp_server(ctx: McpContext) -> Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    info!(
        network = %ctx.network,
        address = %ctx.agent_address,
        "MCP server started (stdio transport)"
    );

    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut lines = BufReader::new(stdin).lines();

    while let Some(line) = lines.next_line().await? {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        if let Some(response) = handle_request(&ctx, &line).await {
            let json = serde_json::to_string(&response)?;
            stdout.write_all(json.as_bytes()).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
    }

    info!("MCP server: stdin closed, shutting down");
    Ok(())
}

/// Build an McpContext from the provided credentials and network config.
#[allow(clippy::too_many_arguments)]
pub async fn build_context(
    network_name: String,
    algod_url: String,
    algod_token: String,
    indexer_url: String,
    indexer_token: String,
    seed: [u8; 32],
    agent_address: String,
    data_dir: String,
) -> Result<McpContext> {
    let signing_key = SigningKey::from_bytes(&seed);

    let data_path = std::path::Path::new(&data_dir);
    std::fs::create_dir_all(data_path)?;

    // Create two algod instances: one for AlgoChat, one retained for tool calls.
    let algod_for_chat = HttpAlgodClient::new(&algod_url, &algod_token);
    let algod_for_ctx = HttpAlgodClient::new(&algod_url, &algod_token);
    let indexer = HttpIndexerClient::new(&indexer_url, &indexer_token);

    let algo_config =
        AlgorandConfig::new(&algod_url, &algod_token).with_indexer(&indexer_url, &indexer_token);
    let config = AlgoChatConfig::new(algo_config);

    let key_storage = SqliteKeyStorage::open(data_path.join("keys.db"))
        .map_err(|e| anyhow::anyhow!("Failed to open key storage: {e}"))?;
    let message_cache = SqliteMessageCache::open(data_path.join("messages.db"))
        .map_err(|e| anyhow::anyhow!("Failed to open message cache: {e}"))?;

    let client: NanoAlgoChat = AlgoChat::from_seed(
        &seed,
        &agent_address,
        config,
        algod_for_chat,
        indexer,
        key_storage,
        message_cache,
    )
    .await
    .map_err(|e| anyhow::anyhow!("Failed to initialize AlgoChat: {e}"))?;

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

    Ok(McpContext {
        network: network_name,
        agent_address,
        data_dir,
        client: Arc::new(client),
        algod: Arc::new(algod_for_ctx),
        signing_key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_list_has_required_tools() {
        let list = tools_list();
        let tools = list["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"send_message"));
        assert!(names.contains(&"list_contacts"));
        assert!(names.contains(&"add_contact"));
        assert!(names.contains(&"remove_contact"));
        assert!(names.contains(&"list_messages"));
        assert!(names.contains(&"get_status"));
        assert!(names.contains(&"get_balance"));
    }

    #[test]
    fn tools_list_schemas_have_required_fields() {
        let list = tools_list();
        let tools = list["tools"].as_array().unwrap();
        for tool in tools {
            assert!(tool["name"].is_string(), "tool missing name");
            assert!(tool["description"].is_string(), "tool missing description");
            assert!(tool["inputSchema"].is_object(), "tool missing inputSchema");
        }
    }

    #[test]
    fn json_rpc_response_ok_serializes() {
        let resp = JsonRpcResponse::ok(json!(1), json!({"status": "ok"}));
        let s = serde_json::to_string(&resp).unwrap();
        assert!(s.contains("\"result\""));
        assert!(!s.contains("\"error\""));
        assert!(s.contains("\"id\":1"));
    }

    #[test]
    fn json_rpc_response_err_serializes() {
        let resp = JsonRpcResponse::err(json!(2), -32601, "method not found");
        let s = serde_json::to_string(&resp).unwrap();
        assert!(s.contains("\"error\""));
        assert!(!s.contains("\"result\""));
        assert!(s.contains("-32601"));
    }

    #[tokio::test]
    async fn handle_request_returns_none_for_notification() {
        // A notification has no "id" field — simulate via raw parsing
        // We create a minimal McpContext-less test by checking the parse path
        let notification = r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#;
        let req: JsonRpcRequest = serde_json::from_str(notification).unwrap();
        assert!(req.id.is_none());
    }

    #[tokio::test]
    async fn tools_list_response_structure() {
        let list = tools_list();
        let tools = list["tools"].as_array().expect("tools should be array");
        assert!(!tools.is_empty());
    }
}
