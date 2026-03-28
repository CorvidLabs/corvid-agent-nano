//! Wallet generation, mnemonic encoding/decoding, and address derivation.
//!
//! Implements the Algorand mnemonic standard: 25 words using BIP-39 English wordlist
//! with Algorand-specific 11-bit encoding and SHA-512/256 checksum.

use anyhow::{bail, Result};
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use sha2::{Digest, Sha512_256};
use zeroize::Zeroize;

/// BIP-39 English word list (2048 words), used by Algorand mnemonics.
const WORDLIST: &str = include_str!("wordlist.txt");

/// Get the word list as a vector of &str.
fn words() -> Vec<&'static str> {
    WORDLIST.lines().collect()
}

/// Look up a word's index in the word list.
fn word_index(word: &str) -> Option<usize> {
    words().iter().position(|w| *w == word)
}

/// Generate a new random 32-byte Ed25519 seed.
pub fn generate_seed() -> [u8; 32] {
    let key = SigningKey::generate(&mut OsRng);
    *key.as_bytes()
}

/// Derive the Algorand address from a 32-byte Ed25519 seed.
pub fn address_from_seed(seed: &[u8; 32]) -> String {
    let signing_key = SigningKey::from_bytes(seed);
    let public_key = signing_key.verifying_key();
    encode_address(public_key.as_bytes())
}

/// Encode a 32-byte public key as an Algorand address (base32 + 4-byte checksum).
pub fn encode_address(public_key: &[u8; 32]) -> String {
    let mut hasher = Sha512_256::new();
    hasher.update(public_key);
    let hash = hasher.finalize();
    let checksum = &hash[28..32]; // last 4 bytes

    let mut addr_bytes = [0u8; 36];
    addr_bytes[..32].copy_from_slice(public_key);
    addr_bytes[32..].copy_from_slice(checksum);

    data_encoding::BASE32_NOPAD.encode(&addr_bytes)
}

/// Decode an Algorand address to the 32-byte public key.
pub fn decode_address(address: &str) -> Result<[u8; 32]> {
    let bytes = data_encoding::BASE32_NOPAD
        .decode(address.as_bytes())
        .map_err(|e| anyhow::anyhow!("Invalid base32: {}", e))?;

    if bytes.len() != 36 {
        bail!(
            "Address must be 36 bytes (got {}). Check your address.",
            bytes.len()
        );
    }

    let public_key: [u8; 32] = bytes[..32].try_into().unwrap();
    let checksum = &bytes[32..];

    // Verify checksum
    let mut hasher = Sha512_256::new();
    hasher.update(public_key);
    let hash = hasher.finalize();
    if &hash[28..32] != checksum {
        bail!("Address checksum mismatch");
    }

    Ok(public_key)
}

/// Convert a 32-byte key to a 25-word Algorand mnemonic.
///
/// Algorithm:
/// 1. Convert 32 bytes (256 bits) to 11-bit groups → 24 values (23 full + 1 partial)
/// 2. Compute SHA-512/256 checksum, take first 11 bits as 25th word
pub fn seed_to_mnemonic(seed: &[u8; 32]) -> String {
    let wl = words();

    // Convert bytes to 11-bit values
    let mut indices = bytes_to_11bit(seed);

    // Compute checksum: first 11 bits of SHA-512/256(seed)
    let mut hasher = Sha512_256::new();
    hasher.update(seed);
    let hash = hasher.finalize();
    let checksum = ((hash[0] as u16) << 3) | ((hash[1] as u16) >> 5);
    indices.push(checksum as usize);

    // Map indices to words
    indices.iter().map(|&i| wl[i]).collect::<Vec<_>>().join(" ")
}

/// Convert a 25-word Algorand mnemonic back to a 32-byte seed.
pub fn mnemonic_to_seed(mnemonic: &str) -> Result<[u8; 32]> {
    let mnemonic_words: Vec<&str> = mnemonic.split_whitespace().collect();
    if mnemonic_words.len() != 25 {
        bail!("Mnemonic must be 25 words (got {})", mnemonic_words.len());
    }

    // Convert words to 11-bit indices
    let mut indices = Vec::with_capacity(25);
    for (i, word) in mnemonic_words.iter().enumerate() {
        match word_index(word) {
            Some(idx) => indices.push(idx),
            None => bail!("Unknown word at position {}: \"{}\"", i + 1, word),
        }
    }

    // First 24 indices → 32 bytes
    let mut seed = bits_to_bytes(&indices[..24])?;

    // Verify checksum (25th word)
    let mut hasher = Sha512_256::new();
    hasher.update(seed);
    let hash = hasher.finalize();
    let expected_checksum = (((hash[0] as u16) << 3) | ((hash[1] as u16) >> 5)) as usize;

    if indices[24] != expected_checksum {
        // Zero out the seed before returning error
        seed.zeroize();
        bail!("Mnemonic checksum mismatch — the phrase may be incorrect");
    }

    Ok(seed)
}

/// Convert bytes to a list of 11-bit values.
fn bytes_to_11bit(data: &[u8]) -> Vec<usize> {
    let mut buffer: u32 = 0;
    let mut num_bits: u32 = 0;
    let mut output = Vec::new();

    for &byte in data {
        buffer = (buffer << 8) | (byte as u32);
        num_bits += 8;
        while num_bits >= 11 {
            num_bits -= 11;
            output.push(((buffer >> num_bits) & 0x7FF) as usize);
        }
    }

    // Handle remaining bits (pad with zeros on the right)
    if num_bits > 0 {
        output.push(((buffer << (11 - num_bits)) & 0x7FF) as usize);
    }

    output
}

/// Convert 24 eleven-bit values back to 32 bytes.
fn bits_to_bytes(indices: &[usize]) -> Result<[u8; 32]> {
    let mut buffer: u32 = 0;
    let mut num_bits: u32 = 0;
    let mut output = Vec::new();

    for &index in indices {
        buffer = (buffer << 11) | (index as u32);
        num_bits += 11;
        while num_bits >= 8 {
            num_bits -= 8;
            output.push(((buffer >> num_bits) & 0xFF) as u8);
        }
    }

    if output.len() < 32 {
        bail!(
            "Failed to reconstruct seed: got {} bytes, expected 32",
            output.len()
        );
    }

    let mut seed = [0u8; 32];
    seed.copy_from_slice(&output[..32]);
    Ok(seed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wordlist_has_2048_entries() {
        assert_eq!(words().len(), 2048);
    }

    #[test]
    fn generate_seed_is_32_bytes() {
        let seed = generate_seed();
        assert_eq!(seed.len(), 32);
    }

    #[test]
    fn mnemonic_roundtrip() {
        let seed = generate_seed();
        let mnemonic = seed_to_mnemonic(&seed);
        let words: Vec<&str> = mnemonic.split_whitespace().collect();
        assert_eq!(words.len(), 25);

        let recovered = mnemonic_to_seed(&mnemonic).unwrap();
        assert_eq!(seed, recovered);
    }

    #[test]
    fn address_from_seed_is_58_chars() {
        let seed = generate_seed();
        let address = address_from_seed(&seed);
        assert_eq!(address.len(), 58);
    }

    #[test]
    fn address_roundtrip() {
        let seed = generate_seed();
        let signing_key = SigningKey::from_bytes(&seed);
        let public_key = signing_key.verifying_key();
        let address = encode_address(public_key.as_bytes());
        let decoded = decode_address(&address).unwrap();
        assert_eq!(decoded, *public_key.as_bytes());
    }

    #[test]
    fn invalid_mnemonic_word_count() {
        let result = mnemonic_to_seed("hello world");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("25 words"));
    }

    #[test]
    fn invalid_mnemonic_unknown_word() {
        let mnemonic = "abandon ".repeat(24) + "xyznotaword";
        let result = mnemonic_to_seed(&mnemonic);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown word"));
    }

    #[test]
    fn invalid_address_bad_checksum() {
        let seed = generate_seed();
        let mut address = address_from_seed(&seed);
        // Flip the last character to corrupt checksum
        let last = address.pop().unwrap();
        let replacement = if last == 'A' { 'B' } else { 'A' };
        address.push(replacement);
        // May fail on base32 decode or checksum — either is fine
        assert!(decode_address(&address).is_err());
    }

    #[test]
    fn deterministic_mnemonic() {
        let seed = [42u8; 32];
        let m1 = seed_to_mnemonic(&seed);
        let m2 = seed_to_mnemonic(&seed);
        assert_eq!(m1, m2);
    }
}
