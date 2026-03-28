---
module: hub
version: 1
status: active
files:
  - src/hub.rs
depends_on: []
---

# Hub Client

## Purpose

HTTP client for communicating with corvid-agent's API. Handles Flock Directory registration, periodic heartbeats, and A2A (agent-to-agent) task forwarding. When a nano agent receives an AlgoChat message it can't handle locally, it forwards to the hub via A2A and relays the response back on-chain.

## Public API

### Structs

| Struct | Description |
|--------|-------------|
| `HubClient` | HTTP client wrapping reqwest, tracks registration state |

### HubClient Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `new` | `(base_url: &str) -> Self` | Create client (strips trailing slash) |
| `register` | `(&mut self, address, name, encryption_key_hex) -> Result<String>` | Register with Flock Directory, returns agent ID |
| `heartbeat` | `(&self) -> Result<()>` | Send keepalive to maintain active status |
| `forward_message` | `(&self, message, sender_address) -> Result<String>` | Submit A2A task, poll for response, return text |

### API Endpoints Used

| Method | Endpoint | Description |
|--------|----------|-------------|
| `register` | `POST /api/flock-directory/agents` | Flock Directory registration |
| `heartbeat` | `POST /api/flock-directory/agents/{id}/heartbeat` | Status keepalive |
| `forward_message` | `POST /a2a/tasks/send` | Submit A2A task |
| `poll_task` | `GET /a2a/tasks/{id}` | Poll task status |

## Invariants

1. `agent_id` is `None` until `register` succeeds — `heartbeat` fails if not registered
2. A2A task polling retries every 3 seconds, up to 100 times (5 minute timeout)
3. Forward messages are prefixed with `[AlgoChat from {address}]` for context
4. A2A requests include `X-Source-Agent: nano` header
5. Task timeout is set to 300,000ms (5 minutes)
6. Response extraction finds the last `agent` role message's `text` part
7. Registration includes capabilities `["messaging", "lightweight"]`

## Behavioral Examples

### Scenario: Register and heartbeat

- **Given** a hub running at localhost:3578
- **When** `register` is called with valid address and name
- **Then** returns agent ID, subsequent `heartbeat` calls succeed

### Scenario: Forward message and get response

- **Given** a registered hub client
- **When** `forward_message` is called with an AlgoChat message
- **Then** submits A2A task, polls until completed, returns the agent's text response

### Scenario: Hub unreachable

- **Given** hub URL points to a non-running server
- **When** `register` is called
- **Then** returns error "Failed to connect to hub for registration"

## Error Cases

| Condition | Behavior |
|-----------|----------|
| Hub unreachable | Error: "Failed to connect to hub..." |
| Non-success HTTP status | Error with status code and body text |
| Not registered + heartbeat | Error: "Cannot heartbeat: not registered" |
| A2A task fails | Error: "A2A task failed" |
| A2A task timeout | Error: "A2A task timed out after polling" |

## Dependencies

### Consumes

| Module | What is used |
|--------|-------------|
| `reqwest` | HTTP client |
| `serde` | Request/response serialization |

### Consumed By

| Module | What is used |
|--------|-------------|
| `src/main.rs` | `HubClient::new`, `HubClient::register` |
| `src/agent.rs` | `HubClient::heartbeat`, `HubClient::forward_message` |

## Change Log

| Date | Author | Change |
|------|--------|--------|
| 2026-03-28 | CorvidAgent | Initial spec |
