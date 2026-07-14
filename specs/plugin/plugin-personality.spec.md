---
module: plugin-personality
version: 2
status: stable
files:
  - plugins/personality/src/lib.rs
depends_on:
  - specs/plugin/plugin-sdk.spec.md
  - specs/plugin/plugin-macros.spec.md
---

# Personality Plugin

## Purpose

WASM plugin that provides a configurable LLM-backed personality engine for corvid-agent. Exposes four tools for chat, configuration, persona management, and state inspection. Supports Claude and OpenAI as providers; custom providers (e.g. Ollama on a public host) are supported via a user-supplied base URL.

All plugin state is persisted via host KV so it survives across WASM Store resets. Emotion detection runs on every chat response, updating the `EmotionState` in KV for downstream plugins (e.g. avatar rendering).

Plugin ID: `corvid-personality`. Trust tier: `Trusted`.

## Public API

### Plugin Tools

| Tool | Description |
|------|-------------|
| `personality.chat` | Send a user message; get a persona-flavored LLM response. Maintains per-session conversation history |
| `personality.configure` | Set LLM provider, model, API key, and optional base URL |
| `personality.set-persona` | Set persona name, traits, speech style, and tone |
| `personality.get-state` | Read current emotion state and active persona config |

### Tool Input Schemas

#### `personality.chat`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `message` | `string` | yes | The user's message |
| `session_id` | `string` | no | Conversation session key (default: `"default"`) |

#### `personality.configure`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `provider` | `string` | yes | `"claude"`, `"openai"`, or `"custom"` |
| `model` | `string` | yes | Model identifier (e.g. `"claude-haiku-4-5-20251001"`) |
| `api_key` | `string` | yes | API key for the provider |
| `base_url` | `string` | no | Base URL override (required for `"custom"` provider) |

#### `personality.set-persona`

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | `string` | yes | Persona display name |
| `traits` | `string[]` | yes | Personality traits (e.g. `["curious", "playful"]`) |
| `speech_style` | `string` | yes | How the persona speaks (e.g. `"casual and warm"`) |
| `tone` | `string` | yes | Emotional tone (e.g. `"encouraging"`) |

#### `personality.get-state`

No input fields.

### Exported Structs

| Struct | Description |
|--------|-------------|
| `PersonalityConfig` | LLM provider, model, API key, optional base URL |
| `Persona` | Name, traits, speech style, tone |
| `EmotionState` | Current emotion label and intensity (0–100) |
| `Message` | Single conversation turn: role (`"user"` or `"assistant"`) and content |
| `Provider` | Enum: `Claude`, `OpenAI`, `Custom` |
| `PersonalityPlugin` | Root plugin struct implementing `CorvidPlugin` |

### Exported Functions

| Function | Description |
|----------|-------------|
| `kv_get` | Read plugin-scoped state from host KV storage |
| `kv_set` | Write plugin-scoped state to host KV storage |
| `http_post` | Send an allowlisted HTTP request through the host |
| `detect_emotion` | Derive emotion state from a response |
| `build_system_prompt` | Build the active persona system prompt |

### KV Storage Keys

| Key | Type | Description |
|-----|------|-------------|
| `config` | msgpack `PersonalityConfig` | Active LLM configuration |
| `persona` | msgpack `Persona` | Active persona definition |
| `emotion` | msgpack `EmotionState` | Latest detected emotion |
| `history:{session_id}` | msgpack `Vec<Message>` | Per-session conversation history |

### Defaults

| Item | Default |
|------|---------|
| Provider | `Claude` |
| Model | `claude-haiku-4-5-20251001` |
| Persona name | `Kira` |
| Persona traits | `["curious", "playful", "helpful"]` |
| Speech style | `"casual and warm"` |
| Tone | `"encouraging"` |
| Emotion | `neutral` at intensity 50 |

## Invariants

1. Plugin ID is `corvid-personality` — must match registry key
2. All state is stored in KV — plugin has no in-memory state between invocations
3. `PersonalityConfig.api_key` is stored in KV; it is the plugin author's responsibility to use Trusted tier to limit access
4. `Custom` provider requires a non-localhost `base_url` — localhost URLs are SSRF-blocked by the host
5. Emotion detection is keyword-based and runs on every assistant response — it does not call an external API
6. Conversation history is per-session; different `session_id` values are independent
7. The plugin requires `Network` and `Storage` capabilities; `AlgoRead`, `DbRead`, `FsProjectDir`, and `AgentMessage` are not requested
8. The plugin is compiled for `wasm32-wasip1` — the native build is test-only (host functions are stubbed)

## Behavioral Examples

### Scenario: First chat with unconfigured plugin

- **Given** the plugin has no `config` key in KV
- **When** `personality.chat` is called with `{"message": "hello"}`
- **Then** returns an error: `"LLM not configured — call personality.configure first"`

### Scenario: Configure then chat

- **Given** `personality.configure` was called with a valid Claude API key
- **When** `personality.chat` is called with `{"message": "tell me something fun"}`
- **Then** calls the Claude API with a system prompt built from the active persona and returns the response; updates `history:{session_id}` and `emotion` in KV

### Scenario: Emotion detection

- **Given** the LLM response contains excited or enthusiastic language
- **When** the emotion detector runs on the response
- **Then** `EmotionState.current` is set to `"excited"` or similar, and intensity is updated

### Scenario: Custom provider (Ollama)

- **Given** `personality.configure` with `provider: "custom"`, `base_url: "https://my-ollama.example.com"`
- **When** `personality.chat` is called
- **Then** HTTP POST is sent to `https://my-ollama.example.com/api/generate` with the model and prompt

## Error Cases

| Condition | Behavior |
|-----------|----------|
| `personality.chat` called before `personality.configure` | Returns `Err("LLM not configured — call personality.configure first")` |
| Unknown `provider` value | Returns `Err("unknown provider: ...")` |
| `Custom` provider with no `base_url` | Returns `Err("base_url required for custom provider")` |
| HTTP POST to LLM fails | Returns `Err("http_post failed: ...")` |
| LLM response not parseable as JSON | Returns `Err("failed to parse LLM response: ...")` |
| KV set fails silently | State not persisted; next invocation uses defaults |

## Dependencies

### Consumes

| Module | What is used |
|--------|-------------|
| `corvid-plugin-sdk` | `CorvidPlugin`, `PluginTool`, `PluginManifest`, `PluginEvent`, `ToolContext`, `Capability`, `TrustTier`, `EventKind`, `PluginError`, `InitContext` |
| `corvid-plugin-macros` | `#[corvid_plugin]` |
| `rmp-serde` | KV serialization |
| `serde_json` | LLM request/response JSON |

### Consumed By

| Module | What is used |
|--------|-------------|
| corvid-agent server | Invokes `personality.chat` to generate agent responses |
| Avatar/UI plugins | Reads `EmotionState` from shared KV for visual expression |

## Configuration

| Setting | How to set | Description |
|---------|-----------|-------------|
| LLM provider | `personality.configure` tool | `claude`, `openai`, or `custom` |
| Model | `personality.configure` tool | Model identifier string |
| API key | `personality.configure` tool | Stored in plugin-scoped KV |
| Base URL | `personality.configure` tool | Required for `custom` provider |
| Persona | `personality.set-persona` tool | Name, traits, speech style, tone |

## Change Log

| Date | Author | Change |
|------|--------|--------|
| 2026-04-18 | Jackdaw | Initial spec (spec-sync 4.x) — covers all four tools, KV storage layout, emotion detection, provider support |
| 2026-07-14 | SpecSync | CHG-0001-adopt-specsync-5-0-1-and-the-unified-trust-1-0-0-governance-gate: Adopt SpecSync 5.0.1 and the unified Trust 1.0.0 governance gate |
