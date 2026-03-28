//! Encrypted vault for seed and PSK storage.
//!
//! Uses Argon2id for key derivation and ChaCha20-Poly1305 for encryption.
//! Secrets are zeroized from memory on drop.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use argon2::Argon2;
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Salt length for Argon2id.
const SALT_LEN: usize = 16;
/// Nonce length for ChaCha20-Poly1305.
const NONCE_LEN: usize = 12;
/// Derived key length.
const KEY_LEN: usize = 32;
/// Magic bytes to identify vault files.
const VAULT_MAGIC: &[u8; 4] = b"NANO";
/// Vault format version.
const VAULT_VERSION: u8 = 1;

/// A contact entry with address and pre-shared key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contact {
    pub name: String,
    pub address: String,
    #[serde(with = "base64_bytes")]
    pub psk: Vec<u8>,
}

/// The plaintext vault contents — zeroized on drop.
#[derive(Debug, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct VaultContents {
    /// 32-byte Ed25519 seed (hex-encoded for serialization).
    pub seed_hex: String,
    /// Algorand address derived from seed.
    pub address: String,
    /// Known contacts with PSKs.
    #[zeroize(skip)]
    pub contacts: Vec<Contact>,
}

/// Encrypted vault on disk.
///
/// File format: MAGIC(4) || VERSION(1) || SALT(16) || NONCE(12) || CIPHERTEXT(variable)
pub struct Vault;

impl Vault {
    /// Create a new vault file with the given contents, encrypted under `passphrase`.
    pub fn create(path: &Path, contents: &VaultContents, passphrase: &str) -> Result<()> {
        let plaintext = serde_json::to_vec(contents).context("Failed to serialize vault")?;
        let encrypted = Self::encrypt(&plaintext, passphrase)?;

        // Ensure parent directory exists with secure permissions
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("Failed to create vault directory")?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                // Best-effort: may fail if parent is /tmp or similar
                let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
            }
        }

        fs::write(path, &encrypted).context("Failed to write vault file")?;

        // Set file permissions to owner-only
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .context("Failed to set vault file permissions")?;
        }

        Ok(())
    }

    /// Open and decrypt an existing vault file.
    pub fn open(path: &Path, passphrase: &str) -> Result<VaultContents> {
        let data = fs::read(path).context("Failed to read vault file")?;
        let plaintext = Self::decrypt(&data, passphrase)?;
        let contents: VaultContents =
            serde_json::from_slice(&plaintext).context("Failed to parse vault contents")?;
        Ok(contents)
    }

    /// Update an existing vault (decrypt, apply changes, re-encrypt).
    pub fn update<F>(path: &Path, passphrase: &str, f: F) -> Result<()>
    where
        F: FnOnce(&mut VaultContents),
    {
        let mut contents = Self::open(path, passphrase)?;
        f(&mut contents);
        Self::create(path, &contents, passphrase)?;
        Ok(())
    }

    /// Check if a vault file exists at the given path.
    pub fn exists(path: &Path) -> bool {
        path.is_file()
    }

    fn encrypt(plaintext: &[u8], passphrase: &str) -> Result<Vec<u8>> {
        let mut salt = [0u8; SALT_LEN];
        rand::rng().fill_bytes(&mut salt);

        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::rng().fill_bytes(&mut nonce_bytes);

        let key = Self::derive_key(passphrase, &salt)?;
        let cipher = ChaCha20Poly1305::new((&key).into());
        let nonce = chacha20poly1305::Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| anyhow::anyhow!("Encryption failed: {}", e))?;

        // Assemble: MAGIC || VERSION || SALT || NONCE || CIPHERTEXT
        let mut output = Vec::with_capacity(4 + 1 + SALT_LEN + NONCE_LEN + ciphertext.len());
        output.extend_from_slice(VAULT_MAGIC);
        output.push(VAULT_VERSION);
        output.extend_from_slice(&salt);
        output.extend_from_slice(&nonce_bytes);
        output.extend_from_slice(&ciphertext);

        Ok(output)
    }

    fn decrypt(data: &[u8], passphrase: &str) -> Result<Vec<u8>> {
        let header_len = 4 + 1 + SALT_LEN + NONCE_LEN;
        if data.len() < header_len {
            anyhow::bail!("Vault file too short");
        }

        // Validate magic and version
        if &data[..4] != VAULT_MAGIC {
            anyhow::bail!("Not a valid vault file (bad magic)");
        }
        if data[4] != VAULT_VERSION {
            anyhow::bail!("Unsupported vault version: {}", data[4]);
        }

        let salt = &data[5..5 + SALT_LEN];
        let nonce_bytes = &data[5 + SALT_LEN..5 + SALT_LEN + NONCE_LEN];
        let ciphertext = &data[header_len..];

        let key = Self::derive_key(passphrase, salt)?;
        let cipher = ChaCha20Poly1305::new((&key).into());
        let nonce = chacha20poly1305::Nonce::from_slice(nonce_bytes);

        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|_| anyhow::anyhow!("Decryption failed — wrong passphrase?"))?;

        Ok(plaintext)
    }

    fn derive_key(passphrase: &str, salt: &[u8]) -> Result<[u8; KEY_LEN]> {
        let mut key = [0u8; KEY_LEN];
        Argon2::default()
            .hash_password_into(passphrase.as_bytes(), salt, &mut key)
            .map_err(|e| anyhow::anyhow!("Key derivation failed: {}", e))?;
        Ok(key)
    }
}

/// Base64 serde helper for PSK bytes.
mod base64_bytes {
    use data_encoding::BASE64;
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&BASE64.encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        BASE64
            .decode(s.as_bytes())
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn roundtrip_vault() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();

        let contents = VaultContents {
            seed_hex: "ab".repeat(32),
            address: "TESTADDR".to_string(),
            contacts: vec![Contact {
                name: "alice".to_string(),
                address: "ALICEADDR".to_string(),
                psk: vec![1, 2, 3, 4, 5],
            }],
        };

        Vault::create(path, &contents, "hunter2").unwrap();
        let recovered = Vault::open(path, "hunter2").unwrap();

        assert_eq!(recovered.seed_hex, contents.seed_hex);
        assert_eq!(recovered.address, contents.address);
        assert_eq!(recovered.contacts.len(), 1);
        assert_eq!(recovered.contacts[0].name, "alice");
        assert_eq!(recovered.contacts[0].psk, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn wrong_passphrase_fails() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();

        let contents = VaultContents {
            seed_hex: "ab".repeat(32),
            address: "TEST".to_string(),
            contacts: vec![],
        };

        Vault::create(path, &contents, "correct").unwrap();
        let result = Vault::open(path, "wrong");
        assert!(result.is_err());
    }

    #[test]
    fn update_vault() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path();

        let contents = VaultContents {
            seed_hex: "cd".repeat(32),
            address: "TEST".to_string(),
            contacts: vec![],
        };

        Vault::create(path, &contents, "pass").unwrap();

        Vault::update(path, "pass", |c| {
            c.contacts.push(Contact {
                name: "bob".to_string(),
                address: "BOBADDR".to_string(),
                psk: vec![9, 8, 7],
            });
        })
        .unwrap();

        let recovered = Vault::open(path, "pass").unwrap();
        assert_eq!(recovered.contacts.len(), 1);
        assert_eq!(recovered.contacts[0].name, "bob");
    }
}
