---
module: crypto
version: 1
status: active
files:
  - crates/crypto/src/lib.rs
  - crates/crypto/src/identity.rs
  - crates/crypto/src/encrypt.rs
depends_on:
  - specs/core/core.spec.md
---

# Crypto

## Purpose

X25519 key exchange and ChaCha20-Poly1305 authenticated encryption for AlgoChat messages. This crate provides the same cryptographic protocol as corvid-agent's TypeScript implementation, ensuring cross-platform interoperability. Any message encrypted by the Rust side can be decrypted by the TypeScript side, and vice versa.

## Public API

### Exported Structs

| Struct | Description |
|--------|-------------|
| `KeyPair` | X25519 keypair — holds a `StaticSecret` and derived `PublicKey` |

### KeyPair Methods

| Method | Parameters | Returns | Description |
|--------|-----------|---------|-------------|
| `generate` | `()` | `Self` | Generate a new random X25519 keypair using OS entropy |
| `from_secret_b64` | `(secret_b64: &str)` | `Result<Self>` | Reconstruct keypair from a base64-encoded 32-byte secret key |
| `public_key_b64` | `(&self)` | `String` | Export the public key as base64 |
| `secret_key_b64` | `(&self)` | `String` | Export the secret key as base64 (for persistence) |
| `diffie_hellman` | `(&self, their_public: &[u8; 32])` | `[u8; 32]` | Perform X25519 DH to derive a 32-byte shared secret |

### Exported Functions

| Function | Parameters | Returns | Description |
|----------|-----------|---------|-------------|
| `encrypt` | `(shared_secret: &[u8; 32], plaintext: &[u8])` | `Result<Vec<u8>>` | Encrypt with ChaCha20-Poly1305; returns `nonce (12B) \|\| ciphertext` |
| `decrypt` | `(shared_secret: &[u8; 32], data: &[u8])` | `Result<Vec<u8>>` | Decrypt `nonce \|\| ciphertext` with ChaCha20-Poly1305 |

### Re-exports (lib.rs)

| Symbol | Source | Description |
|--------|--------|-------------|
| `KeyPair` | `identity::KeyPair` | Re-exported for convenience |

## Invariants

1. `KeyPair::generate()` uses `OsRng` — never a deterministic or seeded RNG
2. `encrypt()` prepends a random 12-byte nonce to the ciphertext; every call produces different output for the same input
3. `decrypt()` expects exactly the format produced by `encrypt()`: first 12 bytes are nonce, remainder is ciphertext + auth tag
4. The wire format is interoperable with corvid-agent's TypeScript `encryptMessage()` / `decryptMessage()` — same key derivation (X25519 DH), same cipher (ChaCha20-Poly1305), same nonce-prefix layout
5. `from_secret_b64` rejects input that doesn't decode to exactly 32 bytes
6. `diffie_hellman` is commutative: `A.dh(B.pub) == B.dh(A.pub)`
7. Shared secrets from `diffie_hellman` are 32 bytes and suitable as ChaCha20-Poly1305 keys without further derivation

## Behavioral Examples

### Scenario: Generate keypair and export/import

- **Given** a newly generated `KeyPair`
- **When** the secret key is exported via `secret_key_b64()` and re-imported via `from_secret_b64()`
- **Then** the reconstructed keypair has the same `public_key_b64()`

### Scenario: Encrypt/decrypt roundtrip

- **Given** a shared secret `[42u8; 32]` and plaintext `b"hello from nano"`
- **When** encrypted then decrypted
- **Then** the decrypted output equals the original plaintext

### Scenario: Cross-agent message exchange

- **Given** Agent A and Agent B each with their own `KeyPair`
- **When** A derives `shared = A.diffie_hellman(B.public)` and B derives `shared = B.diffie_hellman(A.public)`
- **Then** both shared secrets are identical, and A can decrypt messages B encrypted with that secret

### Scenario: Decrypt with wrong key

- **Given** a message encrypted with shared secret X
- **When** decryption is attempted with a different shared secret Y
- **Then** `decrypt()` returns `Err` ("decryption failed")

## Error Cases

| Condition | Behavior |
|-----------|----------|
| `from_secret_b64` with invalid base64 | Returns `Err` (base64 decode error) |
| `from_secret_b64` with wrong-length bytes | Panics at `copy_from_slice` (must be exactly 32 bytes) |
| `decrypt` with data shorter than 12 bytes | Returns `Err("data too short for nonce")` |
| `decrypt` with wrong shared secret | Returns `Err("decryption failed: ...")` |
| `decrypt` with tampered ciphertext | Returns `Err("decryption failed: ...")` (AEAD auth tag check fails) |

## Dependencies

### Consumes

| Module | What is used |
|--------|-------------|
| `x25519-dalek` | `StaticSecret`, `PublicKey` for key exchange |
| `chacha20poly1305` | `ChaCha20Poly1305`, `Aead`, `KeyInit`, `Nonce` |
| `rand` | `OsRng`, `RngCore` for nonce and key generation |
| `base64` | `Engine`, `STANDARD` for key serialization |
| `anyhow` | `Result`, `anyhow!` for error handling |

### Consumed By

| Module | What is used |
|--------|-------------|
| `crates/algochat/src/client.rs` | `KeyPair`, `encrypt`, `decrypt` for message encryption |
| `crates/algochat/src/listener.rs` | `decrypt` for incoming message decryption |
| `src/main.rs` | `KeyPair` for identity initialization |

## Change Log

| Date | Author | Change |
|------|--------|--------|
| 2026-03-28 | CorvidAgent | Initial spec |
