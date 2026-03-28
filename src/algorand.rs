//! HTTP-based Algorand client implementations.
//!
//! Implements the `AlgodClient` and `IndexerClient` traits from rs-algochat
//! using reqwest to talk to real Algorand nodes.

use algochat::{
    AccountInfo, AlgoChatError, AlgodClient, IndexerClient, NoteTransaction, SuggestedParams,
    TransactionInfo,
};
use algochat::Result;
use reqwest::Client;
use serde::Deserialize;
use tracing::debug;

/// HTTP client for the Algorand algod REST API.
pub struct HttpAlgodClient {
    client: Client,
    base_url: String,
    token: String,
}

impl HttpAlgodClient {
    pub fn new(base_url: &str, token: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
        }
    }
}

#[derive(Deserialize)]
struct AlgodStatus {
    #[serde(rename = "last-round")]
    last_round: u64,
}

#[derive(Deserialize)]
struct AlgodTransactionParams {
    fee: u64,
    #[serde(rename = "min-fee")]
    min_fee: u64,
    #[serde(rename = "last-round")]
    last_round: u64,
    #[serde(rename = "genesis-id")]
    genesis_id: String,
    #[serde(rename = "genesis-hash")]
    genesis_hash: String,
}

#[derive(Deserialize)]
struct AlgodAccountInfo {
    address: String,
    amount: u64,
    #[serde(rename = "min-balance")]
    min_balance: u64,
}

#[derive(Deserialize)]
#[allow(non_snake_case)]
struct AlgodSubmitResponse {
    txId: String,
}

#[derive(Deserialize)]
struct AlgodPendingTx {
    #[serde(rename = "confirmed-round")]
    confirmed_round: Option<u64>,
    #[serde(rename = "pool-error")]
    pool_error: Option<String>,
}

#[async_trait::async_trait]
impl AlgodClient for HttpAlgodClient {
    async fn get_suggested_params(&self) -> Result<SuggestedParams> {
        let url = format!("{}/v2/transactions/params", self.base_url);
        let resp: AlgodTransactionParams = self
            .client
            .get(&url)
            .header("X-Algo-API-Token", &self.token)
            .send()
            .await
            .map_err(|e| AlgoChatError::TransactionFailed(e.to_string()))?
            .json()
            .await
            .map_err(|e| AlgoChatError::TransactionFailed(e.to_string()))?;

        let genesis_hash_bytes = base64_decode_32(&resp.genesis_hash)?;

        Ok(SuggestedParams {
            fee: resp.fee,
            min_fee: resp.min_fee,
            first_valid: resp.last_round,
            last_valid: resp.last_round + 1000,
            genesis_id: resp.genesis_id,
            genesis_hash: genesis_hash_bytes,
        })
    }

    async fn get_account_info(&self, address: &str) -> Result<AccountInfo> {
        let url = format!("{}/v2/accounts/{}", self.base_url, address);
        let resp: AlgodAccountInfo = self
            .client
            .get(&url)
            .header("X-Algo-API-Token", &self.token)
            .send()
            .await
            .map_err(|e| AlgoChatError::TransactionFailed(e.to_string()))?
            .json()
            .await
            .map_err(|e| AlgoChatError::TransactionFailed(e.to_string()))?;

        Ok(AccountInfo {
            address: resp.address,
            amount: resp.amount,
            min_balance: resp.min_balance,
        })
    }

    async fn submit_transaction(&self, signed_txn: &[u8]) -> Result<String> {
        let url = format!("{}/v2/transactions", self.base_url);
        let resp: AlgodSubmitResponse = self
            .client
            .post(&url)
            .header("X-Algo-API-Token", &self.token)
            .header("Content-Type", "application/x-binary")
            .body(signed_txn.to_vec())
            .send()
            .await
            .map_err(|e| AlgoChatError::TransactionFailed(e.to_string()))?
            .json()
            .await
            .map_err(|e| AlgoChatError::TransactionFailed(e.to_string()))?;

        Ok(resp.txId)
    }

    async fn wait_for_confirmation(&self, txid: &str, rounds: u32) -> Result<TransactionInfo> {
        let url = format!("{}/v2/transactions/pending/{}", self.base_url, txid);

        for _ in 0..rounds {
            let resp: AlgodPendingTx = self
                .client
                .get(&url)
                .header("X-Algo-API-Token", &self.token)
                .send()
                .await
                .map_err(|e| AlgoChatError::TransactionFailed(e.to_string()))?
                .json()
                .await
                .map_err(|e| AlgoChatError::TransactionFailed(e.to_string()))?;

            if let Some(error) = &resp.pool_error {
                if !error.is_empty() {
                    return Err(AlgoChatError::TransactionFailed(error.clone()));
                }
            }

            if let Some(round) = resp.confirmed_round {
                return Ok(TransactionInfo {
                    txid: txid.to_string(),
                    confirmed_round: Some(round),
                });
            }

            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }

        Err(AlgoChatError::TransactionFailed(format!(
            "Transaction {} not confirmed after {} rounds",
            txid, rounds
        )))
    }

    async fn get_current_round(&self) -> Result<u64> {
        let url = format!("{}/v2/status", self.base_url);
        let resp: AlgodStatus = self
            .client
            .get(&url)
            .header("X-Algo-API-Token", &self.token)
            .send()
            .await
            .map_err(|e| AlgoChatError::TransactionFailed(e.to_string()))?
            .json()
            .await
            .map_err(|e| AlgoChatError::TransactionFailed(e.to_string()))?;

        Ok(resp.last_round)
    }
}

/// HTTP client for the Algorand Indexer REST API.
pub struct HttpIndexerClient {
    client: Client,
    base_url: String,
    token: String,
}

impl HttpIndexerClient {
    pub fn new(base_url: &str, token: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
        }
    }
}

#[derive(Deserialize)]
struct IndexerSearchResponse {
    transactions: Option<Vec<IndexerTransaction>>,
}

#[derive(Deserialize)]
struct IndexerTransaction {
    id: String,
    sender: String,
    #[serde(rename = "payment-transaction")]
    payment_transaction: Option<IndexerPaymentTx>,
    note: Option<String>,
    #[serde(rename = "confirmed-round")]
    confirmed_round: u64,
    #[serde(rename = "round-time")]
    round_time: u64,
}

#[derive(Deserialize)]
struct IndexerPaymentTx {
    receiver: String,
}

#[derive(Deserialize)]
struct IndexerSingleTxResponse {
    transaction: IndexerTransaction,
}

impl IndexerTransaction {
    fn to_note_transaction(&self) -> Option<NoteTransaction> {
        let receiver = self
            .payment_transaction
            .as_ref()
            .map(|p| p.receiver.clone())
            .unwrap_or_default();

        let note = self
            .note
            .as_ref()
            .and_then(|n| base64::decode(n).ok())
            .unwrap_or_default();

        Some(NoteTransaction {
            txid: self.id.clone(),
            sender: self.sender.clone(),
            receiver,
            note,
            confirmed_round: self.confirmed_round,
            round_time: self.round_time,
        })
    }
}

#[async_trait::async_trait]
impl IndexerClient for HttpIndexerClient {
    async fn search_transactions(
        &self,
        address: &str,
        after_round: Option<u64>,
        limit: Option<u32>,
    ) -> Result<Vec<NoteTransaction>> {
        let mut url = format!(
            "{}/v2/transactions?address={}&note-prefix=AQ",
            self.base_url, address
        );
        if let Some(round) = after_round {
            url.push_str(&format!("&min-round={}", round + 1));
        }
        if let Some(limit) = limit {
            url.push_str(&format!("&limit={}", limit));
        }

        debug!(url = %url, "searching indexer transactions");

        let resp: IndexerSearchResponse = self
            .client
            .get(&url)
            .header("X-Indexer-API-Token", &self.token)
            .send()
            .await
            .map_err(|e| AlgoChatError::TransactionFailed(e.to_string()))?
            .json()
            .await
            .map_err(|e| AlgoChatError::TransactionFailed(e.to_string()))?;

        Ok(resp
            .transactions
            .unwrap_or_default()
            .iter()
            .filter_map(|tx| tx.to_note_transaction())
            .collect())
    }

    async fn search_transactions_between(
        &self,
        address1: &str,
        address2: &str,
        after_round: Option<u64>,
        limit: Option<u32>,
    ) -> Result<Vec<NoteTransaction>> {
        // Indexer doesn't have a direct "between two addresses" endpoint.
        // Fetch for address1, then filter to only include txns involving address2.
        let all = self
            .search_transactions(address1, after_round, limit)
            .await?;
        Ok(all
            .into_iter()
            .filter(|tx| {
                (tx.sender == address1 && tx.receiver == address2)
                    || (tx.sender == address2 && tx.receiver == address1)
            })
            .collect())
    }

    async fn get_transaction(&self, txid: &str) -> Result<NoteTransaction> {
        let url = format!("{}/v2/transactions/{}", self.base_url, txid);
        let resp: IndexerSingleTxResponse = self
            .client
            .get(&url)
            .header("X-Indexer-API-Token", &self.token)
            .send()
            .await
            .map_err(|e| AlgoChatError::TransactionFailed(e.to_string()))?
            .json()
            .await
            .map_err(|e| AlgoChatError::TransactionFailed(e.to_string()))?;

        resp.transaction
            .to_note_transaction()
            .ok_or_else(|| AlgoChatError::TransactionFailed("Failed to parse transaction".into()))
    }

    async fn wait_for_indexer(&self, txid: &str, timeout_secs: u32) -> Result<NoteTransaction> {
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs as u64);

        while std::time::Instant::now() < deadline {
            match self.get_transaction(txid).await {
                Ok(tx) => return Ok(tx),
                Err(_) => {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        }

        Err(AlgoChatError::TransactionFailed(format!(
            "Transaction {} not indexed after {}s",
            txid, timeout_secs
        )))
    }
}

/// Decode a base64 string to a 32-byte array.
fn base64_decode_32(s: &str) -> Result<[u8; 32]> {
    let bytes = base64::decode(s)
        .map_err(|e| AlgoChatError::EncodingError(format!("Invalid base64: {}", e)))?;
    if bytes.len() != 32 {
        return Err(AlgoChatError::EncodingError(format!(
            "Expected 32 bytes, got {}",
            bytes.len()
        )));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Ok(arr)
}

/// Convenience module for base64 decoding (using the base64 crate).
mod base64 {
    use data_encoding::BASE64;

    pub fn decode(s: &str) -> std::result::Result<Vec<u8>, data_encoding::DecodeError> {
        BASE64.decode(s.as_bytes())
    }
}
