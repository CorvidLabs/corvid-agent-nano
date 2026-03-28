---
module: plugin-cli
version: 1
status: active
files:
  - corvid-plugin-cli/src/main.rs
  - corvid-plugin-cli/src/scaffold.rs
  - corvid-plugin-cli/src/validate.rs
depends_on:
  - specs/plugin/plugin-sdk.spec.md
  - specs/plugin/plugin-host.spec.md
---

# Plugin CLI

## Purpose

Command-line tool for plugin authors and operators. Scaffolds new plugin projects from templates, validates manifests and capability declarations against the SDK spec, and installs plugins from GitHub releases into the local plugin host. Invoked as `corvid plugin <subcommand>`.

## Public API

### CLI Subcommands

| Subcommand | Arguments | Description |
|------------|-----------|-------------|
| `new` | `<name> [--author <author>] [--tier <tier>]` | Scaffold a new plugin project from template |
| `validate` | `[path]` | Validate manifest, capabilities, and tool schemas of a built plugin |
| `install` | `<source>@<version>` | Install a plugin from GitHub release |
| `list` | `[--json]` | List installed plugins (queries running host via socket) |
| `uninstall` | `<plugin-id>` | Remove an installed plugin |

### Exported Functions

| Function | Parameters | Returns | Description |
|----------|-----------|---------|-------------|
| `scaffold` | `(name: &str, author: &str, tier: TrustTier)` | `Result<PathBuf>` | Generate plugin project directory from template |
| `validate_plugin` | `(wasm_path: &Path)` | `Result<ValidationReport>` | Full validation: ABI, manifest, capabilities, tool schemas |

## Modules

### main.rs — CLI Entry Point

Parses subcommands via `clap`. Dispatches to scaffold, validate, or install flows.

### scaffold.rs — Template Generation

Generates a complete plugin project directory:

```
corvid-plugin-<name>/
├── Cargo.toml          # [lib] crate-type = ["cdylib"]
├── plugin.toml         # Installable artifact descriptor
├── src/lib.rs          # Skeleton CorvidPlugin impl with #[corvid_plugin]
└── .github/workflows/release.yml  # CI: build wasm32-wasip1, sign, release
```

**`Cargo.toml` template:**
```toml
[lib]
crate-type = ["cdylib"]

[dependencies]
corvid-plugin-sdk    = "0.1"
corvid-plugin-macros = "0.1"

[profile.release]
opt-level = "z"
lto       = true
strip     = true

[features]
dev-mode = []
```

**`plugin.toml` template:**
```toml
[plugin]
id          = "corvid-<name>"
version     = "0.1.0"
trust-tier  = "<tier>"
sdk-version = "^0.1"

[build]
target        = "wasm32-wasip1"
wasm-artifact = "corvid_<name>.wasm"
```

### validate.rs — Manifest + Capability Audit

Validation checks (same as host loader, but offline):

1. ABI version within supported range
2. Plugin ID matches `^[a-z][a-z0-9-]{0,49}$`
3. Version is valid semver
4. `min_host_version` is valid semver
5. All capabilities are known (no unknown variants)
6. No duplicate tool names
7. Each tool's `input_schema` is valid JSON Schema v7
8. Declared capabilities match what the trust tier allows

### Install Flow

```
corvid plugin install CorvidLabs/corvid-plugin-algo-oracle@0.3.1
```

1. Fetch `plugin.toml` from GitHub release
2. Download `.wasm` artifact
3. Verify Ed25519 signature (Trusted tier)
4. Run `validate_plugin()` — fail hard on unknown capabilities
5. Register in `corvid-agent.db` plugins table + copy to `plugins/` dir
6. Send JSON-RPC reload signal to running plugin host (graceful, via drain-and-reload)

**Plugin DB table (SQLite):**
```sql
CREATE TABLE plugins (
    id           TEXT PRIMARY KEY,
    version      TEXT NOT NULL,
    tier         TEXT NOT NULL,  -- 'trusted'|'verified'|'untrusted'
    wasm_hash    TEXT NOT NULL,
    installed_at INTEGER NOT NULL,
    enabled      INTEGER NOT NULL DEFAULT 1
);
```

## Invariants

1. `corvid plugin new` always generates a buildable project — `cargo build --target wasm32-wasip1` must succeed on the scaffold output
2. `corvid plugin validate` performs the same checks as the host loader — a plugin that passes validate will pass host loading
3. Install never overwrites without confirmation if a different version is already installed
4. Ed25519 signature verification is mandatory for Trusted tier installs
5. Install sends a reload signal to the running host — does not require host restart
6. Plugin ID in `plugin.toml` must match the `PluginManifest.id` in the WASM binary

## Behavioral Examples

### Scenario: Scaffold a new plugin

- **Given** `corvid plugin new algo-watcher --author "CorvidLabs" --tier verified`
- **When** the command runs
- **Then** creates `corvid-plugin-algo-watcher/` with Cargo.toml, plugin.toml, src/lib.rs skeleton, and CI workflow

### Scenario: Validate a built plugin

- **Given** a compiled `corvid_algo_oracle.wasm`
- **When** `corvid plugin validate ./target/wasm32-wasip1/release/corvid_algo_oracle.wasm`
- **Then** reports: ABI OK, manifest valid, 2 tools found (schemas valid), capabilities within Trusted tier

### Scenario: Install from GitHub

- **Given** `corvid plugin install CorvidLabs/corvid-plugin-algo-oracle@0.3.1`
- **When** the release exists with `.wasm` + `.sig` + `plugin.toml`
- **Then** downloads, verifies signature, validates manifest, registers in DB, signals host to reload

## Error Cases

| Condition | Behavior |
|-----------|----------|
| Scaffold name conflicts with existing directory | Error: directory already exists |
| Validate: ABI mismatch | Error report with expected range |
| Validate: unknown capability | Error report listing unknown variants |
| Install: GitHub release not found | Error: release not found for version |
| Install: signature missing for Trusted tier | Error: Ed25519 signature required |
| Install: signature invalid | Error: signature verification failed |
| Install: host not running | Warning: plugin registered but host not notified (will load on next start) |
| Install: plugin.toml ID != WASM manifest ID | Error: ID mismatch |

## Dependencies

### Consumes

| Module | What is used |
|--------|-------------|
| `corvid-plugin-sdk` | `PluginManifest`, `Capability`, `TrustTier`, `ABI_VERSION` |
| `clap` | CLI argument parsing |
| `reqwest` | GitHub release API + artifact download |
| `ed25519-dalek` | Signature verification |
| `rusqlite` | Plugin registry DB |

### Consumed By

| Module | What is used |
|--------|-------------|
| Plugin authors | `corvid plugin new`, `corvid plugin validate` |
| Operators | `corvid plugin install`, `corvid plugin list` |

## Configuration

| Env Var / Flag | Default | Description |
|----------------|---------|-------------|
| `--data-dir` | `~/.corvid` | Base directory for DB and plugin storage |
| `--socket-path` | `{data_dir}/plugins.sock` | Socket for reload signal to running host |

## Change Log

| Date | Author | Change |
|------|--------|--------|
| 2026-03-28 | CorvidAgent | Initial spec from council synthesis (Issue #15) |
| 2026-03-28 | CorvidAgent | v2: Implemented — scaffold, validate, install, list, uninstall subcommands |
