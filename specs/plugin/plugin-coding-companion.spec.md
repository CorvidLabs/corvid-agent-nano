---
module: plugin-coding-companion
version: 1
status: stable
files:
  - plugins/coding-companion/src/lib.rs
  - plugins/coding-companion/Cargo.toml
depends_on:
  - specs/plugin/plugin-sdk.spec.md
  - specs/plugin/plugin-host.spec.md
---

# Coding Companion Plugin

## Purpose

WASM plugin that provides an LLM-backed coding assistant for corvid-agent. Exposes five tools for code analysis, code review, code explanation, interactive Q&A, and session management.

Uses `host_llm_chat` (the `LlmChat` capability) so API keys are never stored in WASM memory — the host reads `CORVID_LLM_*` env vars. Uses `host_fs_read` (`FsProjectDir`) to load project files as context.

Plugin ID: `coding-companion`. Trust tier: `Trusted`.

## Public API

### Plugin Tools

| Tool | Description |
|------|-------------|
| `code.analyze` | Detect bugs, security issues, and improvements in code |
| `code.review` | Structured code review (correctness, security, performance, style) |
| `code.explain` | Plain-English explanation of what code does |
| `code.ask` | General coding Q&A with optional file context and session history |
| `code.clear_history` | Clear the conversation history for a session |

### Tool Input Schemas

#### `code.analyze`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `code` | `string` | * | Inline source code to analyze |
| `path` | `string` | * | Project-relative path (alternative to `code`) |
| `focus` | `string` | no | Area to focus on: `security`, `performance`, `correctness`, `style`, or `all` (default: `all areas`) |

*Either `code` or `path` must be provided.

#### `code.review`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `code` | `string` | * | Inline source code to review |
| `path` | `string` | * | Project-relative path (alternative to `code`) |

#### `code.explain`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `code` | `string` | * | Inline source code to explain |
| `path` | `string` | * | Project-relative path (alternative to `code`) |

#### `code.ask`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `message` | `string` | yes | The user's coding question |
| `path` | `string` | no | Project-relative file to include as context |
| `session_id` | `string` | no | Session key for conversation continuity (default: `"default"`) |

#### `code.clear_history`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `session_id` | `string` | no | Session to clear (default: `"default"`) |

### KV Storage Keys

| Key | Type | Description |
|-----|------|-------------|
| `session:{session_id}` | msgpack `Vec<ChatMessage>` | Per-session conversation history for `code.ask` |

### Response Shapes

All tools return msgpack-encoded JSON. On success:

| Tool | Success Shape |
|------|--------------|
| `code.analyze` | `{ ok: true, analysis: string }` |
| `code.review` | `{ ok: true, review: string }` |
| `code.explain` | `{ ok: true, explanation: string }` |
| `code.ask` | `{ ok: true, reply: string, session_id: string }` |
| `code.clear_history` | `{ ok: true, message: string }` |

On error: `{ ok: false, error: string }`.

## Invariants

1. Plugin ID is `coding-companion` — must match registry key
2. Either `code` or `path` must be provided for analyze/review/explain; both may be supplied (code takes precedence)
3. `path` is validated by the host (`FsProjectDir` capability) — path traversal outside project dir returns an error
4. Session history for `code.ask` grows unbounded within a session; call `code.clear_history` to reset
5. `code.ask` history includes only `user`/`assistant` turns; file context is prepended to the user message, not stored as a separate role
6. The plugin requires `LlmChat`, `Storage`, and `FsProjectDir` capabilities; no `Network`, `AlgoRead`, `DbRead`, or `AgentMessage`
7. Compiled for `wasm32-wasip1` — native build is test-only (all host functions are stubbed)

## Behavioral Examples

### Scenario: Analyze inline code

- **Given** `code.analyze` called with `{ "code": "fn foo() { panic!() }", "focus": "correctness" }`
- **Then** the host LLM is called with a correctness-focused system prompt and the code block
- **Returns** `{ "ok": true, "analysis": "..." }`

### Scenario: Review a project file

- **Given** `code.review` called with `{ "path": "src/main.rs" }`
- **Then** `host_fs_read("src/main.rs")` loads the file; the LLM performs a full review
- **Returns** `{ "ok": true, "review": "..." }`

### Scenario: Multi-turn ask session

- **Given** `code.ask` called with `{ "message": "What does this plugin do?", "session_id": "s1" }`
- **Then** LLM is called; reply and user message are saved under `session:s1`
- **When** follow-up `{ "message": "How do I call it?", "session_id": "s1" }` is sent
- **Then** full history is passed to the LLM for context-aware reply

### Scenario: Ask with file context

- **Given** `code.ask` called with `{ "message": "Is this safe?", "path": "src/auth.rs" }`
- **Then** `host_fs_read("src/auth.rs")` loads the file; content is prepended to the user message
- **Returns** `{ "ok": true, "reply": "...", "session_id": "default" }`

## Error Cases

| Condition | Behavior |
|-----------|----------|
| Neither `code` nor `path` provided | Returns `Err("provide either code or path")` |
| `path` does not exist or is outside project dir | Returns `Err("could not read file: {path}")` |
| `LlmChat` capability not configured on host | Returns `Err("LLM error: LlmChat capability not configured")` |
| `message` missing or empty in `code.ask` | Returns `Err("missing required field: message")` / `Err("message cannot be empty")` |

## Dependencies

### Consumes

| Module | What is used |
|--------|-------------|
| `corvid-plugin-sdk` | `Capability`, `PluginManifest`, `ToolInfo`, `TrustTier`, `ABI_VERSION` |
| `rmp-serde` | KV and LLM request/response serialization |
| `serde_json` | Tool input parsing and response construction |

### Consumed By

| Module | What is used |
|--------|-------------|
| corvid-agent server | Invokes tools for coding assistance in agent workflows |

## Configuration

LLM provider is configured entirely on the host via environment variables:

| Env Var | Description |
|---------|-------------|
| `CORVID_LLM_PROVIDER` | `claude`, `openai`, or `ollama` |
| `CORVID_LLM_API_KEY` | API key (not required for Ollama) |
| `CORVID_LLM_MODEL` | Model name override |
| `CORVID_LLM_ENDPOINT` | Endpoint URL override |

## Change Log

| Date | Author | Change |
|------|--------|--------|
| 2026-04-18 | Jackdaw | Initial spec — five tools, host LLM chat, FsProjectDir context, session history |
