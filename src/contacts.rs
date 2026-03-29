//! Contact management — SQLite-backed PSK contact storage with CLI operations.

use std::path::Path;
use std::sync::Mutex;

use anyhow::{bail, Result};
use rusqlite::Connection;

/// Acquires the database mutex, converting a poison error into anyhow::Error.
fn lock_db(mutex: &Mutex<Connection>) -> Result<std::sync::MutexGuard<'_, Connection>> {
    mutex
        .lock()
        .map_err(|e| anyhow::anyhow!("Database lock poisoned: {e}"))
}

/// A PSK contact entry.
#[derive(Debug, Clone)]
pub struct Contact {
    pub name: String,
    pub address: String,
    pub psk: Vec<u8>, // 32-byte pre-shared key
    pub added_at: String,
}

/// SQLite-backed contact store.
pub struct ContactStore {
    conn: Mutex<Connection>,
}

impl ContactStore {
    /// Open or create the contacts database.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS contacts (
                name TEXT PRIMARY KEY,
                address TEXT NOT NULL,
                psk BLOB NOT NULL,
                added_at TEXT NOT NULL DEFAULT (datetime('now'))
            );",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Create an in-memory store (for testing).
    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS contacts (
                name TEXT PRIMARY KEY,
                address TEXT NOT NULL,
                psk BLOB NOT NULL,
                added_at TEXT NOT NULL DEFAULT (datetime('now'))
            );",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Add a new contact. Returns error if name already exists (use `upsert` to overwrite).
    pub fn add(&self, name: &str, address: &str, psk: &[u8]) -> Result<()> {
        if psk.len() != 32 {
            bail!("PSK must be exactly 32 bytes (got {})", psk.len());
        }
        let conn = lock_db(&self.conn)?;
        conn.execute(
            "INSERT INTO contacts (name, address, psk) VALUES (?1, ?2, ?3)",
            rusqlite::params![name, address, psk],
        )
        .map_err(|e| {
            if matches!(e, rusqlite::Error::SqliteFailure(_, _)) {
                anyhow::anyhow!(
                    "Contact \"{}\" already exists. Use --force to overwrite.",
                    name
                )
            } else {
                anyhow::anyhow!("Failed to add contact: {}", e)
            }
        })?;
        Ok(())
    }

    /// Add or update a contact (upsert).
    pub fn upsert(&self, name: &str, address: &str, psk: &[u8]) -> Result<()> {
        if psk.len() != 32 {
            bail!("PSK must be exactly 32 bytes (got {})", psk.len());
        }
        let conn = lock_db(&self.conn)?;
        conn.execute(
            "INSERT OR REPLACE INTO contacts (name, address, psk) VALUES (?1, ?2, ?3)",
            rusqlite::params![name, address, psk],
        )?;
        Ok(())
    }

    /// Remove a contact by name.
    pub fn remove(&self, name: &str) -> Result<bool> {
        let conn = lock_db(&self.conn)?;
        let rows = conn.execute("DELETE FROM contacts WHERE name = ?1", [name])?;
        Ok(rows > 0)
    }

    /// List all contacts.
    pub fn list(&self) -> Result<Vec<Contact>> {
        let conn = lock_db(&self.conn)?;
        let mut stmt =
            conn.prepare("SELECT name, address, psk, added_at FROM contacts ORDER BY name")?;
        let contacts = stmt
            .query_map([], |row| {
                Ok(Contact {
                    name: row.get(0)?,
                    address: row.get(1)?,
                    psk: row.get(2)?,
                    added_at: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(contacts)
    }

    /// Get a contact by name.
    #[allow(dead_code)]
    pub fn get(&self, name: &str) -> Result<Option<Contact>> {
        let conn = lock_db(&self.conn)?;
        let mut stmt =
            conn.prepare("SELECT name, address, psk, added_at FROM contacts WHERE name = ?1")?;
        let mut rows = stmt.query_map([name], |row| {
            Ok(Contact {
                name: row.get(0)?,
                address: row.get(1)?,
                psk: row.get(2)?,
                added_at: row.get(3)?,
            })
        })?;
        match rows.next() {
            Some(Ok(c)) => Ok(Some(c)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    /// Get a contact by Algorand address.
    #[allow(dead_code)]
    pub fn get_by_address(&self, address: &str) -> Result<Option<Contact>> {
        let conn = lock_db(&self.conn)?;
        let mut stmt =
            conn.prepare("SELECT name, address, psk, added_at FROM contacts WHERE address = ?1")?;
        let mut rows = stmt.query_map([address], |row| {
            Ok(Contact {
                name: row.get(0)?,
                address: row.get(1)?,
                psk: row.get(2)?,
                added_at: row.get(3)?,
            })
        })?;
        match rows.next() {
            Some(Ok(c)) => Ok(Some(c)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    /// Export all contacts as JSON.
    pub fn export_json(&self) -> Result<String> {
        let contacts = self.list()?;
        let export: Vec<serde_json::Value> = contacts
            .iter()
            .map(|c| {
                serde_json::json!({
                    "name": c.name,
                    "address": c.address,
                    "psk": hex::encode(&c.psk),
                })
            })
            .collect();
        Ok(serde_json::to_string_pretty(&export)?)
    }

    /// Import contacts from JSON (merges with existing).
    pub fn import_json(&self, json: &str) -> Result<usize> {
        let entries: Vec<serde_json::Value> = serde_json::from_str(json)?;
        let mut count = 0;
        for entry in &entries {
            let name = entry["name"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'name' field"))?;
            let address = entry["address"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'address' field"))?;
            let psk_hex = entry["psk"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'psk' field"))?;
            let psk = hex::decode(psk_hex)?;
            self.upsert(name, address, &psk)?;
            count += 1;
        }
        Ok(count)
    }

    /// Count the number of contacts.
    pub fn count(&self) -> Result<usize> {
        let conn = lock_db(&self.conn)?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM contacts", [], |row| row.get(0))?;
        Ok(count as usize)
    }
}

/// Parse a PSK from either hex (64 chars) or base64 (44 chars).
pub fn parse_psk(input: &str) -> Result<[u8; 32]> {
    let bytes = if input.len() == 64 && input.chars().all(|c| c.is_ascii_hexdigit()) {
        hex::decode(input)?
    } else {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(input)
            .map_err(|e| anyhow::anyhow!("Invalid PSK (not hex or base64): {}", e))?
    };

    if bytes.len() != 32 {
        bail!("PSK must be 32 bytes (got {})", bytes.len());
    }

    let mut psk = [0u8; 32];
    psk.copy_from_slice(&bytes);
    Ok(psk)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_list_contacts() {
        let store = ContactStore::in_memory().unwrap();
        store.add("alice", "ALICE_ADDR", &[1u8; 32]).unwrap();
        store.add("bob", "BOB_ADDR", &[2u8; 32]).unwrap();

        let contacts = store.list().unwrap();
        assert_eq!(contacts.len(), 2);
        assert_eq!(contacts[0].name, "alice");
        assert_eq!(contacts[1].name, "bob");
    }

    #[test]
    fn duplicate_name_fails() {
        let store = ContactStore::in_memory().unwrap();
        store.add("alice", "ADDR1", &[1u8; 32]).unwrap();
        let result = store.add("alice", "ADDR2", &[2u8; 32]);
        assert!(result.is_err());
    }

    #[test]
    fn upsert_overwrites() {
        let store = ContactStore::in_memory().unwrap();
        store.upsert("alice", "ADDR1", &[1u8; 32]).unwrap();
        store.upsert("alice", "ADDR2", &[2u8; 32]).unwrap();

        let contact = store.get("alice").unwrap().unwrap();
        assert_eq!(contact.address, "ADDR2");
    }

    #[test]
    fn remove_contact() {
        let store = ContactStore::in_memory().unwrap();
        store.add("alice", "ADDR", &[1u8; 32]).unwrap();
        assert!(store.remove("alice").unwrap());
        assert!(!store.remove("alice").unwrap()); // already gone
        assert_eq!(store.count().unwrap(), 0);
    }

    #[test]
    fn get_by_address() {
        let store = ContactStore::in_memory().unwrap();
        store.add("alice", "ALICE_ADDR", &[1u8; 32]).unwrap();

        let contact = store.get_by_address("ALICE_ADDR").unwrap().unwrap();
        assert_eq!(contact.name, "alice");
        assert!(store.get_by_address("UNKNOWN").unwrap().is_none());
    }

    #[test]
    fn export_import_roundtrip() {
        let store1 = ContactStore::in_memory().unwrap();
        store1.add("alice", "ADDR1", &[1u8; 32]).unwrap();
        store1.add("bob", "ADDR2", &[2u8; 32]).unwrap();

        let json = store1.export_json().unwrap();

        let store2 = ContactStore::in_memory().unwrap();
        let count = store2.import_json(&json).unwrap();
        assert_eq!(count, 2);
        assert_eq!(store2.count().unwrap(), 2);
    }

    #[test]
    fn parse_psk_hex() {
        let hex_str = "a".repeat(64);
        let psk = parse_psk(&hex_str).unwrap();
        assert_eq!(psk, [0xaa; 32]);
    }

    #[test]
    fn parse_psk_wrong_length() {
        let result = parse_psk("aabb");
        assert!(result.is_err());
    }

    #[test]
    fn invalid_psk_length_rejected() {
        let store = ContactStore::in_memory().unwrap();
        let result = store.add("alice", "ADDR", &[1u8; 16]); // 16 bytes, not 32
        assert!(result.is_err());
    }
}
