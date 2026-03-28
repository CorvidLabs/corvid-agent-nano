//! Core types and utilities for corvid-agent-nano.

pub mod agent;
pub mod config;
pub mod message;

pub use agent::AgentIdentity;
pub use message::Message;

// Re-export the external algochat crate for convenient access.
pub use algochat;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_identity_roundtrip() {
        let identity = AgentIdentity {
            address: "ALGO123".to_string(),
            name: "test-agent".to_string(),
            public_key: "dGVzdA==".to_string(),
            capabilities: vec!["chat".to_string(), "code".to_string()],
        };

        let json = serde_json::to_string(&identity).unwrap();
        let parsed: AgentIdentity = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.address, "ALGO123");
        assert_eq!(parsed.name, "test-agent");
        assert_eq!(parsed.public_key, "dGVzdA==");
        assert_eq!(parsed.capabilities, vec!["chat", "code"]);
    }

    #[test]
    fn message_roundtrip() {
        let msg = Message {
            from: "SENDER".to_string(),
            to: "RECIPIENT".to_string(),
            content: "hello world".to_string(),
            timestamp: 1234567890,
            txid: Some("TXID123".to_string()),
        };

        let json = serde_json::to_string(&msg).unwrap();
        let parsed: Message = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.from, "SENDER");
        assert_eq!(parsed.to, "RECIPIENT");
        assert_eq!(parsed.content, "hello world");
        assert_eq!(parsed.timestamp, 1234567890);
        assert_eq!(parsed.txid, Some("TXID123".to_string()));
    }

    #[test]
    fn message_without_txid() {
        let msg = Message {
            from: "A".to_string(),
            to: "B".to_string(),
            content: "test".to_string(),
            timestamp: 0,
            txid: None,
        };

        let json = serde_json::to_string(&msg).unwrap();
        let parsed: Message = serde_json::from_str(&json).unwrap();
        assert!(parsed.txid.is_none());
    }

    #[test]
    fn nano_config_roundtrip() {
        let config = config::NanoConfig {
            algod_url: "http://localhost:4001".to_string(),
            algod_token: "token".to_string(),
            agent_name: "nano".to_string(),
            hub_url: "http://localhost:3578".to_string(),
            data_dir: "/tmp/nano".to_string(),
        };

        let json = serde_json::to_string(&config).unwrap();
        let parsed: config::NanoConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.algod_url, "http://localhost:4001");
        assert_eq!(parsed.agent_name, "nano");
    }
}
