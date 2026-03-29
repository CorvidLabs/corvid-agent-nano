---
module: contacts
version: 1
status: active
files:
  - src/contacts.rs
depends_on: []
---

# Contacts

## Purpose

PSK (pre-shared key) contact management backed by SQLite. Provides CRUD operations for storing contacts with their Algorand addresses and 32-byte pre-shared keys, plus JSON export/import for backup and transfer. Contacts enable PSK-encrypted AlgoChat messaging between known parties.

## Public API

### Exported Structs

| Struct | Description |
|--------|-------------|
| `Contact` | A PSK contact entry: name, Algorand address, 32-byte PSK, and timestamp |
| `ContactStore` | SQLite-backed contact store with thread-safe `Mutex<Connection>` |

### Exported Functions

| Function | Parameters | Returns | Description |
|----------|-----------|---------|-------------|
| `parse_psk` | `input: &str` | `Result<[u8; 32]>` | Parse a PSK from hex (64 chars) or base64 (44 chars) format |
| `open` | `path: impl AsRef<Path>` | `Result<Self>` | Open or create the contacts SQLite database |
| `in_memory` | — | `Result<Self>` | Create an in-memory contacts database for testing |
| `add` | `name: &str`, `address: &str`, `psk: &[u8]` | `Result<()>` | Add a new contact; errors if name already exists |
| `upsert` | `name: &str`, `address: &str`, `psk: &[u8]` | `Result<()>` | Add or overwrite a contact (INSERT OR REPLACE) |
| `remove` | `name: &str` | `Result<bool>` | Remove a contact by name; returns true if deleted, false if not found |
| `list` | — | `Result<Vec<Contact>>` | List all contacts ordered by name |
| `get` | `name: &str` | `Result<Option<Contact>>` | Get a contact by name |
| `get_by_address` | `address: &str` | `Result<Option<Contact>>` | Get a contact by Algorand address |
| `export_json` | — | `Result<String>` | Export all contacts as pretty-printed JSON (PSK as hex) |
| `import_json` | `json: &str` | `Result<usize>` | Import contacts from JSON (merges via upsert); returns count imported |
| `count` | — | `Result<usize>` | Count the number of stored contacts |

### Contact Fields

| Field | Type | Description |
|-------|------|-------------|
| `name` | `String` | Contact name (primary key) |
| `address` | `String` | Algorand address |
| `psk` | `Vec<u8>` | 32-byte pre-shared key |
| `added_at` | `String` | ISO datetime when the contact was added |

### Database Schema

**contacts table:**
- `name TEXT PRIMARY KEY` — Contact name (unique identifier)
- `address TEXT NOT NULL` — Algorand address
- `psk BLOB NOT NULL` — 32-byte pre-shared key
- `added_at TEXT NOT NULL DEFAULT (datetime('now'))` — Timestamp

## Invariants

1. PSK must be exactly 32 bytes — `add` and `upsert` reject other lengths
2. Contact names are unique (PRIMARY KEY) — `add` fails on duplicate, `upsert` overwrites
3. `ContactStore` uses `Mutex<Connection>` for thread safety
4. Database table is created on open (`CREATE TABLE IF NOT EXISTS`)
5. JSON export encodes PSKs as hex strings; import decodes hex back to bytes
6. `list` returns contacts ordered alphabetically by name
7. `remove` returns `false` (not an error) when the contact doesn't exist

## Behavioral Examples

### Scenario: Add and list contacts

- **Given** an empty contact store
- **When** "alice" and "bob" are added with valid PSKs
- **Then** `list` returns both contacts in alphabetical order

### Scenario: Duplicate name rejected by add

- **Given** a contact "alice" already exists
- **When** `add("alice", ...)` is called again
- **Then** it returns an error suggesting `--force` to overwrite

### Scenario: Upsert overwrites existing contact

- **Given** a contact "alice" with address "ADDR1"
- **When** `upsert("alice", "ADDR2", ...)` is called
- **Then** the contact's address is updated to "ADDR2"

### Scenario: Export/import roundtrip

- **Given** a store with contacts "alice" and "bob"
- **When** exported to JSON and imported into a fresh store
- **Then** the new store contains both contacts with identical data

### Scenario: Parse PSK from hex

- **Given** a 64-character hex string
- **When** `parse_psk` is called
- **Then** it returns the decoded 32-byte array

## Error Cases

| Condition | Behavior |
|-----------|----------|
| PSK not 32 bytes | `add`/`upsert` returns error: "PSK must be exactly 32 bytes (got N)" |
| Duplicate name on `add` | Returns error: "Contact \"name\" already exists. Use --force to overwrite." |
| PSK not valid hex or base64 | `parse_psk` returns error: "Invalid PSK (not hex or base64)" |
| PSK decodes to wrong length | `parse_psk` returns error: "PSK must be 32 bytes (got N)" |
| Missing JSON field on import | Returns error: "Missing 'name'/'address'/'psk' field" |
| Invalid hex in JSON PSK | Returns hex decode error |
| Database open failure | Returns rusqlite error |

## Dependencies

### Consumes

| Module | What is used |
|--------|-------------|
| `rusqlite` | SQLite database access |
| `serde_json` | JSON serialization for export/import |
| `hex` | Hex encoding/decoding of PSKs |
| `base64` | Base64 decoding of PSKs in `parse_psk` |
| `anyhow` | Error handling |

### Consumed By

| Module | What is used |
|--------|-------------|
| `src/main.rs` | `ContactStore`, `parse_psk` for contact CLI subcommands |
| `src/agent.rs` | Contact lookup for PSK-encrypted messaging |

## Change Log

| Date | Author | Change |
|------|--------|--------|
| 2026-03-28 | CorvidAgent | Initial spec |
