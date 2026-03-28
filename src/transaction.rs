//! Algorand transaction construction, signing, and submission.
//!
//! Builds minimal payment transactions with note fields for AlgoChat messages.
//! Uses canonical msgpack encoding as required by the Algorand protocol.

use algochat::{AlgoChatError, AlgodClient, SuggestedParams};
use ed25519_dalek::{Signer, SigningKey};
use serde::Serialize;
use tracing::debug;

/// A raw Algorand payment transaction (fields in canonical msgpack key order).
///
/// Field names use Algorand's short-form keys. Fields are ordered alphabetically
/// by their serialized key name to produce canonical msgpack encoding.
#[derive(Serialize)]
struct PaymentTxn<'a> {
    /// Amount in microAlgos.
    #[serde(rename = "amt")]
    amount: u64,
    /// Transaction fee in microAlgos.
    #[serde(rename = "fee")]
    fee: u64,
    /// First valid round.
    #[serde(rename = "fv")]
    first_valid: u64,
    /// Genesis ID string.
    #[serde(rename = "gen")]
    genesis_id: &'a str,
    /// Genesis hash (32 bytes).
    #[serde(rename = "gh", with = "serde_bytes")]
    genesis_hash: &'a [u8; 32],
    /// Last valid round.
    #[serde(rename = "lv")]
    last_valid: u64,
    /// Note field (encrypted message bytes).
    #[serde(rename = "note", with = "serde_bytes")]
    note: &'a [u8],
    /// Receiver address (32-byte Ed25519 public key).
    #[serde(rename = "rcv", with = "serde_bytes")]
    receiver: &'a [u8; 32],
    /// Sender address (32-byte Ed25519 public key).
    #[serde(rename = "snd", with = "serde_bytes")]
    sender: &'a [u8; 32],
    /// Transaction type (always "pay").
    #[serde(rename = "type")]
    txn_type: &'a str,
}

/// A signed Algorand transaction (canonical msgpack key order).
#[derive(Serialize)]
struct SignedTxn<'a> {
    /// Ed25519 signature (64 bytes).
    #[serde(with = "serde_bytes")]
    sig: &'a [u8],
    /// The unsigned transaction.
    txn: PaymentTxn<'a>,
}

/// Decodes an Algorand address to its 32-byte Ed25519 public key.
///
/// Algorand addresses are base32-encoded (no padding):
/// - 32 bytes: Ed25519 public key
/// - 4 bytes: checksum (last 4 bytes of SHA-512/256 of public key)
pub fn decode_address(address: &str) -> algochat::Result<[u8; 32]> {
    let decoded = data_encoding::BASE32_NOPAD
        .decode(address.as_bytes())
        .map_err(|e| AlgoChatError::EncodingError(format!("Invalid address encoding: {}", e)))?;

    if decoded.len() != 36 {
        return Err(AlgoChatError::EncodingError(format!(
            "Address decoded to {} bytes, expected 36",
            decoded.len()
        )));
    }

    let public_key = &decoded[..32];
    let checksum = &decoded[32..36];

    // Verify checksum
    use sha2::Digest;
    let hash = sha2::Sha512_256::digest(public_key);
    if checksum != &hash[hash.len() - 4..] {
        return Err(AlgoChatError::EncodingError(
            "Address checksum mismatch".to_string(),
        ));
    }

    let mut result = [0u8; 32];
    result.copy_from_slice(public_key);
    Ok(result)
}

/// Builds, signs, and submits an Algorand payment transaction with an encrypted note.
///
/// This is the core send path: encrypt a message → build a payment txn with the
/// ciphertext as the note → sign with Ed25519 → submit to algod.
pub async fn send_note_transaction(
    algod: &dyn AlgodClient,
    signing_key: &SigningKey,
    sender_address: &str,
    recipient_address: &str,
    note: &[u8],
) -> algochat::Result<String> {
    // Decode addresses to 32-byte public keys
    let sender_pk = decode_address(sender_address)?;
    let receiver_pk = decode_address(recipient_address)?;

    // Get suggested params from the network
    let params: SuggestedParams = algod.get_suggested_params().await?;

    // Build the unsigned transaction
    let txn = PaymentTxn {
        amount: 0, // AlgoChat messages are 0-value payments
        fee: params.min_fee.max(1000), // At least 1000 microAlgos
        first_valid: params.first_valid,
        genesis_id: &params.genesis_id,
        genesis_hash: &params.genesis_hash,
        last_valid: params.last_valid,
        note,
        receiver: &receiver_pk,
        sender: &sender_pk,
        txn_type: "pay",
    };

    // Encode the transaction to msgpack
    let txn_bytes =
        rmp_serde::to_vec_named(&txn).map_err(|e| AlgoChatError::EncodingError(e.to_string()))?;

    // Sign: Ed25519(SHA-512/256("TX" + msgpack(txn))) — but Algorand signs the raw prefixed bytes
    let mut sign_data = Vec::with_capacity(2 + txn_bytes.len());
    sign_data.extend_from_slice(b"TX");
    sign_data.extend_from_slice(&txn_bytes);

    let signature = signing_key.sign(&sign_data);

    debug!(
        recipient = %recipient_address,
        note_len = note.len(),
        fee = txn.fee,
        "submitting AlgoChat transaction"
    );

    // Build the signed transaction
    let signed = SignedTxn {
        sig: &signature.to_bytes(),
        txn,
    };

    let signed_bytes = rmp_serde::to_vec_named(&signed)
        .map_err(|e| AlgoChatError::EncodingError(e.to_string()))?;

    // Submit to algod
    let txid = algod.submit_transaction(&signed_bytes).await?;

    debug!(txid = %txid, "transaction submitted");

    Ok(txid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_address_roundtrip() {
        // Build a valid address from a known public key
        let pubkey = [42u8; 32];
        use sha2::Digest;
        let hash = sha2::Sha512_256::digest(pubkey);
        let checksum = &hash[hash.len() - 4..];
        let mut full = Vec::with_capacity(36);
        full.extend_from_slice(&pubkey);
        full.extend_from_slice(checksum);
        let address = data_encoding::BASE32_NOPAD.encode(&full);

        let decoded = decode_address(&address).unwrap();
        assert_eq!(decoded, pubkey);
    }

    #[test]
    fn test_decode_address_bad_checksum() {
        let pubkey = [1u8; 32];
        let bad_checksum = [0xFF; 4];
        let mut full = Vec::with_capacity(36);
        full.extend_from_slice(&pubkey);
        full.extend_from_slice(&bad_checksum);
        let address = data_encoding::BASE32_NOPAD.encode(&full);

        assert!(decode_address(&address).is_err());
    }

    #[test]
    fn test_decode_address_too_short() {
        assert!(decode_address("AAAA").is_err());
    }

    #[test]
    fn test_payment_txn_serializes_to_msgpack() {
        let gh = [0u8; 32];
        let sender = [1u8; 32];
        let receiver = [2u8; 32];
        let note = b"hello";

        let txn = PaymentTxn {
            amount: 0,
            fee: 1000,
            first_valid: 100,
            genesis_id: "testnet-v1.0",
            genesis_hash: &gh,
            last_valid: 1100,
            note: note.as_slice(),
            receiver: &receiver,
            sender: &sender,
            txn_type: "pay",
        };

        let bytes = rmp_serde::to_vec_named(&txn).unwrap();
        assert!(!bytes.is_empty());

        // Verify it's valid msgpack by deserializing as generic value
        let value: rmpv::Value = rmp_serde::from_slice(&bytes).unwrap();
        assert!(value.is_map());
    }
}
