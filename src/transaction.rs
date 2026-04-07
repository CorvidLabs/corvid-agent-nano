//! Algorand transaction building and signing.
//!
//! Builds minimal payment transactions (0 ALGO with a note field)
//! for sending encrypted AlgoChat messages on-chain.

use algochat::{AlgodClient, SuggestedParams};
use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha512_256};

/// Build, sign, and submit a 0-ALGO payment transaction with an encrypted note.
///
/// This is the primary interface for sending AlgoChat messages on-chain.
/// Returns the transaction ID on success.
pub async fn send_note_transaction(
    algod: &impl AlgodClient,
    sender_address: &str,
    receiver_address: &str,
    note: &[u8],
    signing_key: &SigningKey,
) -> anyhow::Result<String> {
    let params = algod
        .get_suggested_params()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to get suggested params: {}", e))?;

    let raw_txn = build_payment_transaction(sender_address, receiver_address, note, &params)?;
    let signed = sign_transaction(&raw_txn, signing_key)?;

    let txid = algod
        .submit_transaction(&signed)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to submit transaction: {}", e))?;

    Ok(txid)
}

/// Build a msgpack-encoded Algorand payment transaction with an amount.
///
/// Creates a payment transaction for the given amount (in microAlgos).
/// Fields are written in strict alphabetical order as required by Algorand.
/// If amount is 0, the `amt` field is omitted.
pub fn build_payment_transaction_with_amount(
    sender: &str,
    receiver: &str,
    amount: u64,
    params: &SuggestedParams,
) -> anyhow::Result<Vec<u8>> {
    let sender_bytes = decode_address(sender)?;
    let receiver_bytes = decode_address(receiver)?;

    let mut buf = Vec::with_capacity(256);

    // Fields in alphabetical order: amt, fee, fv, gen, gh, lv, rcv, snd, type
    let field_count: u32 = if amount > 0 { 9 } else { 8 };
    rmp::encode::write_map_len(&mut buf, field_count)
        .map_err(|e| anyhow::anyhow!("msgpack encode error: {}", e))?;

    if amount > 0 {
        write_str(&mut buf, "amt")?;
        write_uint(&mut buf, amount)?;
    }

    write_str(&mut buf, "fee")?;
    write_uint(&mut buf, params.min_fee)?;

    write_str(&mut buf, "fv")?;
    write_uint(&mut buf, params.first_valid)?;

    write_str(&mut buf, "gen")?;
    write_str(&mut buf, &params.genesis_id)?;

    write_str(&mut buf, "gh")?;
    write_bin(&mut buf, &params.genesis_hash)?;

    write_str(&mut buf, "lv")?;
    write_uint(&mut buf, params.last_valid)?;

    write_str(&mut buf, "rcv")?;
    write_bin(&mut buf, &receiver_bytes)?;

    write_str(&mut buf, "snd")?;
    write_bin(&mut buf, &sender_bytes)?;

    write_str(&mut buf, "type")?;
    write_str(&mut buf, "pay")?;

    Ok(buf)
}

/// Build a msgpack-encoded Algorand payment transaction.
///
/// Creates a 0-ALGO payment transaction with the given note field.
/// Fields are written in strict alphabetical order as required by Algorand.
fn build_payment_transaction(
    sender: &str,
    receiver: &str,
    note: &[u8],
    params: &SuggestedParams,
) -> anyhow::Result<Vec<u8>> {
    let sender_bytes = decode_address(sender)?;
    let receiver_bytes = decode_address(receiver)?;

    // Algorand transactions are msgpack maps with alphabetically sorted string keys.
    // For a 0-ALGO payment with note, the fields (in order) are:
    //   fee, fv, gen, gh, lv, note, rcv, snd, type
    // We omit `amt` because it is 0.
    let mut buf = Vec::with_capacity(256);

    // Map header: 9 fields
    rmp::encode::write_map_len(&mut buf, 9)
        .map_err(|e| anyhow::anyhow!("msgpack encode error: {}", e))?;

    // fee (u64)
    write_str(&mut buf, "fee")?;
    write_uint(&mut buf, params.min_fee)?;

    // fv (first valid round)
    write_str(&mut buf, "fv")?;
    write_uint(&mut buf, params.first_valid)?;

    // gen (genesis ID)
    write_str(&mut buf, "gen")?;
    write_str(&mut buf, &params.genesis_id)?;

    // gh (genesis hash, 32 bytes)
    write_str(&mut buf, "gh")?;
    write_bin(&mut buf, &params.genesis_hash)?;

    // lv (last valid round)
    write_str(&mut buf, "lv")?;
    write_uint(&mut buf, params.last_valid)?;

    // note (encrypted message bytes)
    write_str(&mut buf, "note")?;
    write_bin(&mut buf, note)?;

    // rcv (receiver public key, 32 bytes)
    write_str(&mut buf, "rcv")?;
    write_bin(&mut buf, &receiver_bytes)?;

    // snd (sender public key, 32 bytes)
    write_str(&mut buf, "snd")?;
    write_bin(&mut buf, &sender_bytes)?;

    // type
    write_str(&mut buf, "type")?;
    write_str(&mut buf, "pay")?;

    Ok(buf)
}

/// Sign a raw transaction with an Ed25519 key.
///
/// Prepends "TX" to the raw transaction bytes, signs the result,
/// and wraps in a signed transaction envelope.
fn sign_transaction(raw_txn: &[u8], signing_key: &SigningKey) -> anyhow::Result<Vec<u8>> {
    // Algorand signs "TX" prefix + raw transaction bytes directly
    let mut to_sign = Vec::with_capacity(2 + raw_txn.len());
    to_sign.extend_from_slice(b"TX");
    to_sign.extend_from_slice(raw_txn);

    let signature = signing_key.sign(&to_sign);
    let sig_bytes = signature.to_bytes();

    // Build signed transaction envelope: {"sig": <64 bytes>, "txn": <transaction map>}
    let mut buf = Vec::with_capacity(sig_bytes.len() + raw_txn.len() + 16);

    // Map header: 2 fields (sig, txn)
    rmp::encode::write_map_len(&mut buf, 2)
        .map_err(|e| anyhow::anyhow!("msgpack encode error: {}", e))?;

    // sig (64-byte Ed25519 signature)
    write_str(&mut buf, "sig")?;
    write_bin(&mut buf, &sig_bytes)?;

    // txn (embed the raw transaction map directly — it's already valid msgpack)
    write_str(&mut buf, "txn")?;
    buf.extend_from_slice(raw_txn);

    Ok(buf)
}

// ---- msgpack helpers ----

fn write_str(buf: &mut Vec<u8>, s: &str) -> anyhow::Result<()> {
    rmp::encode::write_str(buf, s).map_err(|e| anyhow::anyhow!("msgpack encode error: {}", e))
}

fn write_uint(buf: &mut Vec<u8>, v: u64) -> anyhow::Result<()> {
    rmp::encode::write_uint(buf, v).map_err(|e| anyhow::anyhow!("msgpack encode error: {}", e))?;
    Ok(())
}

fn write_bin(buf: &mut Vec<u8>, data: &[u8]) -> anyhow::Result<()> {
    rmp::encode::write_bin(buf, data).map_err(|e| anyhow::anyhow!("msgpack encode error: {}", e))
}

/// Decode an Algorand address (base32 with 4-byte checksum) to 32 raw bytes.
pub fn decode_address(address: &str) -> anyhow::Result<[u8; 32]> {
    use data_encoding::BASE32_NOPAD;

    let decoded = BASE32_NOPAD
        .decode(address.as_bytes())
        .map_err(|e| anyhow::anyhow!("Invalid Algorand address '{}': {}", address, e))?;

    if decoded.len() != 36 {
        anyhow::bail!(
            "Invalid Algorand address length: expected 36 bytes (32 + 4 checksum), got {}",
            decoded.len()
        );
    }

    let mut public_key = [0u8; 32];
    public_key.copy_from_slice(&decoded[..32]);

    // Verify checksum: last 4 bytes of SHA-512/256(public_key)
    let hash = Sha512_256::digest(public_key);
    let expected_checksum = &hash[hash.len() - 4..];
    let actual_checksum = &decoded[32..];

    if expected_checksum != actual_checksum {
        anyhow::bail!("Algorand address checksum mismatch for '{}'", address);
    }

    Ok(public_key)
}

/// Encode 32 raw bytes as an Algorand address (base32 with checksum).
#[cfg(test)]
fn encode_address(public_key: &[u8; 32]) -> String {
    use data_encoding::BASE32_NOPAD;

    let hash = Sha512_256::digest(public_key);
    let checksum = &hash[hash.len() - 4..];

    let mut addr_bytes = Vec::with_capacity(36);
    addr_bytes.extend_from_slice(public_key);
    addr_bytes.extend_from_slice(checksum);

    BASE32_NOPAD.encode(&addr_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_and_encode_address_roundtrip() {
        let public_key = [0u8; 32];
        let address = encode_address(&public_key);
        let decoded = decode_address(&address).unwrap();
        assert_eq!(decoded, public_key);
    }

    #[test]
    fn decode_address_invalid_base32() {
        let result = decode_address("not-valid-base32!!!");
        assert!(result.is_err());
    }

    #[test]
    fn decode_address_wrong_length() {
        use data_encoding::BASE32_NOPAD;
        let short = BASE32_NOPAD.encode(&[0u8; 16]);
        let result = decode_address(&short);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("length"));
    }

    #[test]
    fn decode_address_bad_checksum() {
        use data_encoding::BASE32_NOPAD;
        let mut bytes = vec![0u8; 36];
        bytes[32] = 0xFF; // corrupt checksum
        let address = BASE32_NOPAD.encode(&bytes);
        let result = decode_address(&address);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("checksum"));
    }

    #[test]
    fn encode_address_deterministic() {
        let key = [42u8; 32];
        let a1 = encode_address(&key);
        let a2 = encode_address(&key);
        assert_eq!(a1, a2);
        assert_eq!(a1.len(), 58); // Standard Algorand address length
    }

    #[test]
    fn build_payment_transaction_produces_valid_msgpack() {
        let sender = encode_address(&[1u8; 32]);
        let receiver = encode_address(&[2u8; 32]);
        let note = b"test note";
        let params = SuggestedParams {
            fee: 0,
            min_fee: 1000,
            first_valid: 100,
            last_valid: 1100,
            genesis_id: "testnet-v1.0".to_string(),
            genesis_hash: [3u8; 32],
        };

        let raw = build_payment_transaction(&sender, &receiver, note, &params).unwrap();
        assert!(!raw.is_empty());

        // Verify it starts with a msgpack map header
        // fixmap with 9 elements: 0x80 | 9 = 0x89
        assert_eq!(raw[0], 0x89);
    }

    #[test]
    fn sign_transaction_produces_valid_envelope() {
        let signing_key = SigningKey::from_bytes(&[1u8; 32]);
        let sender = encode_address(&[1u8; 32]);
        let receiver = encode_address(&[2u8; 32]);
        let params = SuggestedParams {
            fee: 0,
            min_fee: 1000,
            first_valid: 100,
            last_valid: 1100,
            genesis_id: "testnet-v1.0".to_string(),
            genesis_hash: [3u8; 32],
        };

        let raw = build_payment_transaction(&sender, &receiver, b"hello", &params).unwrap();
        let signed = sign_transaction(&raw, &signing_key).unwrap();

        // Should start with a msgpack map with 2 fields: 0x82
        assert_eq!(signed[0], 0x82);
        assert!(signed.len() > 64); // Must contain at least the 64-byte signature
    }

    #[test]
    fn sign_transaction_deterministic() {
        let signing_key = SigningKey::from_bytes(&[1u8; 32]);
        let raw = build_payment_transaction(
            &encode_address(&[1u8; 32]),
            &encode_address(&[2u8; 32]),
            b"test",
            &SuggestedParams {
                fee: 0,
                min_fee: 1000,
                first_valid: 100,
                last_valid: 1100,
                genesis_id: "test".to_string(),
                genesis_hash: [0u8; 32],
            },
        )
        .unwrap();

        let s1 = sign_transaction(&raw, &signing_key).unwrap();
        let s2 = sign_transaction(&raw, &signing_key).unwrap();
        assert_eq!(s1, s2);
    }
}
