---
module: a2a-server
version: 1
status: draft
files:
  - src/a2a.rs
depends_on:
  - core
---

# A2A Server

## Purpose

HTTP server enabling direct Agent-to-Agent communication without on-chain
AlgoChat messaging. Exposes the same interface as the corvid-agent hub so
other agents can interact with this nano agent over HTTP.

When enabled via `--a2a-port`, the server listens for inbound tasks, tracks
their lifecycle, and delegates processing to the hub (or responds directly in
P2P mode).

## Public API

### Exported Structs

| Struct | Description |
|--------|-------------|
| `TaskStore` | Thread-safe in-memory store for A2A task state |
| `Task` | A tracked A2A task with state, response, and metadata |
| `TaskState` | Enum: Submitted, Working, Completed, Failed, Cancelled |
| `TaskSendRequest` | Inbound JSON payload for `POST /a2a/tasks/send` |
| `TaskSendResponse` | Response for task submission (id + state) |
| `TaskStatusResponse` | Response for task polling (state + response/error) |
| `InboundTask` | Message passed from server to handler via channel |
| `AgentCard` | Discovery metadata at `/.well-known/agent.json` |
| `A2aServerConfig` | Server configuration (port, agent name, address, version) |

### Exported Functions

| Function | Parameters | Returns | Description |
|----------|-----------|---------|-------------|
| `serve` | `(config, store, task_tx)` | `Result<()>` | Start the HTTP server (runs forever) |

### HTTP Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/a2a/tasks/send` | Submit a message, returns task ID |
| `GET` | `/a2a/tasks/{id}` | Poll task status and response |
| `GET` | `/.well-known/agent.json` | Agent card (discovery metadata) |
| `GET` | `/health` | Basic health check |
| `OPTIONS` | `*` | CORS preflight |

## Invariants

1. Task IDs are 32-character hex strings (16 random bytes)
2. Tasks transition only forward: Submitted → Working → Completed/Failed
3. The task channel has bounded capacity (64); when full, new tasks get 503
4. Timeouts are capped at 10 minutes regardless of client request
5. Completed/failed tasks are garbage-collected after 1 hour
6. All responses include CORS headers for browser-based clients
7. Empty messages are rejected with 400

## Behavioral Examples

### Scenario: Submit and poll a task

- **Given** the A2A server is running on port 9091
- **When** a client sends `POST /a2a/tasks/send` with `{"message":"hello"}`
- **Then** the server returns `201` with `{"id":"<hex>","state":"submitted"}`
- **When** the client polls `GET /a2a/tasks/<hex>`
- **Then** the server returns `{"state":"working"}` while processing
- **Then** the server returns `{"state":"completed","response":"..."}` when done

### Scenario: Agent discovery

- **Given** the A2A server is running
- **When** a client fetches `GET /.well-known/agent.json`
- **Then** the server returns the agent card with name, address, version, and capabilities

### Scenario: P2P mode (no hub)

- **Given** the agent is running with `--no-hub`
- **When** a task is submitted via A2A
- **Then** the handler echoes the message back (placeholder for local processing)

## Error Cases

| Condition | Behavior |
|-----------|----------|
| Missing request body | Returns 400 `{"error":"missing request body"}` |
| Invalid JSON | Returns 400 `{"error":"invalid JSON: ..."}` |
| Empty message | Returns 400 `{"error":"message must not be empty"}` |
| Handler queue full | Returns 503 `{"error":"agent is busy, try again later"}` |
| Task not found | Returns 404 `{"error":"task not found"}` |
| Unknown route | Returns 404 `{"error":"not found"}` |
| Hub unreachable | Task fails with `"hub unreachable: ..."` error |
| Hub timeout | Task fails with `"timeout waiting for hub response"` error |

## Dependencies

### Consumes

| Module | What is used |
|--------|-------------|
| `tokio` | TcpListener, mpsc, RwLock, spawn |
| `serde` / `serde_json` | Request/response serialization |
| `rand` | Task ID generation |
| `hex` | Task ID encoding |
| `tracing` | Structured logging |

### Consumed By

| Module | What is used |
|--------|-------------|
| `src/main.rs` | `serve`, `TaskStore`, `A2aServerConfig`, `InboundTask` |

## Configuration

| Env Var / Flag | Default | Description |
|----------------|---------|-------------|
| `CAN_A2A_PORT` / `--a2a-port` | None (disabled) | Port to listen on |

## Change Log

| Date | Author | Change |
|------|--------|--------|
| 2026-04-07 | CorvidAgent | Initial spec |
