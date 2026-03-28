---
module: identity
version: 1
status: active
files:
  - src/identity.rs
depends_on: []
---

# Identity

## Purpose

Agent identity generation — creates random Ed25519 seeds and derives Algorand addresses. Used by `can init` to bootstrap a new agent identity.

## Public API

| Function | Signature | Description |
|----------|-----------|-------------|
| `generate_seed` | `() -> [u8; 32]` | Generate a cryptographically random 32-byte seed |
| `address_from_seed` | `(seed: &[u8; 32]) -> String` | Derive Algorand address from Ed25519 seed |

## Invariants

1. `generate_seed` uses `rand::rng().fill_bytes` for CSPRNG randomness
2. Address derivation: Ed25519 seed → signing key → verifying key (public key) → base32(pubkey ∥ checksum)
3. Checksum = last 4 bytes of SHA-512/256(public_key)
4. Addresses are 58 characters, uppercase alphanumeric (base32 no-pad encoding)
5. Same seed always produces the same address (deterministic)

## Dependencies

### Consumes

| Module | What is used |
|--------|-------------|
| `ed25519-dalek` | `SigningKey` for key derivation |
| `data-encoding` | `BASE32_NOPAD` for address encoding |
| `sha2` | `Sha512_256` for checksum |
| `rand` | CSPRNG for seed generation |

### Consumed By

| Module | What is used |
|--------|-------------|
| `src/main.rs` | `generate_seed`, `address_from_seed` in `cmd_init` |

## Change Log

| Date | Author | Change |
|------|--------|--------|
| 2026-03-28 | CorvidAgent | Initial spec |
