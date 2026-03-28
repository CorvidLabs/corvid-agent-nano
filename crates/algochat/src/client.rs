use anyhow::Result;
use tracing::info;

/// Client for sending/receiving AlgoChat messages via Algorand REST API.
pub struct AlgoChatClient {
    algod_url: String,
    algod_token: String,
    http: reqwest::Client,
}

impl AlgoChatClient {
    pub fn new(algod_url: &str, algod_token: &str) -> Self {
        Self {
            algod_url: algod_url.to_string(),
            algod_token: algod_token.to_string(),
            http: reqwest::Client::new(),
        }
    }

    /// Check connection to the Algorand node.
    pub async fn health_check(&self) -> Result<bool> {
        let resp = self
            .http
            .get(format!("{}/health", self.algod_url))
            .header("X-Algo-API-Token", &self.algod_token)
            .send()
            .await?;

        let ok = resp.status().is_success();
        info!(ok, "algorand node health check");
        Ok(ok)
    }

    // TODO: send_message — construct payment txn with encrypted note field
    // TODO: poll_messages — watch for incoming transactions
    // TODO: parse_note — decrypt and decode AlgoChat message format
}
