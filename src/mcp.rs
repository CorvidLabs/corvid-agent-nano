//! MCP (Model Context Protocol) stdio server.
//!
//! Exposes agent tools over JSON-RPC 2.0 on stdin/stdout, making corvid-agent-nano
//! usable as an MCP server by Claude Code, Cursor, and other MCP clients.
//!
//! Start with: `can mcp [--network testnet] [--password <pw>]`

use anyhow::{bail, Result};
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use algochat::{AlgoChat, AlgoChatConfig, AlgorandConfig};

use crate::algorand::{HttpAlgodClient, HttpIndexerClient};
use crate::contacts::ContactStore;
use crate::storage::{SqliteKeyStorage, SqliteMessageCache};

// ── Config ───────────────────────────────────────────────────────────────────

/// Configuration for the MCP server (mirrors Run command options).
#[derive(Debug, Clone)]
pub struct McpConfig {
    pub data_dir: String,
    pub algod_url: String,
    pub algod_token: String,
    pub indexer_url: String,
    pub indexer_token: String,
    /// Optional override: hex-encoded 32-byte Ed25519 seed (skips keystore).
    pub seed_hex: Option<String>,
    /// Optional override: Algorand address (used with seed_hex).
    pub address: Option<String>,
    /// Keystore password (required for send_message if seed_hex is not set).
    pub password: Option<String>,
}

// ── JSON-RPC types ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct RpcRequest {
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    method: String,
    params: Option<Value>,
    id: Option<Value>,
}

#[derive(Debug, Serialize)]
struct RpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
    id: Option<Value>,
}

#[derive(Debug, Serialize)]
struct RpcError {
    code: i32,
    message: String,
}

impl RpcResponse {
    fn ok(id: Option<Value>, result: Value) -> Self {
        Self { jsonrpc: "2.0", result: Some(result), error: None, id }
    }
    fn err(id: Option<Value>, code: i32, msg: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            result: None,
            error: Some(RpcError { code, message: msg.into() }),
            id,
        }
    }
}

// ── MCP content helpers ───────────────────────────────────────────────────────

fn text_result(text: impl Into<String>) -> Value {
    serde_json::json!({ "content": [{ "type": "text", "text": text.into() }] })
}

fn error_content(msg: impl Into<String>) -> Value {
    serde_json::json!({
        "content": [{ "type": "text", "text": msg.into() }],
        "isError": true
    })
}

// ── Server loop ───────────────────────────────────────────────────────────────

/// Run the MCP stdio server until EOF (client disconnected).
pub async fn run(config: McpConfig) -> Result<()> {
    let stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin);
    let mut line = String::new();

    loop {
        line.clear();
        if reader.read_line(&mut line).await? == 0 {
            break; // EOF — client disconnected
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let req: RpcRequest = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                let resp = RpcResponse::err(None, -32700, format!("Parse error: {e}"));
                emit(&mut stdout, &resp).await?;
                continue;
            }
        };

        // Notifications have no `id` — process but don't respond.
        let is_notification = req.id.is_none();
        let resp = dispatch(&config, req).await;

        if !is_notification {
            if let Some(r) = resp {
                emit(&mut stdout, &r).await?;
            }
        }
    }

    Ok(())
}

async fn emit(out: &mut tokio::io::Stdout, resp: &RpcResponse) -> Result<()> {
    let json = serde_json::to_string(resp)?;
    out.write_all(format!("{json}\n").as_bytes()).await?;
    out.flush().await?;
    Ok(())
}

// ── Dispatch ──────────────────────────────────────────────────────────────────

async fn dispatch(config: &McpConfig, req: RpcRequest) -> Option<RpcResponse> {
    let id = req.id.clone();
    let params = &req.params;

    Some(match req.method.as_str() {
        "initialize" => handle_initialize(id),
        // Notifications — no response
        "initialized" | "notifications/initialized" => return None,
        "ping" => RpcResponse::ok(id, serde_json::json!({})),
        "tools/list" => handle_tools_list(id),
        "tools/call" => handle_tools_call(config, id, params).await,
        other => RpcResponse::err(id, -32601, format!("Method not found: {other}")),
    })
}

fn handle_initialize(id: Option<Value>) -> RpcResponse {
    RpcResponse::ok(
        id,
        serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": { "tools": {} },
            "serverInfo": {
                "name": "corvid-agent-nano",
                "version": env!("CARGO_PKG_VERSION")
            }
        }),
    )
}

fn handle_tools_list(id: Option<Value>) -> RpcResponse {
    RpcResponse::ok(
        id,
        serde_json::json!({
            "tools": [
                {
                    "name": "agent_info",
                    "description": "Show agent identity, wallet address, contacts count, and cached message count.",
                    "inputSchema": { "type": "object", "properties": {}, "required": [] }
                },
                {
                    "name": "list_contacts",
                    "description": "List all saved contacts with their name and Algorand address.",
                    "inputSchema": { "type": "object", "properties": {}, "required": [] }
                },
                {
                    "name": "get_inbox",
                    "description": "Read recent messages from the local message cache.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "limit": {
                                "type": "integer",
                                "description": "Maximum number of messages to return (default 20)"
                            },
                            "from": {
                                "type": "string",
                                "description": "Filter by contact name or Algorand address"
                            }
                        },
                        "required": []
                    }
                },
                {
                    "name": "check_balance",
                    "description": "Check the agent's ALGO balance on-chain.",
                    "inputSchema": { "type": "object", "properties": {}, "required": [] }
                },
                {
                    "name": "send_message",
                    "description": "Send an encrypted AlgoChat message to a contact or Algorand address.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "to": {
                                "type": "string",
                                "description": "Recipient: contact name or Algorand address"
                            },
                            "message": {
                                "type": "string",
                                "description": "Message text to send"
                            }
                        },
                        "required": ["to", "message"]
                    }
                }
            ]
        }),
    )
}

async fn handle_tools_call(
    config: &McpConfig,
    id: Option<Value>,
    params: &Option<Value>,
) -> RpcResponse {
    let Some(params) = params else {
        return RpcResponse::err(id, -32602, "Missing params");
    };

    let Some(name) = params.get("name").and_then(|v| v.as_str()) else {
        return RpcResponse::err(id, -32602, "Missing tool name");
    };

    let args = params.get("arguments").cloned().unwrap_or_default();

    let result = match name {
        "agent_info" => tool_agent_info(config),
        "list_contacts" => tool_list_contacts(config),
        "get_inbox" => tool_get_inbox(config, &args),
        "check_balance" => tool_check_balance(config).await,
        "send_message" => tool_send_message(config, &args).await,
        other => Err(format!("Unknown tool: {other}")),
    };

    match result {
        Ok(output) => RpcResponse::ok(id, text_result(output)),
        Err(e) => RpcResponse::ok(id, error_content(e)),
    }
}

// ── Tool implementations ──────────────────────────────────────────────────────

fn tool_agent_info(config: &McpConfig) -> Result<String, String> {
    let data_path = std::path::Path::new(&config.data_dir);
    let ks_path = data_path.join("keystore.enc");

    let mut out = String::from("Corvid Agent CAN\n");
    out.push_str(&format!("  Data dir: {}\n", config.data_dir));

    if ks_path.exists() {
        match crate::keystore::keystore_address(&ks_path) {
            Ok(addr) => out.push_str(&format!("  Address:  {}\n", addr)),
            Err(e) => out.push_str(&format!("  Address:  (error reading keystore: {e})\n")),
        }
    } else if let Some(ref addr) = config.address {
        out.push_str(&format!("  Address:  {} (from --address flag)\n", addr));
    } else {
        out.push_str("  Address:  not configured (run `can setup`)\n");
    }

    let contacts_path = data_path.join("contacts.db");
    if contacts_path.exists() {
        if let Ok(store) = ContactStore::open(&contacts_path) {
            if let Ok(count) = store.count() {
                out.push_str(&format!("  Contacts: {}\n", count));
            }
        }
    } else {
        out.push_str("  Contacts: 0\n");
    }

    let messages_db = data_path.join("messages.db");
    if messages_db.exists() {
        if let Ok(conn) = rusqlite::Connection::open(&messages_db) {
            if let Ok(n) =
                conn.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get::<_, i64>(0))
            {
                out.push_str(&format!("  Messages: {} cached\n", n));
            }
        }
    } else {
        out.push_str("  Messages: 0 cached\n");
    }

    Ok(out)
}

fn tool_list_contacts(config: &McpConfig) -> Result<String, String> {
    let contacts_path = std::path::Path::new(&config.data_dir).join("contacts.db");

    if !contacts_path.exists() {
        return Ok("No contacts. Add one with `can contacts add`.".to_string());
    }

    let store = ContactStore::open(&contacts_path)
        .map_err(|e| format!("Failed to open contacts: {e}"))?;
    let contacts = store.list().map_err(|e| format!("Failed to list contacts: {e}"))?;

    if contacts.is_empty() {
        return Ok(
            "No contacts. Add one with `can contacts add --name <n> --address <addr> --psk <key>`"
                .to_string(),
        );
    }

    let mut out = format!("{:<16} {:<60} ADDED\n", "NAME", "ADDRESS");
    out.push_str(&"-".repeat(90));
    out.push('\n');
    for c in &contacts {
        out.push_str(&format!("{:<16} {:<60} {}\n", c.name, c.address, c.added_at));
    }
    out.push_str(&format!("\n{} contact(s)", contacts.len()));
    Ok(out)
}

fn tool_get_inbox(config: &McpConfig, args: &Value) -> Result<String, String> {
    let data_path = std::path::Path::new(&config.data_dir);
    let messages_db = data_path.join("messages.db");

    if !messages_db.exists() {
        return Ok("No messages yet. Run `can run` to start receiving messages.".to_string());
    }

    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
    let from_filter = args.get("from").and_then(|v| v.as_str());

    let conn = rusqlite::Connection::open(&messages_db)
        .map_err(|e| format!("Failed to open messages DB: {e}"))?;

    let contacts_path = data_path.join("contacts.db");
    let contact_store = if contacts_path.exists() { ContactStore::open(&contacts_path).ok() } else { None };

    // Resolve `from` filter to an address
    let from_address: Option<String> = if let Some(from_str) = from_filter {
        if let Some(store) = &contact_store {
            store
                .get(from_str)
                .ok()
                .flatten()
                .map(|c| c.address)
                .or_else(|| Some(from_str.to_string()))
        } else {
            Some(from_str.to_string())
        }
    } else {
        None
    };

    // Build query
    let (query, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
        if let Some(ref addr) = from_address {
            (
                "SELECT sender, recipient, content, timestamp_secs, confirmed_round, direction \
                 FROM messages WHERE participant = ?1 ORDER BY timestamp_secs DESC LIMIT ?2"
                    .to_string(),
                vec![
                    Box::new(addr.clone()) as Box<dyn rusqlite::types::ToSql>,
                    Box::new(limit as i64),
                ],
            )
        } else {
            (
                "SELECT sender, recipient, content, timestamp_secs, confirmed_round, direction \
                 FROM messages ORDER BY timestamp_secs DESC LIMIT ?1"
                    .to_string(),
                vec![Box::new(limit as i64) as Box<dyn rusqlite::types::ToSql>],
            )
        };

    let mut stmt = conn.prepare(&query).map_err(|e| format!("Query prepare failed: {e}"))?;
    let params_refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|p| &**p).collect();
    let rows = stmt
        .query_map(params_refs.as_slice(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, u64>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|e| format!("Query failed: {e}"))?;

    let mut messages: Vec<_> = rows.flatten().collect();

    if messages.is_empty() {
        return Ok("Inbox is empty.".to_string());
    }

    messages.reverse(); // Display oldest first

    let resolve_name = |addr: &str| -> String {
        if let Some(store) = &contact_store {
            if let Ok(Some(contact)) = store.get_by_address(addr) {
                return contact.name;
            }
        }
        if addr.len() > 12 { format!("{}...", &addr[..12]) } else { addr.to_string() }
    };

    let mut out =
        format!("{:<6} {:<5} {:<16} {:<16} MESSAGE\n", "ROUND", "DIR", "FROM/TO", "TIME");
    out.push_str(&"-".repeat(75));
    out.push('\n');

    for (sender, recipient, content, timestamp_secs, confirmed_round, direction) in &messages {
        let dir_label = if direction == "sent" { ">>>" } else { "<<<" };
        let peer =
            if direction == "sent" { resolve_name(recipient) } else { resolve_name(sender) };
        let time_str = chrono::DateTime::from_timestamp(*timestamp_secs, 0)
            .map(|dt| dt.format("%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "?".to_string());
        let display_content =
            if content.len() > 55 { format!("{}...", &content[..52]) } else { content.clone() };

        out.push_str(&format!(
            "{:<6} {:<5} {:<16} {:<16} {}\n",
            confirmed_round, dir_label, peer, time_str, display_content
        ));
    }

    out.push_str(&format!("\n{} message(s)", messages.len()));
    Ok(out)
}

async fn tool_check_balance(config: &McpConfig) -> Result<String, String> {
    use algochat::AlgodClient;

    let addr = resolve_address(config).map_err(|e| e.to_string())?;
    let algod = HttpAlgodClient::new(&config.algod_url, &config.algod_token);

    match algod.get_account_info(&addr).await {
        Ok(info) => {
            let algo = info.amount as f64 / 1_000_000.0;
            let min_algo = info.min_balance as f64 / 1_000_000.0;
            let mut out = format!("Address: {}\n", addr);
            out.push_str(&format!("Balance: {:.6} ALGO\n", algo));
            out.push_str(&format!("Min balance: {:.6} ALGO\n", min_algo));
            if info.amount < 100_000 {
                out.push_str(
                    "WARNING: Balance is very low — may not be able to send messages.\n",
                );
            }
            Ok(out)
        }
        Err(e) => Err(format!("Failed to get balance: {e}")),
    }
}

async fn tool_send_message(config: &McpConfig, args: &Value) -> Result<String, String> {
    let to = args
        .get("to")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing 'to' argument".to_string())?;
    let message_text = args
        .get("message")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing 'message' argument".to_string())?;

    // Load identity (seed + address)
    let (seed, agent_address) = load_identity(config).map_err(|e| e.to_string())?;
    let signing_key = SigningKey::from_bytes(&seed);

    let data_path = std::path::Path::new(&config.data_dir);
    std::fs::create_dir_all(data_path).map_err(|e| format!("Cannot create data dir: {e}"))?;

    // Initialize storage
    let key_storage = SqliteKeyStorage::open(data_path.join("keys.db"))
        .map_err(|e| format!("Failed to open key storage: {e}"))?;
    let message_cache = SqliteMessageCache::open(data_path.join("messages.db"))
        .map_err(|e| format!("Failed to open message cache: {e}"))?;

    // Build Algorand clients
    let indexer = HttpIndexerClient::new(&config.indexer_url, &config.indexer_token);
    let algo_config = AlgorandConfig::new(&config.algod_url, &config.algod_token)
        .with_indexer(&config.indexer_url, &config.indexer_token);
    let chat_config = AlgoChatConfig::new(algo_config);

    // Initialize AlgoChat client
    let client: AlgoChat<HttpAlgodClient, HttpIndexerClient, SqliteKeyStorage, SqliteMessageCache> =
        AlgoChat::from_seed(
            &seed,
            &agent_address,
            chat_config,
            HttpAlgodClient::new(&config.algod_url, &config.algod_token),
            indexer,
            key_storage,
            message_cache,
        )
        .await
        .map_err(|e| format!("Failed to initialize AlgoChat: {e}"))?;

    // Register PSK contacts
    let contacts_path = data_path.join("contacts.db");
    let contact_store: Option<ContactStore> = if contacts_path.exists() {
        ContactStore::open(&contacts_path).ok()
    } else {
        None
    };

    if let Some(store) = &contact_store {
        for contact in store.list().unwrap_or_default() {
            let mut psk = [0u8; 32];
            psk.copy_from_slice(&contact.psk);
            let _ = client
                .add_psk_contact(&contact.address, &psk, Some(contact.name.clone()))
                .await;
        }
    }

    // Resolve recipient address
    let recipient_address: String = if let Some(store) = &contact_store {
        if let Ok(Some(contact)) = store.get(to) {
            contact.address
        } else if let Ok(Some(contact)) = store.get_by_address(to) {
            contact.address
        } else {
            crate::wallet::decode_address(to)
                .map_err(|e| format!("Invalid address '{}': {e}", to))?;
            to.to_string()
        }
    } else {
        crate::wallet::decode_address(to)
            .map_err(|e| format!("Invalid address '{}': {e}", to))?;
        to.to_string()
    };

    // Send the message
    let algod_for_tx = HttpAlgodClient::new(&config.algod_url, &config.algod_token);
    let txid = crate::agent::send_reply(
        &client,
        &algod_for_tx,
        &agent_address,
        &recipient_address,
        message_text,
        &signing_key,
    )
    .await
    .map_err(|e| format!("Failed to send message: {e}"))?;

    Ok(format!(
        "Message sent!\n  To:   {}\n  TxID: {}\n  Size: {} chars",
        recipient_address,
        txid,
        message_text.len()
    ))
}

// ── Identity helpers ──────────────────────────────────────────────────────────

/// Resolve the agent's Algorand address without decrypting the keystore.
fn resolve_address(config: &McpConfig) -> Result<String> {
    if let Some(ref addr) = config.address {
        return Ok(addr.clone());
    }
    let ks_path = std::path::Path::new(&config.data_dir).join("keystore.enc");
    if ks_path.exists() {
        return crate::keystore::keystore_address(&ks_path);
    }
    bail!("No wallet configured. Run `can setup` or provide --address.")
}

/// Load seed + address from config. Requires password if keystore is used.
fn load_identity(config: &McpConfig) -> Result<([u8; 32], String)> {
    // Direct seed from CLI flags
    if let Some(ref seed_str) = config.seed_hex {
        let seed_bytes =
            hex::decode(seed_str).map_err(|e| anyhow::anyhow!("Invalid seed hex: {e}"))?;
        if seed_bytes.len() != 32 {
            bail!("Seed must be 32 bytes (64 hex chars), got {}", seed_bytes.len());
        }
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&seed_bytes);
        let addr = match &config.address {
            Some(a) => a.clone(),
            None => crate::wallet::address_from_seed(&seed),
        };
        return Ok((seed, addr));
    }

    // Load from keystore
    let ks_path = std::path::Path::new(&config.data_dir).join("keystore.enc");
    if !crate::keystore::keystore_exists(&ks_path) {
        bail!("No wallet configured. Run `can setup` first.");
    }

    let pw = match &config.password {
        Some(p) => p.clone(),
        None => bail!(
            "Wallet is encrypted. Restart `can mcp` with --password <pw> to enable send_message."
        ),
    };

    crate::keystore::load_keystore(&ks_path, &pw)
}
