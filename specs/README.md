# Module Specifications

Structured markdown specs are the **source of truth** for what each module in corvid-agent-nano should do. They exist so that:

1. **The owner** can define and review module behavior without reading Rust
2. **Agents** can validate their changes against a formal contract
3. **Correctness** can be checked automatically via `cargo specsync check` or `./scripts/specsync.sh check`

Validation is powered by [SpecSync](https://github.com/marketplace/actions/specsync), a Rust-based spec-to-code validator available on the GitHub Marketplace.

## Reading a Spec

Each `.spec.md` file has two parts:

### YAML Frontmatter

```yaml
---
module: module-name
version: 1
status: draft | active | deprecated
files:
  - crates/module/src/file.rs
depends_on:
  - specs/other/module.spec.md
---
```

- **module**: Human-readable identifier
- **version**: Increment when the spec changes materially
- **status**: `draft` (untested), `active` (validated), `deprecated` (superseded)
- **files**: Source files this spec covers
- **depends_on**: Other specs this module requires

### Markdown Sections

| Section | What it contains |
|---------|-----------------|
| **Purpose** | Plain English: what this module does and why it exists |
| **Public API** | Tables of exported structs, functions, traits with signatures |
| **Invariants** | Rules that must ALWAYS hold (state machines, ordering, uniqueness) |
| **Behavioral Examples** | Given/When/Then scenarios |
| **Error Cases** | Table of error conditions and expected behavior |
| **Dependencies** | What this module consumes and what consumes it |
| **Configuration** | Environment variables / CLI flags with defaults |
| **Change Log** | Date/author/change history |

## Creating a New Spec

1. Copy `_template.spec.md` to the appropriate subdirectory
2. Fill in the YAML frontmatter with the correct files
3. Write each required section
4. Run `./scripts/specsync.sh check` to validate structure and API coverage
5. Set status to `active` once validated

## How Agents Use Specs

Before modifying any file listed in a spec's `files:` frontmatter:
1. Read the corresponding spec
2. Understand its invariants
3. After modifying, run `./scripts/specsync.sh check`
4. If your change violates a spec invariant, update the spec first (add a Change Log entry)

**Specs take precedence over code comments.** If code contradicts the spec, the code is the bug.

## Validation

```bash
./scripts/specsync.sh check
./scripts/specsync.sh check --strict     # treat warnings as errors
./scripts/specsync.sh coverage           # file coverage report
```

The validator checks three levels:

1. **Structural** — Frontmatter fields, file existence, required sections
2. **API Surface** — Exported symbols in source match the spec's Public API tables
3. **Dependencies** — All referenced specs and consumed-by files exist

Warnings (undocumented exports) don't fail. Errors (missing files, broken refs) cause exit code 1.

## Directory Layout

```
specs/
  README.md                        — This file
  _template.spec.md                — Copy-paste template
  core/
    core.spec.md                   — Shared types (AgentIdentity, Message, NanoConfig)
  binary/
    nano-cli.spec.md               — CAN CLI binary entry point (subcommands, dispatch)
  vault/
    vault.spec.md                  — Encrypted vault (Argon2id + ChaCha20-Poly1305)
  identity/
    identity.spec.md               — Seed generation + Algorand address derivation
  hub/
    hub.spec.md                    — Hub client (Flock Directory, A2A forwarding)
  transaction/
    transaction.spec.md            — Algorand transaction construction + signing
  agent/
    agent.spec.md                  — Message polling loop
```
