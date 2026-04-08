//! End-to-end tests for the A2A HTTP server.
//!
//! Spins up a real TCP listener and makes HTTP requests against it.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// Find a free port by binding to port 0 and reading the assigned port.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// Start the `can run` binary with --a2a-port in a subprocess.
/// Returns the child process and the A2A port.
fn start_agent_with_a2a(port: u16) -> std::process::Child {
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_can"))
        .args([
            "--data-dir",
            "/tmp/can-a2a-test",
            "run",
            "--network",
            "localnet",
            "--no-hub",
            "--no-plugins",
            "--a2a-port",
            &port.to_string(),
            // Use a dummy seed so we don't need a keystore
            "--seed",
            "0000000000000000000000000000000000000000000000000000000000000001",
            "--address",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAY5HFKQ",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("failed to start can with --a2a-port");
    child
}

/// Make an HTTP request and return the status code and body.
fn http_request(port: u16, method: &str, path: &str, body: Option<&str>) -> (u16, String) {
    let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();

    let request = if let Some(body) = body {
        format!(
            "{} {} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            method, path, body.len(), body
        )
    } else {
        format!(
            "{} {} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            method, path
        )
    };

    stream.write_all(request.as_bytes()).unwrap();

    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);

    // Parse status code
    let status = response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);

    // Extract body (after \r\n\r\n)
    let body = response
        .find("\r\n\r\n")
        .map(|pos| response[pos + 4..].to_string())
        .unwrap_or_default();

    (status, body)
}

/// Wait for a port to accept connections.
fn wait_for_port(port: u16, timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if TcpStream::connect(format!("127.0.0.1:{}", port)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

#[test]
fn a2a_agent_card_endpoint() {
    let port = free_port();
    let mut child = start_agent_with_a2a(port);

    if !wait_for_port(port, Duration::from_secs(10)) {
        child.kill().ok();
        panic!("A2A server did not start within 10 seconds");
    }

    let (status, body) = http_request(port, "GET", "/.well-known/agent.json", None);

    child.kill().ok();
    let _ = child.wait();

    assert_eq!(status, 200, "agent card should return 200");
    let card: serde_json::Value = serde_json::from_str(&body).expect("agent card should be JSON");
    assert!(card.get("name").is_some(), "card should have name");
    assert!(
        card.get("capabilities").is_some(),
        "card should have capabilities"
    );
    assert!(
        card.get("algorand").is_some(),
        "card should have algorand field"
    );
}

#[test]
fn a2a_submit_task_and_poll() {
    let port = free_port();
    let mut child = start_agent_with_a2a(port);

    if !wait_for_port(port, Duration::from_secs(10)) {
        child.kill().ok();
        panic!("A2A server did not start within 10 seconds");
    }

    // Submit a task
    let (status, body) = http_request(
        port,
        "POST",
        "/a2a/tasks/send",
        Some(r#"{"message":"hello from e2e test","timeoutMs":5000}"#),
    );

    assert_eq!(status, 200, "task submission should return 200");
    let resp: serde_json::Value = serde_json::from_str(&body).expect("response should be JSON");
    let task_id = resp["id"].as_str().expect("response should have id");
    assert!(task_id.starts_with("a2a-"), "task ID should start with a2a-");
    assert_eq!(resp["state"], "submitted");

    // Poll for the task (it will fail because hub is disabled, but the endpoint should work)
    // Wait a moment for the background task to try the hub and fail
    std::thread::sleep(Duration::from_secs(2));

    let (poll_status, poll_body) =
        http_request(port, "GET", &format!("/a2a/tasks/{}", task_id), None);

    child.kill().ok();
    let _ = child.wait();

    assert_eq!(poll_status, 200, "task poll should return 200");
    let poll_resp: serde_json::Value =
        serde_json::from_str(&poll_body).expect("poll response should be JSON");
    assert_eq!(poll_resp["id"], task_id);
    // State should be either working or failed (hub is disabled)
    let state = poll_resp["state"].as_str().unwrap();
    assert!(
        state == "working" || state == "failed",
        "task should be working or failed, got: {}",
        state
    );
}

#[test]
fn a2a_poll_nonexistent_task() {
    let port = free_port();
    let mut child = start_agent_with_a2a(port);

    if !wait_for_port(port, Duration::from_secs(10)) {
        child.kill().ok();
        panic!("A2A server did not start within 10 seconds");
    }

    let (status, body) = http_request(port, "GET", "/a2a/tasks/nonexistent-id", None);

    child.kill().ok();
    let _ = child.wait();

    assert_eq!(status, 404, "polling nonexistent task should return 404");
    let resp: serde_json::Value = serde_json::from_str(&body).expect("response should be JSON");
    assert!(resp.get("error").is_some());
}

#[test]
fn a2a_invalid_request_body() {
    let port = free_port();
    let mut child = start_agent_with_a2a(port);

    if !wait_for_port(port, Duration::from_secs(10)) {
        child.kill().ok();
        panic!("A2A server did not start within 10 seconds");
    }

    let (status, _) = http_request(
        port,
        "POST",
        "/a2a/tasks/send",
        Some("not valid json"),
    );

    child.kill().ok();
    let _ = child.wait();

    assert_eq!(status, 400, "invalid body should return 400");
}

#[test]
fn a2a_unknown_path_returns_404() {
    let port = free_port();
    let mut child = start_agent_with_a2a(port);

    if !wait_for_port(port, Duration::from_secs(10)) {
        child.kill().ok();
        panic!("A2A server did not start within 10 seconds");
    }

    let (status, _) = http_request(port, "GET", "/unknown/path", None);

    child.kill().ok();
    let _ = child.wait();

    assert_eq!(status, 404, "unknown path should return 404");
}
