use serde::{Deserialize, Serialize};

/// An AlgoChat message (decrypted).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub from: String,
    pub to: String,
    pub content: String,
    pub timestamp: u64,
    /// Transaction ID on Algorand
    pub txid: Option<String>,
}
