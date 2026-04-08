//! A2A (Agent-to-Agent) HTTP server for direct agent communication.
//!
//! Exposes the same interface as the corvid-agent hub so other agents can
//! talk to this nano agent directly over HTTP without on-chain messaging.
//!
//! Endpoints:
//!   POST /a2a/tasks/send   — submit a message, returns task ID
//!   GET  /a2a/tasks/{id}   — poll for task status and response
//!   GET  /.well-known/agent.json — agent card (discovery metadata)
//!   GET  /health            — health check (shared with existing endpoint)

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Task types
// ---------------------------------------------------------------------------

/// Task states matching the hub protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskState {
    Submitted,
    Working,
    Completed,
    Failed,
    Cancelled,
}

impl std::fmt::Display for TaskState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskState::Submitted => write!(f, "submitted"),
            TaskState::Working => write!(f, "working"),
            TaskState::Completed => write!(f, "completed"),
            TaskState::Failed => write!(f, "failed"),
            TaskState::Cancelled => write!(f, "cancelled"),
        }
    }
}

/// An A2A task tracked by the server.
#[derive(Debug, Clone, Serialize)]
pub struct Task {
    pub id: String,
    pub state: TaskState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
    #[serde(skip)]
    pub created_at: Instant,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Inbound request payload for `POST /a2a/tasks/send`.
#[derive(Debug, Deserialize)]
pub struct TaskSendRequest {
    pub message: String,
    #[serde(rename = "timeoutMs", default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_timeout_ms() -> u64 {
    300_000
}

/// Response for `POST /a2a/tasks/send`.
#[derive(Debug, Serialize, Deserialize)]
pub struct TaskSendResponse {
    pub id: String,
    pub state: TaskState,
}

/// Response for `GET /a2a/tasks/{id}`.
#[derive(Debug, Serialize, Deserialize)]
pub struct TaskStatusResponse {
    pub state: TaskState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Task store
// ---------------------------------------------------------------------------

/// Thread-safe in-memory task store.
#[derive(Clone)]
pub struct TaskStore {
    tasks: Arc<RwLock<HashMap<String, Task>>>,
}

impl TaskStore {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new task and return its ID.
    pub async fn create(&self) -> String {
        let id = generate_task_id();
        let task = Task {
            id: id.clone(),
            state: TaskState::Submitted,
            response: None,
            created_at: Instant::now(),
            error: None,
        };
        self.tasks.write().await.insert(id.clone(), task);
        id
    }

    /// Get a task by ID.
    pub async fn get(&self, id: &str) -> Option<Task> {
        self.tasks.read().await.get(id).cloned()
    }

    /// Update task state to Working.
    pub async fn mark_working(&self, id: &str) {
        if let Some(task) = self.tasks.write().await.get_mut(id) {
            task.state = TaskState::Working;
        }
    }

    /// Complete a task with a response.
    pub async fn complete(&self, id: &str, response: String) {
        if let Some(task) = self.tasks.write().await.get_mut(id) {
            task.state = TaskState::Completed;
            task.response = Some(response);
        }
    }

    /// Fail a task with an error message.
    pub async fn fail(&self, id: &str, error: String) {
        if let Some(task) = self.tasks.write().await.get_mut(id) {
            task.state = TaskState::Failed;
            task.error = Some(error);
        }
    }

    /// Remove tasks older than `max_age`.
    pub async fn gc(&self, max_age: Duration) {
        let now = Instant::now();
        self.tasks
            .write()
            .await
            .retain(|_, task| now.duration_since(task.created_at) < max_age);
    }

    /// Number of tasks in the store.
    pub async fn len(&self) -> usize {
        self.tasks.read().await.len()
    }
}

/// A message submitted via A2A, ready for processing.
pub struct InboundTask {
    pub task_id: String,
    pub message: String,
    pub timeout: Duration,
}

// ---------------------------------------------------------------------------
// Agent card
// ---------------------------------------------------------------------------

/// Agent card metadata for `/.well-known/agent.json`.
#[derive(Debug, Clone, Serialize)]
pub struct AgentCard {
    pub name: String,
    pub description: String,
    pub url: String,
    pub version: String,
    pub capabilities: AgentCapabilities,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentCapabilities {
    pub streaming: bool,
    pub push_notifications: bool,
}

// ---------------------------------------------------------------------------
// HTTP server
// ---------------------------------------------------------------------------

/// Configuration for the A2A server.
pub struct A2aServerConfig {
    pub port: u16,
    pub agent_name: String,
    pub agent_address: String,
    pub version: String,
}

/// Start the A2A HTTP server.
///
/// Returns a sender that can be used to submit tasks for processing.
/// The caller is responsible for consuming `task_rx` and calling
/// `store.complete()` or `store.fail()` when done.
pub async fn serve(
    config: A2aServerConfig,
    store: TaskStore,
    task_tx: mpsc::Sender<InboundTask>,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(format!("0.0.0.0:{}", config.port)).await?;
    info!(port = config.port, "A2A server listening");

    let agent_card = serde_json::to_string(&AgentCard {
        name: config.agent_name.clone(),
        description: format!(
            "Corvid Agent CAN ({}) — lightweight Rust AlgoChat agent",
            config.agent_address
        ),
        url: format!("http://localhost:{}", config.port),
        version: config.version,
        capabilities: AgentCapabilities {
            streaming: false,
            push_notifications: false,
        },
    })?;

    // Spawn GC task — clean up old tasks every 5 minutes
    let gc_store = store.clone();
    tokio::spawn(async move {
        let gc_interval = Duration::from_secs(300);
        let max_age = Duration::from_secs(3600); // 1 hour
        loop {
            tokio::time::sleep(gc_interval).await;
            let before = gc_store.len().await;
            gc_store.gc(max_age).await;
            let after = gc_store.len().await;
            if before > after {
                debug!(removed = before - after, remaining = after, "A2A task GC");
            }
        }
    });

    loop {
        let (mut stream, peer) = listener.accept().await?;

        let store = store.clone();
        let task_tx = task_tx.clone();
        let agent_card = agent_card.clone();

        tokio::spawn(async move {
            // Read the full HTTP request (up to 64KB)
            let mut buf = vec![0u8; 65536];
            let n = match stream.read(&mut buf).await {
                Ok(0) => return,
                Ok(n) => n,
                Err(e) => {
                    warn!(peer = %peer, error = %e, "A2A: read error");
                    return;
                }
            };
            buf.truncate(n);

            let request = String::from_utf8_lossy(&buf);
            let first_line = request.lines().next().unwrap_or("");

            let (status, content_type, body) =
                handle_request(first_line, &request, &store, &task_tx, &agent_card).await;

            let status_text = match status {
                200 => "OK",
                201 => "Created",
                400 => "Bad Request",
                404 => "Not Found",
                405 => "Method Not Allowed",
                500 => "Internal Server Error",
                503 => "Service Unavailable",
                _ => "Unknown",
            };

            let response = format!(
                "HTTP/1.1 {} {}\r\n\
                 Content-Type: {}\r\n\
                 Content-Length: {}\r\n\
                 Access-Control-Allow-Origin: *\r\n\
                 Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
                 Access-Control-Allow-Headers: Content-Type\r\n\
                 Connection: close\r\n\r\n{}",
                status, status_text, content_type, body.len(), body
            );

            if let Err(e) = stream.write_all(response.as_bytes()).await {
                warn!(peer = %peer, error = %e, "A2A: write error");
            }
        });
    }
}

/// Route an HTTP request to the appropriate handler.
async fn handle_request(
    first_line: &str,
    full_request: &str,
    store: &TaskStore,
    task_tx: &mpsc::Sender<InboundTask>,
    agent_card: &str,
) -> (u16, &'static str, String) {
    let json_ct = "application/json";

    // OPTIONS (CORS preflight)
    if first_line.starts_with("OPTIONS ") {
        return (200, json_ct, String::new());
    }

    // POST /a2a/tasks/send
    if first_line.starts_with("POST /a2a/tasks/send") {
        return handle_task_send(full_request, store, task_tx).await;
    }

    // GET /a2a/tasks/{id}
    if first_line.starts_with("GET /a2a/tasks/") {
        let path = first_line
            .split_whitespace()
            .nth(1)
            .unwrap_or("");
        let task_id = path.strip_prefix("/a2a/tasks/").unwrap_or("");
        if task_id.is_empty() {
            return (400, json_ct, r#"{"error":"missing task ID"}"#.to_string());
        }
        return handle_task_get(task_id, store).await;
    }

    // GET /.well-known/agent.json
    if first_line.starts_with("GET /.well-known/agent.json") {
        return (200, json_ct, agent_card.to_string());
    }

    // GET /health
    if first_line.starts_with("GET /health") || first_line == "GET / " || first_line == "GET /" {
        let body = serde_json::json!({"status": "healthy"}).to_string();
        return (200, json_ct, body);
    }

    (404, json_ct, r#"{"error":"not found"}"#.to_string())
}

/// Handle `POST /a2a/tasks/send`.
async fn handle_task_send(
    full_request: &str,
    store: &TaskStore,
    task_tx: &mpsc::Sender<InboundTask>,
) -> (u16, &'static str, String) {
    let json_ct = "application/json";

    // Extract body (after the \r\n\r\n separator)
    let body = match full_request.split("\r\n\r\n").nth(1) {
        Some(b) if !b.is_empty() => b,
        _ => {
            return (
                400,
                json_ct,
                r#"{"error":"missing request body"}"#.to_string(),
            )
        }
    };

    let req: TaskSendRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            return (
                400,
                json_ct,
                serde_json::json!({"error": format!("invalid JSON: {}", e)}).to_string(),
            )
        }
    };

    if req.message.is_empty() {
        return (
            400,
            json_ct,
            r#"{"error":"message must not be empty"}"#.to_string(),
        );
    }

    let task_id = store.create().await;
    let timeout = Duration::from_millis(req.timeout_ms.min(600_000)); // Cap at 10 min

    let inbound = InboundTask {
        task_id: task_id.clone(),
        message: req.message,
        timeout,
    };

    // Try to send to the handler channel
    if task_tx.try_send(inbound).is_err() {
        store
            .fail(&task_id, "handler queue full".to_string())
            .await;
        return (
            503,
            json_ct,
            r#"{"error":"agent is busy, try again later"}"#.to_string(),
        );
    }

    info!(task_id = %task_id, "A2A task created");

    let resp = TaskSendResponse {
        id: task_id,
        state: TaskState::Submitted,
    };
    (201, json_ct, serde_json::to_string(&resp).unwrap())
}

/// Handle `GET /a2a/tasks/{id}`.
async fn handle_task_get(task_id: &str, store: &TaskStore) -> (u16, &'static str, String) {
    let json_ct = "application/json";

    match store.get(task_id).await {
        Some(task) => {
            let resp = TaskStatusResponse {
                state: task.state,
                response: task.response,
                error: task.error,
            };
            (200, json_ct, serde_json::to_string(&resp).unwrap())
        }
        None => (
            404,
            json_ct,
            r#"{"error":"task not found"}"#.to_string(),
        ),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Generate a random hex task ID (16 bytes = 32 hex chars).
fn generate_task_id() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: [u8; 16] = rng.gen();
    hex::encode(bytes)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn task_store_create_and_get() {
        let store = TaskStore::new();
        let id = store.create().await;
        assert!(!id.is_empty());
        assert_eq!(id.len(), 32); // 16 bytes hex

        let task = store.get(&id).await.unwrap();
        assert_eq!(task.state, TaskState::Submitted);
        assert!(task.response.is_none());
    }

    #[tokio::test]
    async fn task_store_complete() {
        let store = TaskStore::new();
        let id = store.create().await;

        store.mark_working(&id).await;
        let task = store.get(&id).await.unwrap();
        assert_eq!(task.state, TaskState::Working);

        store.complete(&id, "Hello!".to_string()).await;
        let task = store.get(&id).await.unwrap();
        assert_eq!(task.state, TaskState::Completed);
        assert_eq!(task.response.as_deref(), Some("Hello!"));
    }

    #[tokio::test]
    async fn task_store_fail() {
        let store = TaskStore::new();
        let id = store.create().await;

        store.fail(&id, "timeout".to_string()).await;
        let task = store.get(&id).await.unwrap();
        assert_eq!(task.state, TaskState::Failed);
        assert_eq!(task.error.as_deref(), Some("timeout"));
    }

    #[tokio::test]
    async fn task_store_gc_removes_old_tasks() {
        let store = TaskStore::new();
        let _id = store.create().await;
        assert_eq!(store.len().await, 1);

        // GC with 1-hour max age should keep it
        store.gc(Duration::from_secs(3600)).await;
        assert_eq!(store.len().await, 1);

        // GC with 0 max age should remove it
        store.gc(Duration::from_secs(0)).await;
        assert_eq!(store.len().await, 0);
    }

    #[tokio::test]
    async fn task_store_get_missing_returns_none() {
        let store = TaskStore::new();
        assert!(store.get("nonexistent").await.is_none());
    }

    #[test]
    fn task_send_request_deserializes() {
        let json = r#"{"message":"hello","timeoutMs":5000}"#;
        let req: TaskSendRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.message, "hello");
        assert_eq!(req.timeout_ms, 5000);
    }

    #[test]
    fn task_send_request_default_timeout() {
        let json = r#"{"message":"hello"}"#;
        let req: TaskSendRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.timeout_ms, 300_000);
    }

    #[test]
    fn task_send_response_serializes() {
        let resp = TaskSendResponse {
            id: "abc123".to_string(),
            state: TaskState::Submitted,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["id"], "abc123");
        assert_eq!(json["state"], "submitted");
    }

    #[test]
    fn task_status_response_omits_none_fields() {
        let resp = TaskStatusResponse {
            state: TaskState::Working,
            response: None,
            error: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(!json.contains("response"));
        assert!(!json.contains("error"));
    }

    #[test]
    fn task_state_display() {
        assert_eq!(TaskState::Submitted.to_string(), "submitted");
        assert_eq!(TaskState::Working.to_string(), "working");
        assert_eq!(TaskState::Completed.to_string(), "completed");
        assert_eq!(TaskState::Failed.to_string(), "failed");
        assert_eq!(TaskState::Cancelled.to_string(), "cancelled");
    }

    #[test]
    fn generate_task_id_is_unique() {
        let id1 = generate_task_id();
        let id2 = generate_task_id();
        assert_ne!(id1, id2);
        assert_eq!(id1.len(), 32);
    }

    #[test]
    fn agent_card_serializes() {
        let card = AgentCard {
            name: "test".to_string(),
            description: "A test agent".to_string(),
            url: "http://localhost:9999".to_string(),
            version: "0.3.0".to_string(),
            capabilities: AgentCapabilities {
                streaming: false,
                push_notifications: false,
            },
        };
        let json = serde_json::to_value(&card).unwrap();
        assert_eq!(json["name"], "test");
        assert_eq!(json["capabilities"]["streaming"], false);
    }

    #[tokio::test]
    async fn handle_request_post_task_send() {
        let store = TaskStore::new();
        let (tx, mut rx) = mpsc::channel(16);

        let request = "POST /a2a/tasks/send HTTP/1.1\r\nContent-Type: application/json\r\n\r\n{\"message\":\"hello\"}";
        let (status, _, body) =
            handle_request("POST /a2a/tasks/send HTTP/1.1", request, &store, &tx, "{}").await;

        assert_eq!(status, 201);
        let resp: TaskSendResponse = serde_json::from_str(&body).unwrap();
        assert_eq!(resp.state, TaskState::Submitted);
        assert_eq!(resp.id.len(), 32);

        // Verify inbound task was sent
        let inbound = rx.try_recv().unwrap();
        assert_eq!(inbound.message, "hello");
        assert_eq!(inbound.task_id, resp.id);
    }

    #[tokio::test]
    async fn handle_request_post_empty_message_rejected() {
        let store = TaskStore::new();
        let (tx, _rx) = mpsc::channel(16);

        let request = "POST /a2a/tasks/send HTTP/1.1\r\n\r\n{\"message\":\"\"}";
        let (status, _, body) =
            handle_request("POST /a2a/tasks/send HTTP/1.1", request, &store, &tx, "{}").await;

        assert_eq!(status, 400);
        assert!(body.contains("must not be empty"));
    }

    #[tokio::test]
    async fn handle_request_post_invalid_json() {
        let store = TaskStore::new();
        let (tx, _rx) = mpsc::channel(16);

        let request = "POST /a2a/tasks/send HTTP/1.1\r\n\r\n{bad json}";
        let (status, _, body) =
            handle_request("POST /a2a/tasks/send HTTP/1.1", request, &store, &tx, "{}").await;

        assert_eq!(status, 400);
        assert!(body.contains("invalid JSON"));
    }

    #[tokio::test]
    async fn handle_request_post_missing_body() {
        let store = TaskStore::new();
        let (tx, _rx) = mpsc::channel(16);

        let request = "POST /a2a/tasks/send HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let (status, _, _) =
            handle_request("POST /a2a/tasks/send HTTP/1.1", request, &store, &tx, "{}").await;

        assert_eq!(status, 400);
    }

    #[tokio::test]
    async fn handle_request_get_task_status() {
        let store = TaskStore::new();
        let (tx, _rx) = mpsc::channel(16);
        let id = store.create().await;
        store.complete(&id, "world".to_string()).await;

        let first_line = format!("GET /a2a/tasks/{} HTTP/1.1", id);
        let (status, _, body) =
            handle_request(&first_line, &first_line, &store, &tx, "{}").await;

        assert_eq!(status, 200);
        let resp: TaskStatusResponse = serde_json::from_str(&body).unwrap();
        assert_eq!(resp.state, TaskState::Completed);
        assert_eq!(resp.response.as_deref(), Some("world"));
    }

    #[tokio::test]
    async fn handle_request_get_task_not_found() {
        let store = TaskStore::new();
        let (tx, _rx) = mpsc::channel(16);

        let (status, _, body) = handle_request(
            "GET /a2a/tasks/nonexistent HTTP/1.1",
            "GET /a2a/tasks/nonexistent HTTP/1.1",
            &store,
            &tx,
            "{}",
        )
        .await;

        assert_eq!(status, 404);
        assert!(body.contains("not found"));
    }

    #[tokio::test]
    async fn handle_request_agent_card() {
        let store = TaskStore::new();
        let (tx, _rx) = mpsc::channel(16);
        let card = r#"{"name":"test"}"#;

        let (status, _, body) = handle_request(
            "GET /.well-known/agent.json HTTP/1.1",
            "GET /.well-known/agent.json HTTP/1.1",
            &store,
            &tx,
            card,
        )
        .await;

        assert_eq!(status, 200);
        assert_eq!(body, card);
    }

    #[tokio::test]
    async fn handle_request_unknown_route() {
        let store = TaskStore::new();
        let (tx, _rx) = mpsc::channel(16);

        let (status, _, _) = handle_request(
            "GET /unknown HTTP/1.1",
            "GET /unknown HTTP/1.1",
            &store,
            &tx,
            "{}",
        )
        .await;

        assert_eq!(status, 404);
    }

    #[tokio::test]
    async fn handle_request_options_cors() {
        let store = TaskStore::new();
        let (tx, _rx) = mpsc::channel(16);

        let (status, _, _) = handle_request(
            "OPTIONS /a2a/tasks/send HTTP/1.1",
            "OPTIONS /a2a/tasks/send HTTP/1.1",
            &store,
            &tx,
            "{}",
        )
        .await;

        assert_eq!(status, 200);
    }

    #[tokio::test]
    async fn handle_request_queue_full_returns_503() {
        let store = TaskStore::new();
        // Channel with capacity 1
        let (tx, _rx) = mpsc::channel(1);

        // Fill the channel
        let request1 = "POST /a2a/tasks/send HTTP/1.1\r\n\r\n{\"message\":\"first\"}";
        let (status1, _, _) =
            handle_request("POST /a2a/tasks/send HTTP/1.1", request1, &store, &tx, "{}").await;
        assert_eq!(status1, 201);

        // Second request should fail (channel full, not drained)
        let request2 = "POST /a2a/tasks/send HTTP/1.1\r\n\r\n{\"message\":\"second\"}";
        let (status2, _, body) =
            handle_request("POST /a2a/tasks/send HTTP/1.1", request2, &store, &tx, "{}").await;
        assert_eq!(status2, 503);
        assert!(body.contains("busy"));
    }
}
