---
module: wallet
version: 1
status: active
files:
  - src/wallet.rs
depends_on: []
---

# Wallet

## Purpose

Wallet generation, mnemonic encoding/decoding, and Algorand address derivation. Implements the Algorand mnemonic standard: 25 words using the BIP-39 English wordlist with Algorand-specific 11-bit encoding and SHA-512/256 checksum. Provides deterministic key-to-address derivation using Ed25519.

## Public API

### Exported Functions

| Function | Parameters | Returns | Description |
|----------|-----------|---------|-------------|
| `generate_seed` | — | `[u8; 32]` | Generate a new random 32-byte Ed25519 seed using OS randomness |
| `address_from_seed` | `seed: &[u8; 32]` | `String` | Derive the 58-character Algorand address from a 32-byte Ed25519 seed |
| `encode_address` | `public_key: &[u8; 32]` | `String` | Encode a 32-byte public key as an Algorand address (base32 + 4-byte SHA-512/256 checksum) |
| `decode_address` | `address: &str` | `Result<[u8; 32]>` | Decode an Algorand address to the 32-byte public key, verifying the checksum |
| `seed_to_mnemonic` | `seed: &[u8; 32]` | `String` | Convert a 32-byte seed to a 25-word Algorand mnemonic |
| `mnemonic_to_seed` | `mnemonic: &str` | `Result<[u8; 32]>` | Convert a 25-word Algorand mnemonic back to a 32-byte seed, verifying the checksum word |

### Internal Functions

| Function | Description |
|----------|-------------|
| `words` | Load the BIP-39 English word list (2048 words) from embedded `wordlist.txt` |
| `word_index` | Look up a word's index in the word list |
| `bytes_to_11bit` | Convert bytes to a list of 11-bit values for mnemonic encoding |
| `bits_to_bytes` | Convert 24 eleven-bit values back to 32 bytes for mnemonic decoding |

## Invariants

1. The embedded BIP-39 word list contains exactly 2048 words
2. Generated seeds are always exactly 32 bytes
3. Algorand addresses are always 58 characters (base32-encoded 36 bytes: 32 public key + 4 checksum)
4. Mnemonics are always exactly 25 words: 24 data words + 1 checksum word
5. The checksum word is the first 11 bits of SHA-512/256(seed), mapped to a word list index
6. Mnemonic encoding is deterministic: same seed always produces the same 25-word phrase
7. Address checksum is the last 4 bytes of SHA-512/256(public_key)
8. Invalid mnemonic checksums cause `mnemonic_to_seed` to zeroize the partially-recovered seed before returning an error

## Behavioral Examples

### Scenario: Generate a new wallet

- **Given** a call to `generate_seed`
- **When** the seed is generated
- **Then** it returns exactly 32 random bytes from the OS CSPRNG

### Scenario: Mnemonic roundtrip

- **Given** a 32-byte seed
- **When** converted to mnemonic via `seed_to_mnemonic` and back via `mnemonic_to_seed`
- **Then** the recovered seed equals the original

### Scenario: Address roundtrip

- **Given** a 32-byte Ed25519 public key
- **When** encoded via `encode_address` and decoded via `decode_address`
- **Then** the recovered 32 bytes equal the original public key

### Scenario: Deterministic mnemonic

- **Given** the seed `[42u8; 32]`
- **When** `seed_to_mnemonic` is called twice
- **Then** both calls return the identical 25-word phrase

## Error Cases

| Condition | Behavior |
|-----------|----------|
| Mnemonic word count != 25 | Returns error: "Mnemonic must be 25 words (got N)" |
| Unknown word in mnemonic | Returns error: "Unknown word at position N: \"word\"" |
| Mnemonic checksum mismatch | Zeroizes partial seed, returns error: "Mnemonic checksum mismatch" |
| Address with invalid base32 | Returns error: "Invalid base32: ..." |
| Address wrong byte length | Returns error: "Address must be 36 bytes (got N)" |
| Address checksum mismatch | Returns error: "Address checksum mismatch" |

## Dependencies

### Consumes

| Module | What is used |
|--------|-------------|
| `ed25519-dalek` | `SigningKey`, `OsRng` for Ed25519 key generation and derivation |
| `sha2` | `Sha512_256` for address and mnemonic checksums |
| `data-encoding` | `BASE32_NOPAD` for Algorand address encoding/decoding |
| `zeroize` | Secure memory clearing on checksum failure |
| `rand` | `OsRng` for cryptographic random seed generation |

### Consumed By

| Module | What is used |
|--------|-------------|
| `src/main.rs` | `generate_seed`, `address_from_seed`, `seed_to_mnemonic`, `mnemonic_to_seed` for wallet CLI subcommands |

## Change Log

| Date | Author | Change |
|------|--------|--------|
| 2026-03-28 | CorvidAgent | Initial spec |
