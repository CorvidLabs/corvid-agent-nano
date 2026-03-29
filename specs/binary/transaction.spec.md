---
module: transaction
version: 1
status: active
files:
  - src/transaction.rs
depends_on:
  - CorvidLabs/rs-algochat@algochat
---

# Transaction Building

## Purpose

Builds, signs, and submits minimal Algorand payment transactions (0 ALGO with a note field) for sending encrypted AlgoChat messages on-chain. Handles the full lifecycle: address encoding/decoding, msgpack transaction construction with canonical field ordering, Ed25519 signing with the "TX" prefix, and submission via the algod REST API.

## Public API

### Exported Functions

| Function | Parameters | Returns | Description |
|----------|-----------|---------|-------------|
| `send_note_transaction` | `algod: &impl AlgodClient`, `sender_address: &str`, `receiver_address: &str`, `note: &[u8]`, `signing_key: &SigningKey` | `anyhow::Result<String>` | Build, sign, and submit a 0-ALGO payment transaction with an encrypted note. Returns the transaction ID |
| `decode_address` | `address: &str` | `anyhow::Result<[u8; 32]>` | Decode an Algorand address (base32 + 4-byte checksum) to 32 raw public key bytes |

### Internal Functions

| Function | Description |
|----------|-------------|
| `build_payment_transaction` | Build a msgpack-encoded Algorand payment transaction with canonical field ordering |
| `sign_transaction` | Sign a raw transaction with Ed25519 ("TX" prefix + transaction bytes) |

### Transaction Format

The payment transaction is a msgpack map with 9 fields in strict alphabetical order:

| Field | Type | Description |
|-------|------|-------------|
| `fee` | u64 | Minimum fee from suggested params |
| `fv` | u64 | First valid round |
| `gen` | string | Genesis ID (e.g., "testnet-v1.0") |
| `gh` | bytes (32) | Genesis hash |
| `lv` | u64 | Last valid round |
| `note` | bytes | Encrypted message payload |
| `rcv` | bytes (32) | Receiver public key |
| `snd` | bytes (32) | Sender public key |
| `type` | string | "pay" |

The `amt` field is omitted (0-ALGO transaction).

### Signed Transaction Envelope

| Field | Type | Description |
|-------|------|-------------|
| `sig` | bytes (64) | Ed25519 signature over "TX" + raw transaction bytes |
| `txn` | map | The raw transaction map (embedded directly) |

### Address Format

Algorand addresses are 58-character base32 strings encoding 36 bytes:
- First 32 bytes: Ed25519 public key
- Last 4 bytes: SHA-512/256 checksum (last 4 bytes of hash of public key)

## Invariants

1. Transaction fields are always written in strict alphabetical order (Algorand canonical encoding)
2. The "TX" prefix is prepended before signing (Algorand signing convention)
3. Signatures are Ed25519 over the full "TX" + msgpack bytes (not a hash)
4. Address checksums are verified on decode — invalid checksums are rejected
5. Transactions are always 0-ALGO payments (no amount transferred, only the note field carries data)
6. The signed transaction embeds the raw transaction bytes directly (not re-encoded)

## Behavioral Examples

### Scenario: Send an encrypted reply on-chain

- **Given** a valid algod client, sender/receiver addresses, encrypted note bytes, and a signing key
- **When** `send_note_transaction` is called
- **Then** it fetches suggested params, builds a 0-ALGO payment txn with the note, signs it, submits it, and returns the transaction ID

### Scenario: Invalid Algorand address

- **Given** an address with corrupted base32 or wrong checksum
- **When** `decode_address` is called
- **Then** it returns an error describing the specific failure (invalid base32, wrong length, or checksum mismatch)

### Scenario: Address roundtrip

- **Given** a 32-byte public key
- **When** encoded with `encode_address` and decoded with `decode_address`
- **Then** the original 32 bytes are recovered exactly

## Error Cases

| Condition | Behavior |
|-----------|----------|
| Invalid base32 in address | `decode_address` returns error with address and decode details |
| Wrong address length | Returns error specifying expected vs actual byte count |
| Checksum mismatch | Returns error identifying the address with bad checksum |
| algod unreachable | `send_note_transaction` returns error from `get_suggested_params` |
| Transaction rejected by algod | Returns error from `submit_transaction` |
| Msgpack encoding failure | Returns error (should not occur with valid inputs) |

## Dependencies

### Consumes

| Module | What is used |
|--------|-------------|
| `algochat` | `AlgodClient` trait, `SuggestedParams` struct |
| `ed25519-dalek` | `SigningKey`, `Signer` for Ed25519 signatures |
| `sha2` | `Sha512_256` for address checksums |
| `rmp` | Low-level msgpack encoding (map, string, uint, binary) |
| `data-encoding` | `BASE32_NOPAD` for Algorand address encoding/decoding |

### Consumed By

| Module | What is used |
|--------|-------------|
| `src/agent.rs` | `send_note_transaction` for sending encrypted replies on-chain |

## Change Log

| Date | Author | Change |
|------|--------|--------|
| 2026-03-28 | CorvidAgent | Initial spec — transaction building, signing, and address encoding |
