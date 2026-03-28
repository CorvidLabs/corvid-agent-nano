use anyhow::Result;
use x25519_dalek::{PublicKey, StaticSecret};
use rand::rngs::OsRng;
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};

/// X25519 keypair for AlgoChat encryption.
pub struct KeyPair {
    secret: StaticSecret,
    public: PublicKey,
}

impl KeyPair {
    /// Generate a new random keypair.
    pub fn generate() -> Self {
        let secret = StaticSecret::random_from_rng(OsRng);
        let public = PublicKey::from(&secret);
        Self { secret, public }
    }

    /// Load from a base64-encoded secret key.
    pub fn from_secret_b64(secret_b64: &str) -> Result<Self> {
        let bytes = BASE64.decode(secret_b64)?;
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        let secret = StaticSecret::from(arr);
        let public = PublicKey::from(&secret);
        Ok(Self { secret, public })
    }

    /// Get the public key as base64.
    pub fn public_key_b64(&self) -> String {
        BASE64.encode(self.public.as_bytes())
    }

    /// Get the secret key as base64 (for persistence).
    pub fn secret_key_b64(&self) -> String {
        BASE64.encode(self.secret.to_bytes())
    }

    /// Perform X25519 Diffie-Hellman to derive a shared secret.
    pub fn diffie_hellman(&self, their_public: &[u8; 32]) -> [u8; 32] {
        let their_pk = PublicKey::from(*their_public);
        *self.secret.diffie_hellman(&their_pk).as_bytes()
    }
}
