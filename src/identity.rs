//! Identity generation and Algorand address derivation.

use data_encoding::BASE32_NOPAD;
use ed25519_dalek::SigningKey;
use rand::RngCore;
use sha2::{Digest, Sha512_256};

/// Generate a new random 32-byte seed.
pub fn generate_seed() -> [u8; 32] {
    let mut seed = [0u8; 32];
    rand::rng().fill_bytes(&mut seed);
    seed
}

/// Derive the Algorand address from a 32-byte Ed25519 seed.
pub fn address_from_seed(seed: &[u8; 32]) -> String {
    let signing_key = SigningKey::from_bytes(seed);
    let public_key = signing_key.verifying_key().to_bytes();

    // Algorand address = base32(public_key || checksum)
    // checksum = last 4 bytes of SHA-512/256(public_key)
    let mut hasher = Sha512_256::new();
    hasher.update(public_key);
    let hash = hasher.finalize();
    let checksum = &hash[28..32];

    let mut addr_bytes = Vec::with_capacity(36);
    addr_bytes.extend_from_slice(&public_key);
    addr_bytes.extend_from_slice(checksum);

    BASE32_NOPAD.encode(&addr_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_address_is_valid() {
        let seed = generate_seed();
        let address = address_from_seed(&seed);

        // Algorand addresses are 58 characters, all uppercase + digits
        assert_eq!(address.len(), 58);
        assert!(address.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()));
    }

    #[test]
    fn deterministic_address() {
        let seed = [42u8; 32];
        let addr1 = address_from_seed(&seed);
        let addr2 = address_from_seed(&seed);
        assert_eq!(addr1, addr2);
    }
}
