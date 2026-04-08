//! A2A (Agent-to-Agent) HTTP server.
//!
//! Exposes an HTTP endpoint so external agents can interact with this nano agent
//! directly over HTTP, without waiting for on-chain AlgoChat polling.
//!
//! Endpoints:
//! - `POST /a2a/tasks/send` — submit a task
//! - `GET /a2a/tasks/{id}` — poll task status
//! - `GET /.well-known/agent.json` — agent discovery card

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Task types
// ---------------------------------------------------------------------------

/// Task states matching the hub's A2A protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskState {
    Submitted,
    Working,
    Completed,
    Failed,
    Cancelled,
}

/// An A2A task tracked by this server.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct Task {
    id: String,
    state: TaskState,
    message: String,
    response: Option<String>,
    created_at: Instant,
}

/// Shared task store.
type TaskStore = Arc<Mutex<HashMap<String, Task>>>;

/// JSON payload for `POST /a2a/tasks/send`.
#[derive(Debug, Deserialize)]
struct TaskSendRequest {
    message: String,
    #[serde(default = "default_timeout_ms", rename = "timeoutMs")]
    timeout_ms: u64,
}

fn default_timeout_ms() -> u64 {
    300_000 // 5 minutes
}

/// JSON response from `POST /a2a/tasks/send`.
#[derive(Debug, Serialize)]
struct TaskSendResponse {
    id: String,
    state: TaskState,
}

/// JSON response from `GET /a2a/tasks/{id}`.
#[derive(Debug, Serialize)]
struct TaskStatusResponse {
    id: String,
    state: TaskState,
    #[serde(skip_serializing_if = "Option::is_none")]
    response: Option<String>,
}

// ---------------------------------------------------------------------------
// Hub forwarding (reuses the same protocol as agent.rs)
// ---------------------------------------------------------------------------

/// JSON payload sent to the hub.
#[derive(Debug, Serialize)]
struct HubTaskRequest {
    message: String,
    #[serde(rename = "timeoutMs")]
    timeout_ms: u64,
}

/// JSON response from the hub task creation.
#[derive(Debug, Deserialize)]
struct HubTaskResponse {
    id: String,
    #[allow(dead_code)]
    state: String,
}

/// Full task status from the hub.
#[derive(Debug, Deserialize)]
struct HubTaskStatus {
    state: String,
    #[serde(default)]
    response: Option<String>,
}

const HUB_POLL_INTERVAL: Duration = Duration::from_secs(3);
const HUB_POLL_MAX_ATTEMPTS: u32 = 100;

/// Forward a task to the hub's A2A endpoint and poll for completion.
/// Updates the task store as the task progresses.
async fn process_task(
    http: Client,
    hub_url: String,
    task_id: String,
    message: String,
    timeout_ms: u64,
    tasks: TaskStore,
) {
    // Mark as working
    {
        let mut store = tasks.lock().await;
        if let Some(task) = store.get_mut(&task_id) {
            task.state = TaskState::Working;
        }
    }

    // Forward to hub
    let url = format!("{}/a2a/tasks/send", hub_url.trim_end_matches('/'));
    let payload = HubTaskRequest {
        message,
        timeout_ms,
    };

    let hub_task_id = match http.post(&url).json(&payload).send().await {
        Ok(resp) if resp.status().is_success() => match resp.json::<HubTaskResponse>().await {
            Ok(task) => {
                info!(hub_task_id = %task.id, "forwarded A2A task to hub");
                task.id
            }
            Err(e) => {
                warn!(error = %e, "hub response parse failed");
                complete_task(&tasks, &task_id, TaskState::Failed, Some("[error] Hub returned invalid response".into())).await;
                return;
            }
        },
        Ok(resp) => {
            warn!(status = %resp.status(), "hub rejected task");
            complete_task(&tasks, &task_id, TaskState::Failed, Some("[error] Hub rejected the request".into())).await;
            return;
        }
        Err(e) => {
            warn!(error = %e, "hub unreachable");
            complete_task(&tasks, &task_id, TaskState::Failed, Some("[error] Agent hub is unreachable".into())).await;
            return;
        }
    };

    // Poll hub for response
    let poll_url = format!(
        "{}/a2a/tasks/{}",
        hub_url.trim_end_matches('/'),
        hub_task_id
    );

    for attempt in 1..=HUB_POLL_MAX_ATTEMPTS {
        tokio::time::sleep(HUB_POLL_INTERVAL).await;

        match http.get(&poll_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<HubTaskStatus>().await {
                    Ok(status) => {
                        debug!(hub_task_id = %hub_task_id, state = %status.state, attempt, "polled hub task");
                        match status.state.as_str() {
                            "completed" => {
                                complete_task(&tasks, &task_id, TaskState::Completed, status.response).await;
                                return;
                            }
                            "failed" | "cancelled" => {
                                complete_task(&tasks, &task_id, TaskState::Failed, Some(format!("[error] Hub task {}", status.state))).await;
                                return;
                            }
                            _ => {} // still running
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, attempt, "failed to parse hub task status");
                    }
                }
            }
            Ok(resp) => {
                debug!(status = %resp.status(), attempt, "hub poll non-success");
            }
            Err(e) => {
                warn!(error = %e, attempt, "failed to poll hub task");
            }
        }
    }

    warn!(task_id = %task_id, "hub task poll timed out");
    complete_task(&tasks, &task_id, TaskState::Failed, Some("[error] Request timed out".into())).await;
}

async fn complete_task(
    tasks: &TaskStore,
    task_id: &str,
    state: TaskState,
    response: Option<String>,
) {
    let mut store = tasks.lock().await;
    if let Some(task) = store.get_mut(task_id) {
        task.state = state;
        task.response = response;
    }
}

// ---------------------------------------------------------------------------
// HTTP request parsing
// ---------------------------------------------------------------------------

/// Parsed HTTP request (minimal parser for our needs).
struct HttpRequest {
    method: String,
    path: String,
    body: String,
}

fn parse_http_request(raw: &str) -> Option<HttpRequest> {
    let mut lines = raw.lines();
    let first_line = lines.next()?;
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    let method = parts[0].to_string();
    let path = parts[1].to_string();

    // Find body after blank line
    let body = if let Some(pos) = raw.find("\r\n\r\n") {
        raw[pos + 4..].to_string()
    } else if let Some(pos) = raw.find("\n\n") {
        raw[pos + 2..].to_string()
    } else {
        String::new()
    };

    Some(HttpRequest { method, path, body })
}

fn http_response(status: u16, status_text: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nConnection: close\r\n\r\n{}",
        status,
        status_text,
        body.len(),
        body
    )
}

// ---------------------------------------------------------------------------
// A2A server configuration
// ---------------------------------------------------------------------------

/// Configuration for the A2A server.
pub struct A2aConfig {
    /// Port to listen on.
    pub port: u16,
    /// Hub URL for task forwarding.
    pub hub_url: String,
    /// Agent display name.
    pub agent_name: String,
    /// Agent Algorand address.
    pub agent_address: String,
    /// Agent version.
    pub version: String,
}

// ---------------------------------------------------------------------------
// A2A server entry point
// ---------------------------------------------------------------------------

/// Run the A2A HTTP server.
///
/// Accepts task submissions over HTTP and forwards them to the hub for processing.
/// External agents can poll for results without waiting for on-chain messaging.
pub async fn serve_a2a(config: A2aConfig) -> Result<()> {
    let listener = TcpListener::bind(format!("0.0.0.0:{}", config.port)).await?;
    info!(port = config.port, "A2A server listening");

    let tasks: TaskStore = Arc::new(Mutex::new(HashMap::new()));
    let http = Client::new();
    let hub_url = Arc::new(config.hub_url);
    let agent_name = Arc::new(config.agent_name);
    let agent_address = Arc::new(config.agent_address);
    let version = Arc::new(config.version);

    // Background task to clean up old completed/failed tasks (>30 min)
    let cleanup_tasks = Arc::clone(&tasks);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(300));
        loop {
            interval.tick().await;
            let mut store = cleanup_tasks.lock().await;
            let cutoff = Instant::now() - Duration::from_secs(1800);
            store.retain(|_, task| {
                task.created_at > cutoff
                    || (task.state != TaskState::Completed && task.state != TaskState::Failed)
            });
        }
    });

    loop {
        let (mut stream, peer) = listener.accept().await?;

        let tasks = Arc::clone(&tasks);
        let http = http.clone();
        let hub_url = Arc::clone(&hub_url);
        let agent_name = Arc::clone(&agent_name);
        let agent_address = Arc::clone(&agent_address);
        let version = Arc::clone(&version);

        tokio::spawn(async move {
            let mut buf = vec![0u8; 8192];
            let n = match stream.read(&mut buf).await {
                Ok(n) if n > 0 => n,
                _ => return,
            };

            let raw = String::from_utf8_lossy(&buf[..n]);
            let req = match parse_http_request(&raw) {
                Some(r) => r,
                None => {
                    let resp = http_response(400, "Bad Request", &json!({"error": "invalid request"}).to_string());
                    let _ = stream.write_all(resp.as_bytes()).await;
                    return;
                }
            };

            let response = match (req.method.as_str(), req.path.as_str()) {
                // CORS preflight
                ("OPTIONS", _) => http_response(204, "No Content", ""),

                // Agent discovery card
                ("GET", "/.well-known/agent.json") => {
                    let card = json!({
                        "name": *agent_name,
                        "description": format!("Corvid Agent CAN ({})", *agent_name),
                        "url": format!("http://localhost:{}", stream.local_addr().map(|a| a.port()).unwrap_or(0)),
                        "version": *version,
                        "capabilities": {
                            "tasks": true,
                            "streaming": false,
                        },
                        "provider": {
                            "organization": "CorvidLabs",
                        },
                        "defaultInputModes": ["text"],
                        "defaultOutputModes": ["text"],
                        "skills": [{
                            "id": "chat",
                            "name": "Chat",
                            "description": "General-purpose conversation and task execution",
                        }],
                        "authentication": {
                            "schemes": [],
                        },
                        "algorand": {
                            "address": *agent_address,
                            "network": "localnet",
                        },
                    });
                    http_response(200, "OK", &card.to_string())
                }

                // Submit a new task
                ("POST", "/a2a/tasks/send") => {
                    match serde_json::from_str::<TaskSendRequest>(&req.body) {
                        Ok(task_req) => {
                            let task_id = generate_task_id();
                            let task = Task {
                                id: task_id.clone(),
                                state: TaskState::Submitted,
                                message: task_req.message.clone(),
                                response: None,
                                created_at: Instant::now(),
                            };

                            {
                                let mut store = tasks.lock().await;
                                store.insert(task_id.clone(), task);
                            }

                            info!(task_id = %task_id, "A2A task submitted");

                            // Spawn background processing
                            let process_http = http.clone();
                            let process_hub = (*hub_url).clone();
                            let process_tasks = Arc::clone(&tasks);
                            let process_msg = task_req.message;
                            let process_timeout = task_req.timeout_ms;
                            let process_id = task_id.clone();
                            tokio::spawn(async move {
                                process_task(
                                    process_http,
                                    process_hub,
                                    process_id,
                                    process_msg,
                                    process_timeout,
                                    process_tasks,
                                )
                                .await;
                            });

                            let resp = TaskSendResponse {
                                id: task_id,
                                state: TaskState::Submitted,
                            };
                            http_response(200, "OK", &serde_json::to_string(&resp).unwrap())
                        }
                        Err(e) => {
                            let body = json!({"error": format!("Invalid request body: {}", e)});
                            http_response(400, "Bad Request", &body.to_string())
                        }
                    }
                }

                // Poll task status
                ("GET", path) if path.starts_with("/a2a/tasks/") => {
                    let task_id = &path["/a2a/tasks/".len()..];
                    if task_id.is_empty() {
                        let body = json!({"error": "Missing task ID"});
                        http_response(400, "Bad Request", &body.to_string())
                    } else {
                        let store = tasks.lock().await;
                        match store.get(task_id) {
                            Some(task) => {
                                let resp = TaskStatusResponse {
                                    id: task.id.clone(),
                                    state: task.state.clone(),
                                    response: task.response.clone(),
                                };
                                http_response(200, "OK", &serde_json::to_string(&resp).unwrap())
                            }
                            None => {
                                let body = json!({"error": "Task not found"});
                                http_response(404, "Not Found", &body.to_string())
                            }
                        }
                    }
                }

                _ => {
                    let body = json!({"error": "Not found"});
                    http_response(404, "Not Found", &body.to_string())
                }
            };

            if let Err(e) = stream.write_all(response.as_bytes()).await {
                warn!(peer = %peer, error = %e, "A2A: write error");
            }
        });
    }
}

/// Generate a short random task ID.
fn generate_task_id() -> String {
    use std::time::SystemTime;
    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let rand_part: u32 = rand::random();
    format!("a2a-{:x}-{:08x}", ts, rand_part)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_get_request() {
        let raw = "GET /a2a/tasks/abc123 HTTP/1.1\r\nHost: localhost\r\n\r\n";
        let req = parse_http_request(raw).unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/a2a/tasks/abc123");
        assert!(req.body.is_empty());
    }

    #[test]
    fn parse_post_request_with_body() {
        let raw = "POST /a2a/tasks/send HTTP/1.1\r\nHost: localhost\r\nContent-Length: 27\r\n\r\n{\"message\":\"hello\",\"timeoutMs\":5000}";
        let req = parse_http_request(raw).unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/a2a/tasks/send");
        assert!(req.body.contains("hello"));
    }

    #[test]
    fn parse_invalid_request() {
        assert!(parse_http_request("").is_none());
        assert!(parse_http_request("GARBAGE").is_none());
    }

    #[test]
    fn task_send_request_deserializes() {
        let json = r#"{"message":"hello world","timeoutMs":60000}"#;
        let req: TaskSendRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.message, "hello world");
        assert_eq!(req.timeout_ms, 60000);
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
            id: "a2a-123".to_string(),
            state: TaskState::Submitted,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["id"], "a2a-123");
        assert_eq!(json["state"], "submitted");
    }

    #[test]
    fn task_status_response_omits_none_response() {
        let resp = TaskStatusResponse {
            id: "a2a-123".to_string(),
            state: TaskState::Working,
            response: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(!json.contains("response"));
    }

    #[test]
    fn task_status_response_includes_response() {
        let resp = TaskStatusResponse {
            id: "a2a-123".to_string(),
            state: TaskState::Completed,
            response: Some("Hello!".to_string()),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["state"], "completed");
        assert_eq!(json["response"], "Hello!");
    }

    #[test]
    fn task_state_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&TaskState::Submitted).unwrap(),
            "\"submitted\""
        );
        assert_eq!(
            serde_json::to_string(&TaskState::Working).unwrap(),
            "\"working\""
        );
        assert_eq!(
            serde_json::to_string(&TaskState::Completed).unwrap(),
            "\"completed\""
        );
        assert_eq!(
            serde_json::to_string(&TaskState::Failed).unwrap(),
            "\"failed\""
        );
    }

    #[test]
    fn generate_task_id_unique() {
        let id1 = generate_task_id();
        let id2 = generate_task_id();
        assert_ne!(id1, id2);
        assert!(id1.starts_with("a2a-"));
    }

    #[test]
    fn http_response_format() {
        let resp = http_response(200, "OK", "{\"status\":\"ok\"}");
        assert!(resp.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(resp.contains("Content-Type: application/json"));
        assert!(resp.contains("{\"status\":\"ok\"}"));
    }

    #[tokio::test]
    async fn task_store_lifecycle() {
        let tasks: TaskStore = Arc::new(Mutex::new(HashMap::new()));

        // Create task
        let task = Task {
            id: "test-1".to_string(),
            state: TaskState::Submitted,
            message: "hello".to_string(),
            response: None,
            created_at: Instant::now(),
        };
        tasks.lock().await.insert("test-1".to_string(), task);

        // Verify submitted
        {
            let store = tasks.lock().await;
            assert_eq!(store.get("test-1").unwrap().state, TaskState::Submitted);
        }

        // Complete it
        complete_task(&tasks, "test-1", TaskState::Completed, Some("done".into())).await;

        // Verify completed
        {
            let store = tasks.lock().await;
            let task = store.get("test-1").unwrap();
            assert_eq!(task.state, TaskState::Completed);
            assert_eq!(task.response.as_deref(), Some("done"));
        }
    }

    #[tokio::test]
    async fn complete_nonexistent_task_is_noop() {
        let tasks: TaskStore = Arc::new(Mutex::new(HashMap::new()));
        // Should not panic
        complete_task(&tasks, "nonexistent", TaskState::Failed, None).await;
    }

    #[test]
    fn cors_response_format() {
        let resp = http_response(204, "No Content", "");
        assert!(resp.contains("Access-Control-Allow-Origin: *"));
        assert!(resp.contains("Access-Control-Allow-Methods: GET, POST, OPTIONS"));
    }
}
