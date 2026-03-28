---
module: module-name
version: 1
status: draft
files:
  - crates/module/src/file.rs
depends_on: []
---

# Module Name

## Purpose

<!-- Plain English: what this module does and why it exists -->

## Public API

### Exported Structs

| Struct | Description |
|--------|-------------|
| `ExampleStruct` | Represents a thing |

### Exported Functions

| Function | Parameters | Returns | Description |
|----------|-----------|---------|-------------|
| `example` | `(db: &Database, id: &str)` | `Result<Thing>` | Fetches a thing by ID |

### Exported Traits

| Trait | Description |
|-------|-------------|
| `ExampleTrait` | Defines behavior for things |

## Invariants

<!-- Rules that must ALWAYS hold. Use numbered list. -->

1. Example invariant that must always be true

## Behavioral Examples

### Scenario: Example scenario

- **Given** some precondition
- **When** an action occurs
- **Then** this result is expected

## Error Cases

| Condition | Behavior |
|-----------|----------|
| Thing not found | Returns `Err(anyhow!("not found"))` |

## Dependencies

### Consumes

| Module | What is used |
|--------|-------------|
| `crates/other/src/lib.rs` | `OtherStruct` |

### Consumed By

| Module | What is used |
|--------|-------------|
| `src/main.rs` | All exported types |

## Configuration

| Env Var / Flag | Default | Description |
|----------------|---------|-------------|
| `EXAMPLE_VAR` | `100` | Controls example behavior |

## Change Log

| Date | Author | Change |
|------|--------|--------|
| YYYY-MM-DD | name | Initial spec |
