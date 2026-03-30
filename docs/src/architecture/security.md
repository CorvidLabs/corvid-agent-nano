# Security Model

## Overview

corvid-agent-nano implements defense-in-depth across wallet storage, message encryption, runtime isolation, and plugin sandboxing. This document details the security assumptions, threat model, and protections.

## Threat Model

### What we protect against:

- **Disk compromise** — Attacker gains access to files but not memory
- **Man-in-the-middle (MITM)** — Attacker intercepts network traffic
- **Malicious plugins** — Untrusted WASM code running on the agent
- **Accidental exposure** — Inadvertent logging of secrets
- **Blockchain tampering** — Adversary modifies transaction data on-chain

### What we assume:

- **Trusted initial setup** — You control your wallet at creation time
- **Secure password** — Your keystore password is reasonably strong (8+ chars)
- **Safe running environment** — The machine running the agent is not compromised
- **HTTPS for production** — Hub communication should use HTTPS in production (not just HTTP)

## Wallet Encryption

Your Ed25519 signing key (the seed) is stored in an encrypted keystore file (`keystore.enc`):

### Encryption scheme

```
Plaintext (seed)
    ↓
[Argon2id KDF] → Derived key (256 bits)
    ↓
[ChaCha20-Poly1305 cipher] → Ciphertext + tag
    ↓
Keystore file (JSON)
```

### Parameters

- **KDF algorithm**: Argon2id
  - Memory cost: 64 MiB
  - Time cost: 3 iterations
  - Parallelism: 1 thread
  - Purpose: Maximize resistance to GPU/ASIC brute-force attacks
- **Cipher**: ChaCha20-Poly1305 (AEAD)
  - Authenticated encryption — tampering detected
  - Stream cipher — efficient and constant-time
- **Salt**: 16 random bytes (per keystore)
  - Prevents rainbow-table attacks
- **Nonce**: 12 random bytes (per keystore)
  - Ensures unique ciphertext for same plaintext/key
- **File permissions**: `0600` (Unix only)
  - Readable/writable by owner only

### Keystore format

The keystore is JSON:

```json
{
  "argon2id": {
    "memory_cost": 65536,
    "time_cost": 3,
    "parallelism": 1,
    "salt": "hex-encoded-salt"
  },
  "nonce": "hex-encoded-nonce",
  "ciphertext": "hex-encoded-ciphertext",
  "address": "XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX"
}
```

The address is stored in plaintext to allow identification without decryption (e.g., for backup/recovery).

### Protection analysis

- **Offline attacks**: Argon2id's high memory cost makes GPU/ASIC attacks impractical
- **Online attacks**: Each failed password attempt requires 64 MiB of memory
- **Side channels**: ChaCha20-Poly1305 is constant-time (no timing leaks)

### Recovery

If your keystore password is lost, you can recover from your 25-word recovery phrase:

```bash
rm ./data/keystore.enc
can import --mnemonic "word1 word2 ... word25" --password "new_password"
```

**Keep your recovery phrase safe** — it's the only way to restore your wallet.

## Message Encryption

### AlgoChat protocol

Messages between agents are encrypted end-to-end using a pre-shared key (PSK) model:

```
Alice               Algorand Blockchain      Bob
  │                         │                 │
  ├─ Encrypt(message)       │                 │
  │  with PSK                │                 │
  ├─ Send transaction───────→│                 │
  │  (ciphertext in note)     │                 │
  │                           │───→ Read tx    │
  │                           │─ Decrypt      │
  │                           │  with PSK     │
  │                           ✓ Message      │
```

### Encryption scheme

- **Ephemeral key exchange**: X25519 Diffie-Hellman (one key per message)
  - Provides forward secrecy — compromising one key doesn't reveal past messages
  - Uses the shared PSK as input to derive session keys
- **Authenticated encryption**: ChaCha20-Poly1305
  - Both confidentiality and authentication in one operation
  - Tampering is cryptographically detected (Poly1305 authentication tag)
- **Transport**: Ciphertext stored as Algorand transaction note field
  - 1 KB size limit per note — messages are chunked if necessary
  - Immutable on-chain — provides a permanent audit trail

### Forward secrecy

Each message uses a unique ephemeral key. This means:
- Even if an attacker obtains your PSK, past messages remain encrypted
- Each message can be decrypted independently

### Pre-shared key (PSK) management

PSKs are established out-of-band (not over the network):

```bash
# Agent A generates or provides a PSK:
can groups create --name trusted-group

# Agent B adds Agent A as a contact with that PSK:
can contacts add --name agent-a --address <ADDRESS> --psk <PSK>
```

**Best practices for PSKs:**
- Generate with cryptographically strong RNG (not by hand!)
- Share securely (encrypted email, secure messaging, in-person)
- Use unique PSKs for each contact/group
- Rotate PSKs periodically if long-lived relationships

## Memory Safety

Sensitive data is carefully managed in memory:

### Zeroization

- Seeds, keys, and passwords are zeroized after use via the `zeroize` crate
- Prevents sensitive data from leaking in RAM dumps or core files
- Applied to:
  - Ed25519 signing keys
  - X25519 ephemeral keys
  - Derived encryption keys
  - Plaintext seeds during import

### Runtime constraints

- **Passwords**: Never stored in memory — only used once during KDF
- **Keys**: Held in memory only while needed
  - Signing key: loaded only when signing transactions
  - Decryption key: loaded only when decrypting messages
  - Session keys: zeroized after message processing
- **Recovery phrases**: Immediately zeroized after import

## Plugin Sandboxing

WASM plugins run in a sandboxed WebAssembly environment with strict isolation:

### Sandbox isolation

- **Memory sandbox**: Each plugin has isolated linear memory (32-512 MiB depending on tier)
  - No direct access to agent memory
  - No access to other plugins' memory
- **Table sandbox**: Function table capped at 64K entries
  - Prevents unbounded table growth
- **Capability model**: Plugins access only JSON-RPC tools they're granted
  - No direct filesystem access
  - No direct network access
  - No access to runtime internals

### Trust tiers

Plugins are classified by trustworthiness:

| Tier | Memory | Timeout | CPU limit | Use case |
|------|--------|---------|-----------|----------|
| `trusted` | 512 MiB | 60s | Unlimited | First-party, audited, critical functionality |
| `verified` | 128 MiB | 30s | 100% | Third-party code-reviewed plugins |
| `untrusted` | 32 MiB | 10s | 50% | Unknown/unreviewed plugins (default) |

**CPU limits** prevent denial-of-service attacks (e.g., infinite loops freezing the agent).

### Available capabilities

Plugins can request access to:

1. **Messaging** — Send/receive encrypted messages
2. **Storage** — Key-value store (plugin-isolated)
3. **Algorand** — Query chain state, build transactions
4. **HTTP** — Make outbound HTTP requests (with URL allowlist)

Plugins **cannot**:
- Access the keystore or encryption keys
- Read other plugins' storage
- Execute arbitrary native code
- Access the filesystem
- Open raw network sockets

### Resource limits

Per plugin:
- Memory: Enforced by WebAssembly linear memory limit
- CPU: Execution timeout per tool invocation
- Table growth: Capped at 64K function entries
- Storage: Per-plugin isolation (no shared state leakage)

### Attestation and logging

When a plugin is loaded with a tier, it's logged (with plugin ID and hash):

```
2026-03-30 10:15:42 Loaded plugin hello-world (trusted) hash=sha256:abc123...
```

This audit trail helps track which plugins are installed.

## Network Security

### Logging practices

- **API tokens**: Algod/indexer auth tokens are never logged
- **Encryption keys**: Public keys logged in truncated form only (first 16 hex chars)
  - Example: `pub_key=5a1b2c3d4e5f6a7b...` (not the full key)
- **Passwords**: Never logged (not even as `****`)
- **Seeds**: Never logged

### Hub communication

- **Default**: HTTP (cleartext)
- **Recommended for production**: HTTPS (encrypted)
- **Authentication**: Pre-shared key (PSK) contact ensures both sides are known

If using testnet or mainnet, always use HTTPS for hub communication to prevent MITM attacks.

### Algorand node security

- Consider running your own Algorand node for privacy
- If using public nodes, your queries reveal which addresses/transactions you're interested in
- Public nodes should be accessed over HTTPS

## Cryptographic Primitives

### Algorithms used

| Algorithm | Purpose | Why this choice |
|-----------|---------|-----------------|
| **Ed25519** | Wallet signing | Fast, secure, no recovery parameter issues |
| **X25519** | Key exchange | Post-quantum resistant forward secrecy |
| **ChaCha20-Poly1305** | Message encryption | Fast, constant-time, AEAD (authenticated encryption) |
| **Argon2id** | Password hashing | Memory-hard, GPU/ASIC resistant |
| **SHA-256** | Hashing (if needed) | Standard, widely audited |

All algorithms are well-established and battle-tested. The combinations form a conservative, time-tested security architecture.

### NIST guidance

- Ed25519: NIST approved (RFC 8032, FIPS 186-5)
- ChaCha20-Poly1305: NIST approved (RFC 7539, FIPS 800-38D)
- Argon2id: OWASP recommended for password hashing (2023)

## Auditing and Vulnerabilities

### CI/CD security

- **GitHub Actions**: Least-privilege permissions (`contents: read`)
- **Dependency scanning**: `cargo audit` checks for known vulnerabilities
- **Static analysis**: CodeQL scans for common security issues
- **Spec validation**: `specsync` ensures code matches security specifications

### Reporting vulnerabilities

If you discover a security issue:

1. **Do not** open a public GitHub issue
2. **Email** security@corvidlabs.io with:
   - Vulnerability description
   - Affected version(s)
   - Reproduction steps (if possible)
   - Your suggested fix (if any)

We will:
- Acknowledge within 48 hours
- Assess severity and impact
- Develop a fix
- Release a patched version
- Credit you in the security advisory

## Best Practices for Users

1. **Keep your recovery phrase safe** — Store offline, not in files/email
2. **Use a strong password** — 12+ characters, mix of letters/numbers/symbols
3. **Use HTTPS for hub communication** — Especially on testnet/mainnet
4. **Audit plugins before installing** — Review the WASM source or trust the author
5. **Run your own Algorand node** — For privacy and reliability
6. **Rotate PSKs periodically** — For long-lived agent relationships
7. **Keep Rust updated** — Security patches are released regularly

## Security Limitations

- **Hot wallet** — Your signing key is in memory while the agent runs
  - For high-security use cases, consider air-gapped signing
- **HTTP by default** — Hub communication is unencrypted by default
  - Use HTTPS in production
- **Trust on first use (TOFU)** — PSKs establish trust but can be compromised if shared insecurely
- **Local to host** — Agent doesn't protect against host compromise

These are acceptable trade-offs for a lightweight, user-friendly agent.
