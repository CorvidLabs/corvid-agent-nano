//! Group PSK channels — shared pre-shared keys for multi-party messaging.
//!
//! A group is a named collection of members that share a single PSK.
//! Sending to a group encrypts the message with the shared PSK and sends
//! it individually to each member.

use std::path::Path;
use std::sync::Mutex;

use anyhow::{bail, Result};
use rand::RngCore;
use rusqlite::Connection;

/// Acquires the database mutex, converting a poison error into anyhow::Error.
fn lock_db(mutex: &Mutex<Connection>) -> Result<std::sync::MutexGuard<'_, Connection>> {
    mutex
        .lock()
        .map_err(|e| anyhow::anyhow!("Database lock poisoned: {e}"))
}

/// A group entry.
#[derive(Debug, Clone)]
pub struct Group {
    pub name: String,
    pub psk: Vec<u8>, // 32-byte pre-shared key
    pub created_at: String,
}

/// A group member entry.
#[derive(Debug, Clone)]
pub struct GroupMember {
    #[allow(dead_code)]
    pub group_name: String,
    pub address: String,
    pub label: Option<String>,
    pub added_at: String,
}

/// SQLite-backed group store.
pub struct GroupStore {
    conn: Mutex<Connection>,
}

impl GroupStore {
    /// Open or create the groups database.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS groups (
                name TEXT PRIMARY KEY,
                psk BLOB NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS group_members (
                group_name TEXT NOT NULL REFERENCES groups(name) ON DELETE CASCADE,
                address TEXT NOT NULL,
                label TEXT,
                added_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (group_name, address)
            );",
        )?;
        // Enable foreign keys
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Create an in-memory store (for testing).
    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
            CREATE TABLE IF NOT EXISTS groups (
                name TEXT PRIMARY KEY,
                psk BLOB NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS group_members (
                group_name TEXT NOT NULL REFERENCES groups(name) ON DELETE CASCADE,
                address TEXT NOT NULL,
                label TEXT,
                added_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (group_name, address)
            );",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Create a new group with a random PSK.
    pub fn create(&self, name: &str) -> Result<[u8; 32]> {
        let mut psk = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut psk);
        let conn = lock_db(&self.conn)?;
        conn.execute(
            "INSERT INTO groups (name, psk) VALUES (?1, ?2)",
            rusqlite::params![name, psk.as_slice()],
        )
        .map_err(|e| {
            if matches!(e, rusqlite::Error::SqliteFailure(_, _)) {
                anyhow::anyhow!("Group \"{}\" already exists", name)
            } else {
                anyhow::anyhow!("Failed to create group: {}", e)
            }
        })?;
        Ok(psk)
    }

    /// Create a group with a specific PSK (for import).
    pub fn create_with_psk(&self, name: &str, psk: &[u8]) -> Result<()> {
        if psk.len() != 32 {
            bail!("PSK must be exactly 32 bytes (got {})", psk.len());
        }
        let conn = lock_db(&self.conn)?;
        conn.execute(
            "INSERT OR REPLACE INTO groups (name, psk) VALUES (?1, ?2)",
            rusqlite::params![name, psk],
        )?;
        Ok(())
    }

    /// Remove a group and all its members.
    pub fn remove(&self, name: &str) -> Result<bool> {
        let conn = lock_db(&self.conn)?;
        // Delete members first (foreign key cascade may not be enabled everywhere)
        conn.execute(
            "DELETE FROM group_members WHERE group_name = ?1",
            [name],
        )?;
        let rows = conn.execute("DELETE FROM groups WHERE name = ?1", [name])?;
        Ok(rows > 0)
    }

    /// Get a group by name.
    pub fn get(&self, name: &str) -> Result<Option<Group>> {
        let conn = lock_db(&self.conn)?;
        let mut stmt =
            conn.prepare("SELECT name, psk, created_at FROM groups WHERE name = ?1")?;
        let mut rows = stmt.query_map([name], |row| {
            Ok(Group {
                name: row.get(0)?,
                psk: row.get(1)?,
                created_at: row.get(2)?,
            })
        })?;
        match rows.next() {
            Some(Ok(g)) => Ok(Some(g)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    /// List all groups.
    pub fn list(&self) -> Result<Vec<Group>> {
        let conn = lock_db(&self.conn)?;
        let mut stmt = conn.prepare("SELECT name, psk, created_at FROM groups ORDER BY name")?;
        let groups = stmt
            .query_map([], |row| {
                Ok(Group {
                    name: row.get(0)?,
                    psk: row.get(1)?,
                    created_at: row.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(groups)
    }

    /// Add a member to a group.
    pub fn add_member(&self, group_name: &str, address: &str, label: Option<&str>) -> Result<()> {
        // Verify group exists
        if self.get(group_name)?.is_none() {
            bail!("Group \"{}\" does not exist", group_name);
        }
        let conn = lock_db(&self.conn)?;
        conn.execute(
            "INSERT OR REPLACE INTO group_members (group_name, address, label) VALUES (?1, ?2, ?3)",
            rusqlite::params![group_name, address, label],
        )?;
        Ok(())
    }

    /// Remove a member from a group.
    pub fn remove_member(&self, group_name: &str, address: &str) -> Result<bool> {
        let conn = lock_db(&self.conn)?;
        let rows = conn.execute(
            "DELETE FROM group_members WHERE group_name = ?1 AND address = ?2",
            rusqlite::params![group_name, address],
        )?;
        Ok(rows > 0)
    }

    /// List all members of a group.
    pub fn members(&self, group_name: &str) -> Result<Vec<GroupMember>> {
        let conn = lock_db(&self.conn)?;
        let mut stmt = conn.prepare(
            "SELECT group_name, address, label, added_at FROM group_members \
             WHERE group_name = ?1 ORDER BY address",
        )?;
        let members = stmt
            .query_map([group_name], |row| {
                Ok(GroupMember {
                    group_name: row.get(0)?,
                    address: row.get(1)?,
                    label: row.get(2)?,
                    added_at: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(members)
    }

    /// Count groups.
    pub fn count(&self) -> Result<usize> {
        let conn = lock_db(&self.conn)?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM groups", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    /// Export all groups with members as JSON.
    pub fn export_json(&self) -> Result<String> {
        let groups = self.list()?;
        let mut export = Vec::new();
        for g in &groups {
            let members = self.members(&g.name)?;
            let member_entries: Vec<serde_json::Value> = members
                .iter()
                .map(|m| {
                    let mut entry = serde_json::json!({ "address": m.address });
                    if let Some(ref label) = m.label {
                        entry["label"] = serde_json::Value::String(label.clone());
                    }
                    entry
                })
                .collect();
            export.push(serde_json::json!({
                "name": g.name,
                "psk": hex::encode(&g.psk),
                "members": member_entries,
            }));
        }
        Ok(serde_json::to_string_pretty(&export)?)
    }

    /// Import groups from JSON (merges with existing).
    pub fn import_json(&self, json: &str) -> Result<usize> {
        let entries: Vec<serde_json::Value> = serde_json::from_str(json)?;
        let mut count = 0;
        for entry in &entries {
            let name = entry["name"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'name' field"))?;
            let psk_hex = entry["psk"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'psk' field"))?;
            let psk = hex::decode(psk_hex)?;
            self.create_with_psk(name, &psk)?;

            if let Some(members) = entry["members"].as_array() {
                for member in members {
                    let address = member["address"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("Missing member 'address' field"))?;
                    let label = member["label"].as_str();
                    self.add_member(name, address, label)?;
                }
            }
            count += 1;
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_list_groups() {
        let store = GroupStore::in_memory().unwrap();
        let psk = store.create("team-alpha").unwrap();
        assert_eq!(psk.len(), 32);

        let groups = store.list().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "team-alpha");
    }

    #[test]
    fn duplicate_group_fails() {
        let store = GroupStore::in_memory().unwrap();
        store.create("team").unwrap();
        let result = store.create("team");
        assert!(result.is_err());
    }

    #[test]
    fn add_and_list_members() {
        let store = GroupStore::in_memory().unwrap();
        store.create("team").unwrap();
        store
            .add_member("team", "ALICE_ADDR", Some("alice"))
            .unwrap();
        store
            .add_member("team", "BOB_ADDR", Some("bob"))
            .unwrap();

        let members = store.members("team").unwrap();
        assert_eq!(members.len(), 2);
    }

    #[test]
    fn add_member_nonexistent_group_fails() {
        let store = GroupStore::in_memory().unwrap();
        let result = store.add_member("ghost", "ADDR", None);
        assert!(result.is_err());
    }

    #[test]
    fn remove_member() {
        let store = GroupStore::in_memory().unwrap();
        store.create("team").unwrap();
        store.add_member("team", "ADDR1", None).unwrap();
        assert!(store.remove_member("team", "ADDR1").unwrap());
        assert!(!store.remove_member("team", "ADDR1").unwrap());
    }

    #[test]
    fn remove_group_removes_members() {
        let store = GroupStore::in_memory().unwrap();
        store.create("team").unwrap();
        store.add_member("team", "ADDR1", None).unwrap();
        store.add_member("team", "ADDR2", None).unwrap();

        assert!(store.remove("team").unwrap());
        assert_eq!(store.count().unwrap(), 0);
    }

    #[test]
    fn export_import_roundtrip() {
        let store1 = GroupStore::in_memory().unwrap();
        store1.create("team").unwrap();
        store1
            .add_member("team", "ADDR1", Some("alice"))
            .unwrap();
        store1.add_member("team", "ADDR2", None).unwrap();

        let json = store1.export_json().unwrap();

        let store2 = GroupStore::in_memory().unwrap();
        let count = store2.import_json(&json).unwrap();
        assert_eq!(count, 1);
        assert_eq!(store2.count().unwrap(), 1);
        assert_eq!(store2.members("team").unwrap().len(), 2);
    }

    #[test]
    fn get_group() {
        let store = GroupStore::in_memory().unwrap();
        store.create("team").unwrap();

        let group = store.get("team").unwrap().unwrap();
        assert_eq!(group.name, "team");
        assert_eq!(group.psk.len(), 32);

        assert!(store.get("nonexistent").unwrap().is_none());
    }
}
