---
module: vault
version: 1
status: active
files:
  - src/vault.rs
depends_on: []
---

# Vault

## Purpose

Encrypted on-disk storage for agent secrets (Ed25519 seed and contact PSKs). Uses Argon2id for passphrase-based key derivation and ChaCha20-Poly1305 for authenticated encryption. Secrets are zeroized from memory on drop via the `zeroize` crate.

## Public API

### Structs

| Struct | Description |
|--------|-------------|
| `Vault` | Static methods for create/open/update/exists operations on vault files |
| `VaultContents` | Plaintext vault data: seed, address, contacts. Implements `Zeroize` + `ZeroizeOnDrop` |
| `Contact` | A contact entry: name, Algorand address, PSK bytes |

### VaultContents Fields

| Field | Type | Description |
|-------|------|-------------|
| `seed_hex` | `String` | 32-byte Ed25519 seed (hex-encoded) |
| `address` | `String` | Algorand address derived from seed |
| `contacts` | `Vec<Contact>` | Known contacts with PSKs |

### Contact Fields

| Field | Type | Description |
|-------|------|-------------|
| `name` | `String` | Human-readable contact name |
| `address` | `String` | Contact's Algorand address |
| `psk` | `Vec<u8>` | Pre-shared key bytes (base64-encoded in JSON via serde helper) |

### Vault Methods

| Method | Signature | Description |
|--------|-----------|-------------|
| `create` | `(path: &Path, contents: &VaultContents, passphrase: &str) -> Result<()>` | Create a new encrypted vault file |
| `open` | `(path: &Path, passphrase: &str) -> Result<VaultContents>` | Decrypt and return vault contents |
| `update` | `(path: &Path, passphrase: &str, f: FnOnce(&mut VaultContents)) -> Result<()>` | Decrypt, apply mutation, re-encrypt |
| `exists` | `(path: &Path) -> bool` | Check if vault file exists |

## File Format

```
MAGIC(4 bytes: "NANO") || VERSION(1 byte: 0x01) || SALT(16 bytes) || NONCE(12 bytes) || CIPHERTEXT(variable)
```

- Salt: random 16 bytes for Argon2id
- Nonce: random 12 bytes for ChaCha20-Poly1305
- Ciphertext: JSON-serialized `VaultContents` encrypted with derived key

## Invariants

1. File format starts with magic bytes `NANO` and version byte `0x01`
2. Salt is 16 bytes, nonce is 12 bytes, derived key is 32 bytes
3. Key derivation uses `Argon2::default()` (Argon2id with standard params)
4. `VaultContents` implements `ZeroizeOnDrop` — secrets are wiped when the struct is dropped
5. Vault file permissions are set to `0600` on Unix (owner read/write only)
6. Parent directory permissions are best-effort `0700` (may fail for `/tmp`)
7. `Contact.psk` is serialized as base64 in JSON via custom serde helper
8. Wrong passphrase produces "Decryption failed" error (ChaCha20-Poly1305 auth tag mismatch)
9. Corrupted/truncated files produce "Vault file too short" or "bad magic" errors

## Behavioral Examples

### Scenario: Create and open roundtrip

- **Given** a passphrase and vault contents with one contact
- **When** `Vault::create` then `Vault::open` with the same passphrase
- **Then** recovered contents match original (seed, address, contacts, PSK bytes)

### Scenario: Wrong passphrase

- **Given** a vault created with passphrase "correct"
- **When** `Vault::open` is called with passphrase "wrong"
- **Then** returns error (decryption fails)

### Scenario: Update adds contact

- **Given** an existing vault with no contacts
- **When** `Vault::update` pushes a new contact
- **Then** subsequent `Vault::open` shows the contact

## Error Cases

| Condition | Behavior |
|-----------|----------|
| File too short (< header size) | Error: "Vault file too short" |
| Bad magic bytes | Error: "Not a valid vault file (bad magic)" |
| Unsupported version | Error: "Unsupported vault version: N" |
| Wrong passphrase | Error: "Decryption failed — wrong passphrase?" |
| Serialization failure | Error: "Failed to serialize vault" |
| File write failure | Error: "Failed to write vault file" |

## Dependencies

### Consumes

| Module | What is used |
|--------|-------------|
| `argon2` | Argon2id key derivation |
| `chacha20poly1305` | Authenticated encryption |
| `zeroize` | Memory zeroization |
| `serde` / `serde_json` | JSON serialization of vault contents |
| `rand` | Random salt and nonce generation |
| `data-encoding` | Base64 encoding for PSK serde helper |

### Consumed By

| Module | What is used |
|--------|-------------|
| `src/main.rs` | `Vault::create`, `Vault::open`, `Vault::update`, `Vault::exists`, `VaultContents`, `Contact` |

## Change Log

| Date | Author | Change |
|------|--------|--------|
| 2026-03-28 | CorvidAgent | Initial spec |
