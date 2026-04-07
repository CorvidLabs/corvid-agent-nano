//! A2A (Agent-to-Agent) HTTP server.
//!
//! Implements the A2A protocol so other agents can interact with this nano
//! agent directly over HTTP without going through AlgoChat on-chain polling.
//!
//! Endpoints:
//! - `GET  /.well-known/agent.json` — Agent card (discovery)
//! - `POST /a2a/tasks`              — Create a new task
//! - `GET  /a2a/tasks`              — List recent tasks
//! - `GET  /a2a/tasks/{id}`         — Get task status
//! - `DELETE /a2a/tasks/{id}`       — Cancel a task

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Agent card
// ---------------------------------------------------------------------------

/// A2A Agent Card served at `/.well-known/agent.json`.
#[derive(Debug, Clone, Serialize)]
pub struct AgentCard {
    pub name: String,
    pub version: String,
    pub description: String,
    pub url: String,
    pub capabilities: AgentCapabilities,
    pub skills: Vec<AgentSkill>,
    pub authentication: AgentAuth,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentCapabilities {
    pub streaming: bool,
    #[serde(rename = "pushNotifications")]
    pub push_notifications: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentSkill {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentAuth {
    pub schemes: Vec<String>,
}

// ---------------------------------------------------------------------------
// Task types
// ---------------------------------------------------------------------------

/// Task state lifecycle: pending → running → completed/failed/cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskState {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// An A2A task.
#[derive(Debug, Clone, Serialize)]
pub struct Task {
    pub id: String,
    pub state: TaskState,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
    pub created_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

/// Request body for `POST /a2a/tasks`.
#[derive(Debug, Deserialize)]
pub struct CreateTaskRequest {
    pub message: String,
    #[serde(default = "default_timeout")]
    #[serde(rename = "timeoutMs")]
    pub timeout_ms: u64,
    /// Optional sender identifier.
    #[serde(default)]
    pub from: Option<String>,
}

fn default_timeout() -> u64 {
    300_000
}

/// Hub-compatible task request (forwarded to corvid-agent hub).
#[derive(Debug, Serialize)]
struct HubTaskRequest {
    message: String,
    #[serde(rename = "timeoutMs")]
    timeout_ms: u64,
}

/// Hub task creation response.
#[derive(Debug, Deserialize)]
struct HubTaskResponse {
    id: String,
    #[allow(dead_code)]
    state: String,
}

/// Hub task status response.
#[derive(Debug, Deserialize)]
struct HubTaskStatus {
    state: String,
    #[serde(default)]
    response: Option<String>,
}

// ---------------------------------------------------------------------------
// Task store (in-memory)
// ---------------------------------------------------------------------------

/// In-memory task store with configurable capacity.
#[derive(Debug)]
pub struct TaskStore {
    tasks: HashMap<String, Task>,
    /// Task IDs ordered by creation time (oldest first).
    order: Vec<String>,
    capacity: usize,
}

impl TaskStore {
    pub fn new(capacity: usize) -> Self {
        Self {
            tasks: HashMap::new(),
            order: Vec::new(),
            capacity,
        }
    }

    pub fn insert(&mut self, task: Task) {
        // Evict oldest if at capacity
        while self.order.len() >= self.capacity {
            if let Some(oldest_id) = self.order.first().cloned() {
                self.tasks.remove(&oldest_id);
                self.order.remove(0);
            }
        }
        self.order.push(task.id.clone());
        self.tasks.insert(task.id.clone(), task);
    }

    pub fn get(&self, id: &str) -> Option<&Task> {
        self.tasks.get(id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Task> {
        self.tasks.get_mut(id)
    }

    pub fn list(&self, limit: usize) -> Vec<&Task> {
        self.order
            .iter()
            .rev()
            .take(limit)
            .filter_map(|id| self.tasks.get(id))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Server config
// ---------------------------------------------------------------------------

/// Configuration for the A2A server.
pub struct A2aServerConfig {
    pub port: u16,
    pub agent_name: String,
    pub agent_address: String,
    pub hub_url: Option<String>,
    pub network: String,
    /// Pre-shared key for authentication (optional).
    pub psk: Option<String>,
}

// ---------------------------------------------------------------------------
// A2A HTTP server
// ---------------------------------------------------------------------------

/// Start the A2A HTTP server.
///
/// This runs forever, accepting connections and routing requests.
pub async fn serve_a2a(config: A2aServerConfig) -> anyhow::Result<()> {
    let listener = TcpListener::bind(format!("0.0.0.0:{}", config.port)).await?;
    info!(port = config.port, "A2A server listening");

    let agent_card = AgentCard {
        name: config.agent_name.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        description: format!(
            "Corvid Agent CAN ({}) — lightweight AlgoChat agent [{}]",
            config.network,
            &config.agent_address[..8.min(config.agent_address.len())]
        ),
        url: format!("http://localhost:{}", config.port),
        capabilities: AgentCapabilities {
            streaming: false,
            push_notifications: false,
        },
        skills: build_skills(&config.hub_url),
        authentication: AgentAuth {
            schemes: if config.psk.is_some() {
                vec!["bearer".into()]
            } else {
                vec![]
            },
        },
    };

    let card_json = serde_json::to_string_pretty(&agent_card)?;
    let store = Arc::new(RwLock::new(TaskStore::new(1000)));
    let http = Client::new();
    let hub_url = config.hub_url.clone();
    let psk = config.psk.clone();

    loop {
        let (mut stream, peer) = listener.accept().await?;

        let card_json = card_json.clone();
        let store = Arc::clone(&store);
        let http = http.clone();
        let hub_url = hub_url.clone();
        let psk = psk.clone();

        tokio::spawn(async move {
            // Read the full HTTP request (up to 64KB)
            let mut buf = vec![0u8; 65536];
            let n = match stream.read(&mut buf).await {
                Ok(0) => return,
                Ok(n) => n,
                Err(e) => {
                    warn!(peer = %peer, error = %e, "a2a: read error");
                    return;
                }
            };
            buf.truncate(n);

            let request = String::from_utf8_lossy(&buf);
            let (method, path, headers, body) = match parse_http_request(&request) {
                Some(parsed) => parsed,
                None => {
                    let _ = write_response(&mut stream, 400, r#"{"error":"bad request"}"#).await;
                    return;
                }
            };

            // PSK authentication check
            if let Some(ref expected_psk) = psk {
                let auth_header = headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
                    .map(|(_, v)| v.as_str());

                let authenticated = match auth_header {
                    Some(h) if h.starts_with("Bearer ") => &h[7..] == expected_psk.as_str(),
                    _ => false,
                };

                // Agent card is public (allows discovery)
                if !authenticated && path != "/.well-known/agent.json" {
                    let _ =
                        write_response(&mut stream, 401, r#"{"error":"unauthorized"}"#).await;
                    return;
                }
            }

            // Route request
            let (status, response_body) = route(
                &method,
                &path,
                &body,
                &card_json,
                &store,
                &http,
                hub_url.as_deref(),
            )
            .await;

            if let Err(e) = write_response(&mut stream, status, &response_body).await {
                warn!(peer = %peer, error = %e, "a2a: write error");
            }
        });
    }
}

/// Route an HTTP request to the appropriate handler.
async fn route(
    method: &str,
    path: &str,
    body: &str,
    card_json: &str,
    store: &Arc<RwLock<TaskStore>>,
    http: &Client,
    hub_url: Option<&str>,
) -> (u16, String) {
    match (method, path) {
        // Agent card
        ("GET", "/.well-known/agent.json") => (200, card_json.to_string()),

        // Health (convenience alias)
        ("GET", "/health") => {
            let body = serde_json::json!({"status": "healthy"});
            (200, body.to_string())
        }

        // Create task
        ("POST", "/a2a/tasks") => handle_create_task(body, store, http, hub_url).await,

        // List tasks
        ("GET", "/a2a/tasks") => handle_list_tasks(store).await,

        // Get/cancel task by ID
        (m, p) if p.starts_with("/a2a/tasks/") => {
            let id = &p["/a2a/tasks/".len()..];
            if id.is_empty() {
                return (400, r#"{"error":"missing task id"}"#.to_string());
            }
            match m {
                "GET" => handle_get_task(id, store).await,
                "DELETE" => handle_cancel_task(id, store).await,
                _ => (405, r#"{"error":"method not allowed"}"#.to_string()),
            }
        }

        // CORS preflight
        ("OPTIONS", _) => (204, String::new()),

        _ => (404, r#"{"error":"not found"}"#.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn handle_create_task(
    body: &str,
    store: &Arc<RwLock<TaskStore>>,
    http: &Client,
    hub_url: Option<&str>,
) -> (u16, String) {
    let req: CreateTaskRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            return (
                400,
                serde_json::json!({"error": format!("invalid request: {}", e)}).to_string(),
            );
        }
    };

    let task_id = generate_task_id();
    let now = now_secs();

    let task = Task {
        id: task_id.clone(),
        state: TaskState::Pending,
        message: req.message.clone(),
        response: None,
        created_at: now,
        completed_at: None,
        from: req.from.clone(),
    };

    {
        let mut s = store.write().await;
        s.insert(task);
    }

    info!(task_id = %task_id, from = ?req.from, "A2A task created");

    // Process task asynchronously
    let store = Arc::clone(store);
    let http = http.clone();
    let hub_url = hub_url.map(String::from);
    let message = req.message;
    let timeout_ms = req.timeout_ms;
    let id = task_id.clone();

    tokio::spawn(async move {
        process_task(&id, &message, timeout_ms, &store, &http, hub_url.as_deref()).await;
    });

    let response = serde_json::json!({
        "id": task_id,
        "state": "pending",
    });
    (201, response.to_string())
}

async fn handle_list_tasks(store: &Arc<RwLock<TaskStore>>) -> (u16, String) {
    let s = store.read().await;
    let tasks: Vec<&Task> = s.list(50);
    let body = serde_json::to_string(&tasks).unwrap_or_else(|_| "[]".to_string());
    (200, body)
}

async fn handle_get_task(id: &str, store: &Arc<RwLock<TaskStore>>) -> (u16, String) {
    let s = store.read().await;
    match s.get(id) {
        Some(task) => {
            let body = serde_json::to_string(task).unwrap_or_else(|_| "{}".to_string());
            (200, body)
        }
        None => (404, r#"{"error":"task not found"}"#.to_string()),
    }
}

async fn handle_cancel_task(id: &str, store: &Arc<RwLock<TaskStore>>) -> (u16, String) {
    let mut s = store.write().await;
    match s.get_mut(id) {
        Some(task) => {
            if task.state == TaskState::Pending || task.state == TaskState::Running {
                task.state = TaskState::Cancelled;
                task.completed_at = Some(now_secs());
                info!(task_id = %id, "A2A task cancelled");
                (200, r#"{"status":"cancelled"}"#.to_string())
            } else {
                (
                    409,
                    serde_json::json!({"error": format!("task already {}", serde_json::to_string(&task.state).unwrap_or_default().trim_matches('"'))}).to_string(),
                )
            }
        }
        None => (404, r#"{"error":"task not found"}"#.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Task processing
// ---------------------------------------------------------------------------

/// Process a task by forwarding to the hub or handling locally.
async fn process_task(
    task_id: &str,
    message: &str,
    _timeout_ms: u64,
    store: &Arc<RwLock<TaskStore>>,
    http: &Client,
    hub_url: Option<&str>,
) {
    // Mark as running
    {
        let mut s = store.write().await;
        if let Some(task) = s.get_mut(task_id) {
            if task.state == TaskState::Cancelled {
                return; // Already cancelled before we started
            }
            task.state = TaskState::Running;
        }
    }

    let result = if let Some(hub) = hub_url {
        // Forward to hub
        forward_to_hub_and_poll(http, hub, message).await
    } else {
        // No hub — echo mode (P2P)
        Ok(format!(
            "[nano] Received your message. Hub forwarding is disabled (P2P mode). Message: {}",
            truncate(message, 200)
        ))
    };

    // Update task with result
    let mut s = store.write().await;
    if let Some(task) = s.get_mut(task_id) {
        if task.state == TaskState::Cancelled {
            return; // Cancelled while processing
        }
        match result {
            Ok(response) => {
                task.state = TaskState::Completed;
                task.response = Some(response);
                task.completed_at = Some(now_secs());
                info!(task_id = %task_id, "A2A task completed");
            }
            Err(e) => {
                task.state = TaskState::Failed;
                task.response = Some(format!("error: {}", e));
                task.completed_at = Some(now_secs());
                warn!(task_id = %task_id, error = %e, "A2A task failed");
            }
        }
    }
}

/// Forward message to hub and poll for completion.
async fn forward_to_hub_and_poll(
    http: &Client,
    hub_url: &str,
    message: &str,
) -> Result<String, String> {
    let url = format!("{}/a2a/tasks/send", hub_url.trim_end_matches('/'));
    let payload = HubTaskRequest {
        message: message.to_string(),
        timeout_ms: 300_000,
    };

    // Submit to hub
    let resp = http
        .post(&url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("hub unreachable: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("hub rejected request: {}", resp.status()));
    }

    let hub_task: HubTaskResponse = resp
        .json()
        .await
        .map_err(|e| format!("hub response parse error: {}", e))?;

    debug!(hub_task_id = %hub_task.id, "forwarded to hub, polling...");

    // Poll for completion
    let poll_url = format!(
        "{}/a2a/tasks/{}",
        hub_url.trim_end_matches('/'),
        hub_task.id
    );
    let poll_interval = Duration::from_secs(3);
    let max_attempts = 100; // 5 minutes

    for attempt in 1..=max_attempts {
        tokio::time::sleep(poll_interval).await;

        let resp = http
            .get(&poll_url)
            .send()
            .await
            .map_err(|e| format!("hub poll error: {}", e))?;

        if !resp.status().is_success() {
            debug!(attempt, "hub poll non-success, retrying...");
            continue;
        }

        let status: HubTaskStatus = match resp.json().await {
            Ok(s) => s,
            Err(e) => {
                debug!(attempt, error = %e, "hub poll parse error, retrying...");
                continue;
            }
        };

        match status.state.as_str() {
            "completed" => {
                return status.response.ok_or_else(|| "hub returned completed with no response".to_string());
            }
            "failed" | "cancelled" => {
                return Err(format!("hub task {}", status.state));
            }
            _ => {
                // Still running
                debug!(attempt, state = %status.state, "hub task still running...");
            }
        }
    }

    Err("hub task timed out".to_string())
}

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

/// Parse a raw HTTP request into (method, path, headers, body).
fn parse_http_request(raw: &str) -> Option<(String, String, Vec<(String, String)>, String)> {
    let mut lines = raw.lines();

    // Request line: "GET /path HTTP/1.1"
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();

    // Headers
    let mut headers = Vec::new();
    let mut header_end = false;

    for line in lines {
        if line.is_empty() {
            header_end = true;
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            headers.push((key.trim().to_string(), value.trim().to_string()));
        }
    }

    // Body: everything after the blank line
    let body = if header_end {
        // Find the \r\n\r\n or \n\n separator
        if let Some(pos) = raw.find("\r\n\r\n") {
            raw[pos + 4..].to_string()
        } else if let Some(pos) = raw.find("\n\n") {
            raw[pos + 2..].to_string()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    // Strip path query string for routing (keep it simple)
    let path = path.split('?').next().unwrap_or(&path).to_string();

    Some((method, path, headers, body))
}

/// Write an HTTP response to a stream.
async fn write_response(
    stream: &mut tokio::net::TcpStream,
    status: u16,
    body: &str,
) -> std::io::Result<()> {
    let status_text = match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        _ => "Unknown",
    };

    let cors = "Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, DELETE, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization";

    let response = if status == 204 {
        format!("HTTP/1.1 {} {}\r\n{}\r\nConnection: close\r\n\r\n", status, status_text, cors)
    } else {
        format!(
            "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{}\r\nConnection: close\r\n\r\n{}",
            status,
            status_text,
            body.len(),
            cors,
            body
        )
    };

    stream.write_all(response.as_bytes()).await
}

/// Build the skills list for the agent card.
fn build_skills(hub_url: &Option<String>) -> Vec<AgentSkill> {
    let mut skills = vec![
        AgentSkill {
            id: "algochat-relay".into(),
            name: "AlgoChat Relay".into(),
            description: "Send and receive encrypted messages on Algorand".into(),
        },
        AgentSkill {
            id: "chain-discovery".into(),
            name: "Chain Discovery".into(),
            description: "Discover agents and scan AlgoChat activity on-chain".into(),
        },
    ];

    if hub_url.is_some() {
        skills.push(AgentSkill {
            id: "hub-forwarding".into(),
            name: "Hub Forwarding".into(),
            description: "Forward tasks to corvid-agent hub for AI processing".into(),
        });
    }

    skills.push(AgentSkill {
        id: "plugin-tools".into(),
        name: "Plugin Tools".into(),
        description: "Invoke loaded WASM plugin tools".into(),
    });

    skills
}

/// Generate a unique task ID.
fn generate_task_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let ts = now_secs();
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("task-{}-{}", ts, count)
}

/// Current Unix timestamp in seconds.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

/// Truncate a string for display.
fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..max]
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_get_request() {
        let raw = "GET /.well-known/agent.json HTTP/1.1\r\nHost: localhost:9100\r\n\r\n";
        let (method, path, headers, body) = parse_http_request(raw).unwrap();
        assert_eq!(method, "GET");
        assert_eq!(path, "/.well-known/agent.json");
        assert_eq!(headers.len(), 1);
        assert!(body.is_empty());
    }

    #[test]
    fn parse_post_request_with_body() {
        let raw = "POST /a2a/tasks HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: 42\r\n\r\n{\"message\":\"hello\",\"timeoutMs\":5000}";
        let (method, path, _headers, body) = parse_http_request(raw).unwrap();
        assert_eq!(method, "POST");
        assert_eq!(path, "/a2a/tasks");
        assert_eq!(body, r#"{"message":"hello","timeoutMs":5000}"#);
    }

    #[test]
    fn parse_request_strips_query_string() {
        let raw = "GET /a2a/tasks?limit=10 HTTP/1.1\r\n\r\n";
        let (_, path, _, _) = parse_http_request(raw).unwrap();
        assert_eq!(path, "/a2a/tasks");
    }

    #[test]
    fn task_store_insert_and_get() {
        let mut store = TaskStore::new(100);
        let task = Task {
            id: "task-1".into(),
            state: TaskState::Pending,
            message: "hello".into(),
            response: None,
            created_at: 1000,
            completed_at: None,
            from: None,
        };
        store.insert(task);
        assert!(store.get("task-1").is_some());
        assert_eq!(store.get("task-1").unwrap().message, "hello");
    }

    #[test]
    fn task_store_evicts_oldest() {
        let mut store = TaskStore::new(2);
        for i in 0..3 {
            store.insert(Task {
                id: format!("task-{}", i),
                state: TaskState::Pending,
                message: format!("msg-{}", i),
                response: None,
                created_at: i as u64,
                completed_at: None,
                from: None,
            });
        }
        assert!(store.get("task-0").is_none()); // evicted
        assert!(store.get("task-1").is_some());
        assert!(store.get("task-2").is_some());
    }

    #[test]
    fn task_store_list_returns_newest_first() {
        let mut store = TaskStore::new(100);
        for i in 0..5 {
            store.insert(Task {
                id: format!("task-{}", i),
                state: TaskState::Pending,
                message: format!("msg-{}", i),
                response: None,
                created_at: i as u64,
                completed_at: None,
                from: None,
            });
        }
        let list = store.list(3);
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].id, "task-4");
        assert_eq!(list[1].id, "task-3");
        assert_eq!(list[2].id, "task-2");
    }

    #[test]
    fn generate_task_id_unique() {
        let id1 = generate_task_id();
        let id2 = generate_task_id();
        assert_ne!(id1, id2);
        assert!(id1.starts_with("task-"));
    }

    #[test]
    fn create_task_request_deserializes() {
        let json = r#"{"message":"hello world"}"#;
        let req: CreateTaskRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.message, "hello world");
        assert_eq!(req.timeout_ms, 300_000); // default
        assert!(req.from.is_none());
    }

    #[test]
    fn create_task_request_with_all_fields() {
        let json = r#"{"message":"hi","timeoutMs":5000,"from":"agent-1"}"#;
        let req: CreateTaskRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.message, "hi");
        assert_eq!(req.timeout_ms, 5000);
        assert_eq!(req.from.as_deref(), Some("agent-1"));
    }

    #[test]
    fn agent_card_serializes() {
        let card = AgentCard {
            name: "test".into(),
            version: "0.1.0".into(),
            description: "test agent".into(),
            url: "http://localhost:9100".into(),
            capabilities: AgentCapabilities {
                streaming: false,
                push_notifications: false,
            },
            skills: vec![],
            authentication: AgentAuth {
                schemes: vec!["bearer".into()],
            },
        };
        let json = serde_json::to_string(&card).unwrap();
        assert!(json.contains("\"pushNotifications\":false"));
        assert!(json.contains("\"name\":\"test\""));
    }

    #[test]
    fn task_state_serializes() {
        assert_eq!(
            serde_json::to_string(&TaskState::Completed).unwrap(),
            "\"completed\""
        );
        assert_eq!(
            serde_json::to_string(&TaskState::Running).unwrap(),
            "\"running\""
        );
    }

    #[tokio::test]
    async fn route_agent_card() {
        let store = Arc::new(RwLock::new(TaskStore::new(10)));
        let http = Client::new();
        let card = r#"{"name":"test"}"#;

        let (status, body) = route("GET", "/.well-known/agent.json", "", card, &store, &http, None).await;
        assert_eq!(status, 200);
        assert_eq!(body, card);
    }

    #[tokio::test]
    async fn route_health() {
        let store = Arc::new(RwLock::new(TaskStore::new(10)));
        let http = Client::new();

        let (status, body) = route("GET", "/health", "", "{}", &store, &http, None).await;
        assert_eq!(status, 200);
        assert!(body.contains("healthy"));
    }

    #[tokio::test]
    async fn route_not_found() {
        let store = Arc::new(RwLock::new(TaskStore::new(10)));
        let http = Client::new();

        let (status, _) = route("GET", "/nonexistent", "", "{}", &store, &http, None).await;
        assert_eq!(status, 404);
    }

    #[tokio::test]
    async fn route_create_task_no_hub() {
        let store = Arc::new(RwLock::new(TaskStore::new(10)));
        let http = Client::new();
        let body = r#"{"message":"hello"}"#;

        let (status, response) = route("POST", "/a2a/tasks", body, "{}", &store, &http, None).await;
        assert_eq!(status, 201);
        assert!(response.contains("task-"));

        // Wait a moment for async processing
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Task should be completed (P2P echo mode)
        let s = store.read().await;
        let tasks = s.list(1);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].state, TaskState::Completed);
        assert!(tasks[0].response.as_ref().unwrap().contains("hello"));
    }

    #[tokio::test]
    async fn route_create_task_invalid_json() {
        let store = Arc::new(RwLock::new(TaskStore::new(10)));
        let http = Client::new();

        let (status, _) = route("POST", "/a2a/tasks", "not json", "{}", &store, &http, None).await;
        assert_eq!(status, 400);
    }

    #[tokio::test]
    async fn route_get_task_not_found() {
        let store = Arc::new(RwLock::new(TaskStore::new(10)));
        let http = Client::new();

        let (status, _) = route("GET", "/a2a/tasks/nonexistent", "", "{}", &store, &http, None).await;
        assert_eq!(status, 404);
    }

    #[tokio::test]
    async fn route_cancel_task() {
        let store = Arc::new(RwLock::new(TaskStore::new(10)));

        // Insert a pending task
        {
            let mut s = store.write().await;
            s.insert(Task {
                id: "cancel-me".into(),
                state: TaskState::Pending,
                message: "test".into(),
                response: None,
                created_at: 0,
                completed_at: None,
                from: None,
            });
        }

        let http = Client::new();
        let (status, body) = route("DELETE", "/a2a/tasks/cancel-me", "", "{}", &store, &http, None).await;
        assert_eq!(status, 200);
        assert!(body.contains("cancelled"));

        // Verify state
        let s = store.read().await;
        assert_eq!(s.get("cancel-me").unwrap().state, TaskState::Cancelled);
    }

    #[tokio::test]
    async fn route_cancel_completed_task_returns_conflict() {
        let store = Arc::new(RwLock::new(TaskStore::new(10)));

        {
            let mut s = store.write().await;
            s.insert(Task {
                id: "done".into(),
                state: TaskState::Completed,
                message: "test".into(),
                response: Some("result".into()),
                created_at: 0,
                completed_at: Some(1),
                from: None,
            });
        }

        let http = Client::new();
        let (status, _) = route("DELETE", "/a2a/tasks/done", "", "{}", &store, &http, None).await;
        assert_eq!(status, 409);
    }
}
