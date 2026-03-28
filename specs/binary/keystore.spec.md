---
module: keystore
version: 1
status: active
files:
  - src/keystore.rs
depends_on: []
---

# Keystore

## Purpose

Encrypted keystore for password-protected seed storage. Uses Argon2id for key derivation and ChaCha20-Poly1305 for authenticated encryption. Provides create/load operations for a JSON keystore file format, enabling users to persist wallet seeds securely on disk with password protection.

## Public API

### Exported Structs

| Struct | Description |
|--------|-------------|
| `Keystore` | JSON envelope for the encrypted keystore file: version, KDF params, cipher, ciphertext, nonce, and plaintext address |
| `KdfParams` | Argon2id parameters: memory cost, time cost, parallelism, and salt |

### Exported Functions

| Function | Parameters | Returns | Description |
|----------|-----------|---------|-------------|
| `create_keystore` | `seed: &[u8; 32]`, `address: &str`, `password: &str`, `path: &Path` | `Result<()>` | Encrypt a seed with Argon2id + ChaCha20-Poly1305 and save to a JSON keystore file with 0o600 permissions |
| `load_keystore` | `path: &Path`, `password: &str` | `Result<([u8; 32], String)>` | Decrypt a seed from a keystore file, returning the 32-byte seed and the stored address |
| `keystore_exists` | `path: &Path` | `bool` | Check if a keystore file exists at the given path |
| `keystore_address` | `path: &Path` | `Result<String>` | Read the plaintext address from a keystore file without decrypting |

### Internal Functions

| Function | Description |
|----------|-------------|
| `derive_key` | Derive a 32-byte encryption key from password + salt using Argon2id (64 MiB, 3 iterations, 1 thread) |

### Keystore File Format (JSON)

| Field | Type | Description |
|-------|------|-------------|
| `version` | `u32` | Format version (currently 1) |
| `kdf` | `String` | Key derivation function identifier ("argon2id") |
| `kdf_params` | `KdfParams` | Argon2id parameters |
| `cipher` | `String` | Cipher identifier ("chacha20-poly1305") |
| `ciphertext` | `String` | Base64-encoded encrypted seed |
| `nonce` | `String` | Base64-encoded 12-byte nonce |
| `address` | `String` | Algorand address (plaintext, for identification) |

### KdfParams Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `m_cost` | `u32` | 65536 | Memory cost in KiB (64 MiB) |
| `t_cost` | `u32` | 3 | Number of iterations |
| `p_cost` | `u32` | 1 | Parallelism degree |
| `salt` | `String` | (random) | Base64-encoded 16-byte salt |

## Invariants

1. Passwords must be at least 8 characters — `create_keystore` rejects shorter passwords
2. Keystore files are written atomically: write to `.tmp`, then rename
3. On Unix, keystore files are set to 0o600 permissions (owner read/write only)
4. Derived keys are zeroized after use via the `zeroize` crate
5. Only keystore version 1 is supported — `load_keystore` rejects other versions
6. The address field is stored in plaintext and can be read without the password
7. Argon2id parameters are fixed: 64 MiB memory, 3 iterations, 1 thread, 32-byte output
8. Each keystore uses a unique random 16-byte salt and 12-byte nonce

## Behavioral Examples

### Scenario: Create and load a keystore

- **Given** a 32-byte seed, an address, and a password of 8+ characters
- **When** `create_keystore` is called followed by `load_keystore` with the same password
- **Then** the recovered seed and address match the originals

### Scenario: Wrong password

- **Given** a keystore created with password "correctpassword"
- **When** `load_keystore` is called with "wrongpassword"
- **Then** it returns an error containing "wrong password"

### Scenario: Read address without decryption

- **Given** an existing keystore file
- **When** `keystore_address` is called
- **Then** it returns the plaintext address without requiring the password

### Scenario: Restrictive file permissions (Unix)

- **Given** a newly created keystore file on a Unix system
- **When** the file permissions are checked
- **Then** they are 0o600 (owner read/write only)

## Error Cases

| Condition | Behavior |
|-----------|----------|
| Password shorter than 8 characters | Returns error: "Password must be at least 8 characters" |
| Wrong password on load | Decryption fails with "wrong password?" error |
| Unsupported keystore version | Returns error: "Unsupported keystore version: N" |
| Keystore file not found | Returns filesystem error from `read_to_string` |
| Invalid JSON in keystore file | Returns serde deserialization error |
| Invalid base64 in salt/nonce/ciphertext | Returns error identifying the invalid field |
| Decrypted seed wrong length | Returns error: "Decrypted seed has wrong length: N" |
| Filesystem write failure | Returns IO error |

## Dependencies

### Consumes

| Module | What is used |
|--------|-------------|
| `argon2` | Argon2id key derivation |
| `chacha20poly1305` | ChaCha20-Poly1305 AEAD encryption/decryption |
| `rand` | CSPRNG for salt and nonce generation |
| `base64` | Encoding/decoding of salt, nonce, and ciphertext |
| `serde` / `serde_json` | JSON serialization of keystore format |
| `zeroize` | Secure memory clearing of derived keys |

### Consumed By

| Module | What is used |
|--------|-------------|
| `src/main.rs` | `create_keystore`, `load_keystore`, `keystore_exists`, `keystore_address` for wallet CLI subcommands |

## Change Log

| Date | Author | Change |
|------|--------|--------|
| 2026-03-28 | CorvidAgent | Initial spec |
