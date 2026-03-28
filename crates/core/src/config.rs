use serde::{Deserialize, Serialize};

/// Runtime configuration for the nano agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NanoConfig {
    pub algod_url: String,
    pub algod_token: String,
    pub agent_name: String,
    pub hub_url: String,
    pub data_dir: String,
}
