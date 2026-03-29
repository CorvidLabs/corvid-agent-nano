# Security Model

## Wallet encryption

The agent's Ed25519 signing key is stored in an encrypted keystore file (`keystore.enc`):

- **KDF**: Argon2id (64 MiB memory, 3 iterations, 1 thread)
- **Cipher**: ChaCha20-Poly1305 (AEAD)
- **Salt**: 16 random bytes per keystore
- **Nonce**: 12 random bytes per keystore
- **File permissions**: `0600` (Unix only)

The keystore file is JSON with the KDF parameters, salt, nonce, and ciphertext. The Algorand address is stored in plaintext for identification without decryption.

## Message encryption

AlgoChat messages are encrypted end-to-end:

1. **Key exchange**: X25519 Diffie-Hellman using PSK-derived keys
2. **Encryption**: ChaCha20-Poly1305 authenticated encryption
3. **Transport**: Ciphertext stored in Algorand transaction note fields
4. **Verification**: Each message is authenticated -- tampering is detected

## Memory safety

- Sensitive data (seeds, keys) is zeroized after use via the `zeroize` crate
- Passwords are never stored -- only used to derive encryption keys
- The signing key is held in memory only while the agent is running

## Plugin sandboxing

WASM plugins run in a sandboxed WebAssembly environment:

- Memory limits enforced per trust tier (32-512 MiB)
- Table growth capped at 64K entries
- No direct filesystem or network access
- Communication only via JSON-RPC tools

## Network security

- Algod/indexer tokens are not logged
- Encryption public keys are only logged in truncated form (first 16 hex chars)
- All hub communication uses HTTP (consider HTTPS for production)

## CI/CD

- GitHub Actions workflows use least-privilege permissions (`contents: read`)
- Dependencies are audited with `cargo audit`
- CodeQL scanning for security vulnerabilities
