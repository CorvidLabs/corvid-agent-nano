use serde::{Deserialize, Serialize};

/// An agent's on-chain identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentIdentity {
    /// Algorand address
    pub address: String,
    /// Human-readable name
    pub name: String,
    /// X25519 public key (base64)
    pub public_key: String,
    /// Agent capabilities/tags
    pub capabilities: Vec<String>,
}
