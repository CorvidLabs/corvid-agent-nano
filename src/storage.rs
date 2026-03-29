//! SQLite-backed storage implementations for AlgoChat.
//!
//! Provides persistent `EncryptionKeyStorage` and `MessageCache` implementations
//! using SQLite, so that encryption keys and message history survive restarts.

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, UNIX_EPOCH};

use algochat::{
    AlgoChatError, EncryptionKeyStorage, Message, MessageCache, MessageDirection, ReplyContext,
};
use rusqlite::{params, Connection};

/// Maps a rusqlite error to AlgoChatError::StorageFailed.
fn map_db_err(e: rusqlite::Error) -> AlgoChatError {
    AlgoChatError::StorageFailed(format!("SQLite error: {e}"))
}

/// Acquires the database mutex, converting a poison error into StorageFailed.
fn lock_db(mutex: &Mutex<Connection>) -> algochat::Result<std::sync::MutexGuard<'_, Connection>> {
    mutex
        .lock()
        .map_err(|e| AlgoChatError::StorageFailed(format!("Database lock poisoned: {e}")))
}

// ============================================================================
// SQLite Key Storage
// ============================================================================

/// SQLite-backed encryption key storage.
///
/// Stores X25519 private keys in a local SQLite database. Keys are stored as
/// raw bytes (not encrypted at rest — use full-disk encryption or FileKeyStorage
/// if you need encryption at rest).
pub struct SqliteKeyStorage {
    conn: Mutex<Connection>,
}

impl SqliteKeyStorage {
    /// Opens (or creates) a SQLite database at the given path.
    pub fn open(path: impl AsRef<Path>) -> algochat::Result<Self> {
        let conn = Connection::open(path).map_err(map_db_err)?;
        let storage = Self {
            conn: Mutex::new(conn),
        };
        storage.init_tables()?;
        Ok(storage)
    }

    /// Creates an in-memory SQLite database (useful for testing).
    #[cfg(test)]
    pub fn in_memory() -> algochat::Result<Self> {
        let conn = Connection::open_in_memory().map_err(map_db_err)?;
        let storage = Self {
            conn: Mutex::new(conn),
        };
        storage.init_tables()?;
        Ok(storage)
    }

    fn init_tables(&self) -> algochat::Result<()> {
        let conn = lock_db(&self.conn)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS encryption_keys (
                address TEXT PRIMARY KEY,
                private_key BLOB NOT NULL
            );",
        )
        .map_err(map_db_err)?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl EncryptionKeyStorage for SqliteKeyStorage {
    async fn store(
        &self,
        private_key: &[u8; 32],
        address: &str,
        _require_biometric: bool,
    ) -> algochat::Result<()> {
        let conn = lock_db(&self.conn)?;
        conn.execute(
            "INSERT OR REPLACE INTO encryption_keys (address, private_key) VALUES (?1, ?2)",
            params![address, private_key.as_slice()],
        )
        .map_err(map_db_err)?;
        Ok(())
    }

    async fn retrieve(&self, address: &str) -> algochat::Result<[u8; 32]> {
        let conn = lock_db(&self.conn)?;
        let key_bytes: Vec<u8> = conn
            .query_row(
                "SELECT private_key FROM encryption_keys WHERE address = ?1",
                params![address],
                |row| row.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    AlgoChatError::KeyNotFound(address.to_string())
                }
                other => map_db_err(other),
            })?;

        key_bytes
            .try_into()
            .map_err(|_| AlgoChatError::StorageFailed("Invalid key length in database".into()))
    }

    async fn has_key(&self, address: &str) -> bool {
        let Ok(conn) = lock_db(&self.conn) else {
            return false;
        };
        conn.query_row(
            "SELECT 1 FROM encryption_keys WHERE address = ?1",
            params![address],
            |_| Ok(()),
        )
        .is_ok()
    }

    async fn delete(&self, address: &str) -> algochat::Result<()> {
        let conn = lock_db(&self.conn)?;
        conn.execute(
            "DELETE FROM encryption_keys WHERE address = ?1",
            params![address],
        )
        .map_err(map_db_err)?;
        Ok(())
    }

    async fn list_stored_addresses(&self) -> algochat::Result<Vec<String>> {
        let conn = lock_db(&self.conn)?;
        let mut stmt = conn
            .prepare("SELECT address FROM encryption_keys")
            .map_err(map_db_err)?;
        let addresses = stmt
            .query_map([], |row| row.get(0))
            .map_err(map_db_err)?
            .collect::<Result<Vec<String>, _>>()
            .map_err(map_db_err)?;
        Ok(addresses)
    }
}

// ============================================================================
// SQLite Message Cache
// ============================================================================

/// SQLite-backed message cache.
///
/// Persists decrypted AlgoChat messages and sync-round bookmarks so that
/// the agent does not need to re-scan all Algorand transactions on restart.
pub struct SqliteMessageCache {
    conn: Mutex<Connection>,
}

impl SqliteMessageCache {
    /// Opens (or creates) a SQLite database at the given path.
    pub fn open(path: impl AsRef<Path>) -> algochat::Result<Self> {
        let conn = Connection::open(path).map_err(map_db_err)?;
        let cache = Self {
            conn: Mutex::new(conn),
        };
        cache.init_tables()?;
        Ok(cache)
    }

    /// Creates an in-memory SQLite database (useful for testing).
    #[cfg(test)]
    pub fn in_memory() -> algochat::Result<Self> {
        let conn = Connection::open_in_memory().map_err(map_db_err)?;
        let cache = Self {
            conn: Mutex::new(conn),
        };
        cache.init_tables()?;
        Ok(cache)
    }

    fn init_tables(&self) -> algochat::Result<()> {
        let conn = lock_db(&self.conn)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                participant TEXT NOT NULL,
                sender TEXT NOT NULL,
                recipient TEXT NOT NULL,
                content TEXT NOT NULL,
                timestamp_secs INTEGER NOT NULL,
                confirmed_round INTEGER NOT NULL,
                direction TEXT NOT NULL,
                reply_to_id TEXT,
                reply_to_preview TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_messages_participant
                ON messages (participant);
            CREATE INDEX IF NOT EXISTS idx_messages_round
                ON messages (participant, confirmed_round);

            CREATE TABLE IF NOT EXISTS sync_rounds (
                participant TEXT PRIMARY KEY,
                last_round INTEGER NOT NULL
            );",
        )
        .map_err(map_db_err)?;
        Ok(())
    }

    /// Determines the participant address from a message's sender/recipient
    /// relative to the conversation partner.
    fn row_to_message(row: &rusqlite::Row<'_>) -> rusqlite::Result<Message> {
        let id: String = row.get(0)?;
        let sender: String = row.get(2)?;
        let recipient: String = row.get(3)?;
        let content: String = row.get(4)?;
        let timestamp_secs: u64 = row.get(5)?;
        let confirmed_round: u64 = row.get(6)?;
        let direction_str: String = row.get(7)?;
        let reply_to_id: Option<String> = row.get(8)?;
        let reply_to_preview: Option<String> = row.get(9)?;

        let direction = match direction_str.as_str() {
            "sent" => MessageDirection::Sent,
            _ => MessageDirection::Received,
        };

        let timestamp = UNIX_EPOCH + Duration::from_secs(timestamp_secs);

        let reply_context =
            reply_to_id.map(|rid| ReplyContext::new(rid, reply_to_preview.unwrap_or_default()));

        Ok(Message::new(
            id,
            sender,
            recipient,
            content,
            timestamp,
            confirmed_round,
            direction,
            reply_context,
        ))
    }
}

#[async_trait::async_trait]
impl MessageCache for SqliteMessageCache {
    async fn store(&self, messages: &[Message], participant: &str) -> algochat::Result<()> {
        let conn = lock_db(&self.conn)?;
        for msg in messages {
            let direction_str = match msg.direction {
                MessageDirection::Sent => "sent",
                MessageDirection::Received => "received",
            };
            let timestamp_secs = msg
                .timestamp
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_secs();

            conn.execute(
                "INSERT OR IGNORE INTO messages
                    (id, participant, sender, recipient, content, timestamp_secs,
                     confirmed_round, direction, reply_to_id, reply_to_preview)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    msg.id,
                    participant,
                    msg.sender,
                    msg.recipient,
                    msg.content,
                    timestamp_secs,
                    msg.confirmed_round,
                    direction_str,
                    msg.reply_context.as_ref().map(|r| &r.message_id),
                    msg.reply_context.as_ref().map(|r| &r.preview),
                ],
            )
            .map_err(map_db_err)?;
        }
        Ok(())
    }

    async fn retrieve(
        &self,
        participant: &str,
        after_round: Option<u64>,
    ) -> algochat::Result<Vec<Message>> {
        let conn = lock_db(&self.conn)?;
        let (query, round_param): (&str, Option<u64>) = match after_round {
            Some(round) => (
                "SELECT id, participant, sender, recipient, content, timestamp_secs,
                        confirmed_round, direction, reply_to_id, reply_to_preview
                 FROM messages
                 WHERE participant = ?1 AND confirmed_round > ?2
                 ORDER BY timestamp_secs ASC",
                Some(round),
            ),
            None => (
                "SELECT id, participant, sender, recipient, content, timestamp_secs,
                        confirmed_round, direction, reply_to_id, reply_to_preview
                 FROM messages
                 WHERE participant = ?1
                 ORDER BY timestamp_secs ASC",
                None,
            ),
        };

        let mut stmt = conn.prepare(query).map_err(map_db_err)?;
        let rows = if let Some(round) = round_param {
            stmt.query_map(params![participant, round], Self::row_to_message)
                .map_err(map_db_err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(map_db_err)?
        } else {
            stmt.query_map(params![participant], Self::row_to_message)
                .map_err(map_db_err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(map_db_err)?
        };

        Ok(rows)
    }

    async fn get_last_sync_round(&self, participant: &str) -> algochat::Result<Option<u64>> {
        let conn = lock_db(&self.conn)?;
        match conn.query_row(
            "SELECT last_round FROM sync_rounds WHERE participant = ?1",
            params![participant],
            |row| row.get(0),
        ) {
            Ok(round) => Ok(Some(round)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(map_db_err(e)),
        }
    }

    async fn set_last_sync_round(&self, round: u64, participant: &str) -> algochat::Result<()> {
        let conn = lock_db(&self.conn)?;
        conn.execute(
            "INSERT OR REPLACE INTO sync_rounds (participant, last_round) VALUES (?1, ?2)",
            params![participant, round],
        )
        .map_err(map_db_err)?;
        Ok(())
    }

    async fn get_cached_conversations(&self) -> algochat::Result<Vec<String>> {
        let conn = lock_db(&self.conn)?;
        let mut stmt = conn
            .prepare("SELECT DISTINCT participant FROM messages")
            .map_err(map_db_err)?;
        let participants = stmt
            .query_map([], |row| row.get(0))
            .map_err(map_db_err)?
            .collect::<Result<Vec<String>, _>>()
            .map_err(map_db_err)?;
        Ok(participants)
    }

    async fn clear(&self) -> algochat::Result<()> {
        let conn = lock_db(&self.conn)?;
        conn.execute_batch("DELETE FROM messages; DELETE FROM sync_rounds;")
            .map_err(map_db_err)?;
        Ok(())
    }

    async fn clear_for(&self, participant: &str) -> algochat::Result<()> {
        let conn = lock_db(&self.conn)?;
        conn.execute(
            "DELETE FROM messages WHERE participant = ?1",
            params![participant],
        )
        .map_err(map_db_err)?;
        conn.execute(
            "DELETE FROM sync_rounds WHERE participant = ?1",
            params![participant],
        )
        .map_err(map_db_err)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_message(id: &str, round: u64) -> Message {
        Message::new(
            id,
            "sender",
            "recipient",
            "content",
            UNIX_EPOCH + Duration::from_secs(round),
            round,
            MessageDirection::Sent,
            None,
        )
    }

    // ---- Key Storage Tests ----

    #[tokio::test]
    async fn key_storage_store_and_retrieve() {
        let storage = SqliteKeyStorage::in_memory().unwrap();
        let key = [42u8; 32];

        assert!(!storage.has_key("ADDR1").await);
        storage.store(&key, "ADDR1", false).await.unwrap();
        assert!(storage.has_key("ADDR1").await);

        let retrieved = storage.retrieve("ADDR1").await.unwrap();
        assert_eq!(retrieved, key);
    }

    #[tokio::test]
    async fn key_storage_retrieve_missing() {
        let storage = SqliteKeyStorage::in_memory().unwrap();
        let result = storage.retrieve("NONEXISTENT").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn key_storage_overwrite() {
        let storage = SqliteKeyStorage::in_memory().unwrap();
        storage.store(&[1u8; 32], "ADDR1", false).await.unwrap();
        storage.store(&[2u8; 32], "ADDR1", false).await.unwrap();
        let retrieved = storage.retrieve("ADDR1").await.unwrap();
        assert_eq!(retrieved, [2u8; 32]);
    }

    #[tokio::test]
    async fn key_storage_delete() {
        let storage = SqliteKeyStorage::in_memory().unwrap();
        storage.store(&[42u8; 32], "ADDR1", false).await.unwrap();
        storage.delete("ADDR1").await.unwrap();
        assert!(!storage.has_key("ADDR1").await);
        // Deleting non-existent is fine
        storage.delete("ADDR1").await.unwrap();
    }

    #[tokio::test]
    async fn key_storage_list_addresses() {
        let storage = SqliteKeyStorage::in_memory().unwrap();
        storage.store(&[1u8; 32], "ADDR_A", false).await.unwrap();
        storage.store(&[2u8; 32], "ADDR_B", false).await.unwrap();

        let mut addrs = storage.list_stored_addresses().await.unwrap();
        addrs.sort();
        assert_eq!(addrs, vec!["ADDR_A", "ADDR_B"]);
    }

    // ---- Message Cache Tests ----

    #[tokio::test]
    async fn message_cache_store_and_retrieve() {
        let cache = SqliteMessageCache::in_memory().unwrap();
        let messages = vec![test_message("tx1", 100), test_message("tx2", 200)];

        cache.store(&messages, "alice").await.unwrap();

        let retrieved = cache.retrieve("alice", None).await.unwrap();
        assert_eq!(retrieved.len(), 2);
        assert_eq!(retrieved[0].id, "tx1");
        assert_eq!(retrieved[1].id, "tx2");
    }

    #[tokio::test]
    async fn message_cache_retrieve_after_round() {
        let cache = SqliteMessageCache::in_memory().unwrap();
        let messages = vec![test_message("tx1", 100), test_message("tx2", 200)];
        cache.store(&messages, "alice").await.unwrap();

        let after_100 = cache.retrieve("alice", Some(100)).await.unwrap();
        assert_eq!(after_100.len(), 1);
        assert_eq!(after_100[0].id, "tx2");
    }

    #[tokio::test]
    async fn message_cache_dedup() {
        let cache = SqliteMessageCache::in_memory().unwrap();
        let msg = test_message("tx1", 100);

        cache.store(&[msg.clone()], "alice").await.unwrap();
        cache.store(&[msg], "alice").await.unwrap();

        let retrieved = cache.retrieve("alice", None).await.unwrap();
        assert_eq!(retrieved.len(), 1);
    }

    #[tokio::test]
    async fn message_cache_sync_rounds() {
        let cache = SqliteMessageCache::in_memory().unwrap();

        assert_eq!(cache.get_last_sync_round("alice").await.unwrap(), None);

        cache.set_last_sync_round(500, "alice").await.unwrap();
        assert_eq!(cache.get_last_sync_round("alice").await.unwrap(), Some(500));

        cache.set_last_sync_round(600, "alice").await.unwrap();
        assert_eq!(cache.get_last_sync_round("alice").await.unwrap(), Some(600));
    }

    #[tokio::test]
    async fn message_cache_conversations() {
        let cache = SqliteMessageCache::in_memory().unwrap();
        cache
            .store(&[test_message("tx1", 100)], "alice")
            .await
            .unwrap();
        cache
            .store(&[test_message("tx2", 200)], "bob")
            .await
            .unwrap();

        let mut convs = cache.get_cached_conversations().await.unwrap();
        convs.sort();
        assert_eq!(convs, vec!["alice", "bob"]);
    }

    #[tokio::test]
    async fn message_cache_clear() {
        let cache = SqliteMessageCache::in_memory().unwrap();
        cache
            .store(&[test_message("tx1", 100)], "alice")
            .await
            .unwrap();
        cache.set_last_sync_round(100, "alice").await.unwrap();

        cache.clear().await.unwrap();

        let retrieved = cache.retrieve("alice", None).await.unwrap();
        assert!(retrieved.is_empty());
        assert_eq!(cache.get_last_sync_round("alice").await.unwrap(), None);
    }

    #[tokio::test]
    async fn message_cache_clear_for() {
        let cache = SqliteMessageCache::in_memory().unwrap();
        cache
            .store(&[test_message("tx1", 100)], "alice")
            .await
            .unwrap();
        cache
            .store(&[test_message("tx2", 200)], "bob")
            .await
            .unwrap();
        cache.set_last_sync_round(100, "alice").await.unwrap();
        cache.set_last_sync_round(200, "bob").await.unwrap();

        cache.clear_for("alice").await.unwrap();

        assert!(cache.retrieve("alice", None).await.unwrap().is_empty());
        assert_eq!(cache.get_last_sync_round("alice").await.unwrap(), None);
        assert_eq!(cache.retrieve("bob", None).await.unwrap().len(), 1);
        assert_eq!(cache.get_last_sync_round("bob").await.unwrap(), Some(200));
    }

    #[tokio::test]
    async fn message_cache_reply_context_roundtrip() {
        let cache = SqliteMessageCache::in_memory().unwrap();
        let msg = Message::new(
            "tx1",
            "sender",
            "recipient",
            "reply content",
            UNIX_EPOCH + Duration::from_secs(100),
            100,
            MessageDirection::Received,
            Some(ReplyContext::new("orig_tx", "original message")),
        );

        cache.store(&[msg], "alice").await.unwrap();

        let retrieved = cache.retrieve("alice", None).await.unwrap();
        assert_eq!(retrieved.len(), 1);
        let reply = retrieved[0].reply_context.as_ref().unwrap();
        assert_eq!(reply.message_id, "orig_tx");
        assert_eq!(reply.preview, "original message");
    }

    #[tokio::test]
    async fn message_cache_direction_roundtrip() {
        let cache = SqliteMessageCache::in_memory().unwrap();
        let sent = Message::new(
            "tx1",
            "me",
            "alice",
            "hello",
            UNIX_EPOCH + Duration::from_secs(100),
            100,
            MessageDirection::Sent,
            None,
        );
        let received = Message::new(
            "tx2",
            "alice",
            "me",
            "hi",
            UNIX_EPOCH + Duration::from_secs(200),
            200,
            MessageDirection::Received,
            None,
        );

        cache.store(&[sent, received], "alice").await.unwrap();

        let retrieved = cache.retrieve("alice", None).await.unwrap();
        assert_eq!(retrieved[0].direction, MessageDirection::Sent);
        assert_eq!(retrieved[1].direction, MessageDirection::Received);
    }

    #[tokio::test]
    async fn key_storage_file_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("keys.db");
        let key = [99u8; 32];

        // Store key
        {
            let storage = SqliteKeyStorage::open(&db_path).unwrap();
            storage.store(&key, "PERSIST_ADDR", false).await.unwrap();
        }

        // Reopen and verify persistence
        {
            let storage = SqliteKeyStorage::open(&db_path).unwrap();
            let retrieved = storage.retrieve("PERSIST_ADDR").await.unwrap();
            assert_eq!(retrieved, key);
        }
    }

    #[tokio::test]
    async fn message_cache_file_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("messages.db");

        // Store messages and sync round
        {
            let cache = SqliteMessageCache::open(&db_path).unwrap();
            cache
                .store(&[test_message("tx1", 500)], "alice")
                .await
                .unwrap();
            cache.set_last_sync_round(500, "alice").await.unwrap();
        }

        // Reopen and verify persistence
        {
            let cache = SqliteMessageCache::open(&db_path).unwrap();
            let messages = cache.retrieve("alice", None).await.unwrap();
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].id, "tx1");
            assert_eq!(cache.get_last_sync_round("alice").await.unwrap(), Some(500));
        }
    }
}
