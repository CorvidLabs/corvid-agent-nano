---
module: transaction
version: 1
status: active
files:
  - src/transaction.rs
depends_on:
  - external: algochat (git: https://github.com/CorvidLabs/rs-algochat)
---

# Transaction

## Purpose

Algorand transaction construction, signing, and submission. Builds minimal 0-value payment transactions with encrypted AlgoChat messages in the note field. Uses canonical msgpack encoding as required by the Algorand protocol.

## Public API

| Function | Signature | Description |
|----------|-----------|-------------|
| `decode_address` | `(address: &str) -> algochat::Result<[u8; 32]>` | Decode Algorand address to 32-byte Ed25519 public key with checksum verification |
| `send_note_transaction` | `(algod, signing_key, sender, recipient, note) -> algochat::Result<String>` | Build, sign, and submit a payment txn with note; returns txid |

### Internal Structs

| Struct | Description |
|--------|-------------|
| `PaymentTxn` | Unsigned Algorand payment transaction (msgpack-serializable, canonical field order) |
| `SignedTxn` | Signed transaction wrapper (signature + unsigned txn) |

### PaymentTxn Fields (Algorand short-form keys)

| Field | Serde Name | Type | Description |
|-------|------------|------|-------------|
| `amount` | `amt` | `u64` | microAlgos (always 0 for AlgoChat) |
| `fee` | `fee` | `u64` | Transaction fee (min 1000 microAlgos) |
| `first_valid` | `fv` | `u64` | First valid round |
| `genesis_id` | `gen` | `&str` | Network genesis ID |
| `genesis_hash` | `gh` | `&[u8; 32]` | Network genesis hash |
| `last_valid` | `lv` | `u64` | Last valid round |
| `note` | `note` | `&[u8]` | Encrypted message bytes |
| `receiver` | `rcv` | `&[u8; 32]` | Recipient public key |
| `sender` | `snd` | `&[u8; 32]` | Sender public key |
| `txn_type` | `type` | `&str` | Always `"pay"` |

## Invariants

1. AlgoChat messages are always 0-value payment transactions (`amount = 0`)
2. Fee is `max(suggested_min_fee, 1000)` microAlgos
3. Signing: `Ed25519.sign("TX" || msgpack(txn))` — the "TX" prefix is Algorand protocol
4. Address decoding verifies the 4-byte checksum (last 4 bytes of SHA-512/256 of pubkey)
5. Decoded addresses must be exactly 36 bytes (32 pubkey + 4 checksum)
6. Transaction bytes are msgpack-encoded with named fields via `rmp_serde::to_vec_named`

## Behavioral Examples

### Scenario: Send encrypted message on-chain

- **Given** a signing key, sender address, recipient address, and encrypted note bytes
- **When** `send_note_transaction` is called
- **Then** fetches suggested params from algod, builds payment txn, signs with Ed25519, submits, returns txid

### Scenario: Decode valid address

- **Given** a valid Algorand address (58 chars, base32)
- **When** `decode_address` is called
- **Then** returns the 32-byte Ed25519 public key

### Scenario: Bad checksum

- **Given** an address with corrupted checksum bytes
- **When** `decode_address` is called
- **Then** returns `EncodingError("Address checksum mismatch")`

## Error Cases

| Condition | Behavior |
|-----------|----------|
| Invalid base32 encoding | `EncodingError("Invalid address encoding")` |
| Address wrong length | `EncodingError("Address decoded to N bytes, expected 36")` |
| Checksum mismatch | `EncodingError("Address checksum mismatch")` |
| Algod unreachable | Error from `get_suggested_params` |
| Msgpack encoding failure | `EncodingError` |
| Transaction submission failure | Error from `submit_transaction` |

## Dependencies

### Consumes

| Module | What is used |
|--------|-------------|
| `algochat` | `AlgodClient` trait, `SuggestedParams`, `AlgoChatError` |
| `ed25519-dalek` | `SigningKey`, `Signer` for transaction signing |
| `rmp-serde` | Msgpack serialization |
| `data-encoding` | `BASE32_NOPAD` for address decoding |
| `sha2` | `Sha512_256` for address checksum verification |
| `serde_bytes` | Binary field serialization |

### Consumed By

| Module | What is used |
|--------|-------------|
| `src/agent.rs` | `send_note_transaction` in `send_reply` |

## Change Log

| Date | Author | Change |
|------|--------|--------|
| 2026-03-28 | CorvidAgent | Initial spec |
