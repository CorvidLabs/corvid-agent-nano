use std::fmt;
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, ValueEnum};
use ed25519_dalek::SigningKey;
use tracing::info;

use algochat::{AlgoChat, AlgoChatConfig, AlgorandConfig};
use corvid_core::storage::{SqliteKeyStorage, SqliteMessageCache};

mod agent;
mod algorand;
mod transaction;

use algorand::{HttpAlgodClient, HttpIndexerClient};

/// Algorand network presets.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum Network {
    /// Local sandbox (default) — localhost:4001/8980
    Localnet,
    /// Algorand TestNet via Algonode public API
    Testnet,
    /// Algorand MainNet via Algonode public API
    Mainnet,
}

impl fmt::Display for Network {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Network::Localnet => write!(f, "localnet"),
            Network::Testnet => write!(f, "testnet"),
            Network::Mainnet => write!(f, "mainnet"),
        }
    }
}

/// Resolved URLs and tokens for an Algorand network.
struct NetworkConfig {
    algod_url: String,
    algod_token: String,
    indexer_url: String,
    indexer_token: String,
}

impl Network {
    fn defaults(self) -> NetworkConfig {
        match self {
            Network::Localnet => NetworkConfig {
                algod_url: "http://localhost:4001".into(),
                algod_token: "a".repeat(64),
                indexer_url: "http://localhost:8980".into(),
                indexer_token: "a".repeat(64),
            },
            Network::Testnet => NetworkConfig {
                algod_url: "https://testnet-api.4160.nodely.dev".into(),
                algod_token: String::new(),
                indexer_url: "https://testnet-idx.4160.nodely.dev".into(),
                indexer_token: String::new(),
            },
            Network::Mainnet => NetworkConfig {
                algod_url: "https://mainnet-api.4160.nodely.dev".into(),
                algod_token: String::new(),
                indexer_url: "https://mainnet-idx.4160.nodely.dev".into(),
                indexer_token: String::new(),
            },
        }
    }
}

#[derive(Parser)]
#[command(name = "nano", about = "Corvid Agent Nano — lightweight Rust agent")]
struct Cli {
    /// Algorand network preset (localnet, testnet, mainnet)
    #[arg(long, default_value = "localnet", env = "NANO_NETWORK")]
    network: Network,

    /// Override: Algorand node URL
    #[arg(long, env = "NANO_ALGOD_URL")]
    algod_url: Option<String>,

    /// Override: Algorand node token
    #[arg(long, env = "NANO_ALGOD_TOKEN")]
    algod_token: Option<String>,

    /// Override: Algorand indexer URL
    #[arg(long, env = "NANO_INDEXER_URL")]
    indexer_url: Option<String>,

    /// Override: Algorand indexer token
    #[arg(long, env = "NANO_INDEXER_TOKEN")]
    indexer_token: Option<String>,

    /// Agent seed (hex-encoded 32-byte Ed25519 private key)
    #[arg(long, env = "NANO_SEED")]
    seed: String,

    /// Agent Algorand address
    #[arg(long, env = "NANO_ADDRESS")]
    address: String,

    /// Agent name for discovery
    #[arg(long, default_value = "nano")]
    name: String,

    /// corvid-agent hub URL (for API communication)
    #[arg(long, default_value = "http://localhost:3578")]
    hub_url: String,

    /// Data directory for persistent storage
    #[arg(long, default_value = "./data")]
    data_dir: String,

    /// Poll interval in seconds
    #[arg(long, default_value = "5")]
    poll_interval: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();

    // Resolve network config: preset defaults + CLI overrides
    let net = cli.network.defaults();
    let algod_url = cli.algod_url.unwrap_or(net.algod_url);
    let algod_token = cli.algod_token.unwrap_or(net.algod_token);
    let indexer_url = cli.indexer_url.unwrap_or(net.indexer_url);
    let indexer_token = cli.indexer_token.unwrap_or(net.indexer_token);

    info!(
        name = %cli.name,
        network = %cli.network,
        algod = %algod_url,
        indexer = %indexer_url,
        hub = %cli.hub_url,
        "starting corvid-agent-nano"
    );

    // Parse seed from hex
    let seed_bytes =
        hex::decode(&cli.seed).map_err(|e| anyhow::anyhow!("Invalid seed hex: {}", e))?;
    if seed_bytes.len() != 32 {
        anyhow::bail!(
            "Seed must be exactly 32 bytes (64 hex chars), got {}",
            seed_bytes.len()
        );
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&seed_bytes);

    // Derive Ed25519 signing key from seed
    let signing_key = SigningKey::from_bytes(&seed);

    // Ensure data directory exists
    let data_dir = std::path::Path::new(&cli.data_dir);
    std::fs::create_dir_all(data_dir)?;

    // Build Algorand clients
    let algod = HttpAlgodClient::new(&algod_url, &algod_token);
    let indexer = HttpIndexerClient::new(&indexer_url, &indexer_token);

    // Build AlgoChat config
    let network =
        AlgorandConfig::new(&algod_url, &algod_token).with_indexer(&indexer_url, &indexer_token);
    let config = AlgoChatConfig::new(network);

    // Initialize persistent SQLite storage
    let key_storage = SqliteKeyStorage::open(data_dir.join("keys.db"))
        .map_err(|e| anyhow::anyhow!("Failed to open key storage: {}", e))?;
    let message_cache = SqliteMessageCache::open(data_dir.join("messages.db"))
        .map_err(|e| anyhow::anyhow!("Failed to open message cache: {}", e))?;

    info!(data_dir = %cli.data_dir, "persistent storage initialized");

    // Initialize AlgoChat client
    let client = AlgoChat::from_seed(
        &seed,
        &cli.address,
        config,
        algod,
        indexer,
        key_storage,
        message_cache,
    )
    .await
    .map_err(|e| anyhow::anyhow!("Failed to initialize AlgoChat: {}", e))?;

    let pub_key = hex::encode(client.encryption_public_key());
    info!(
        address = %cli.address,
        encryption_key = %pub_key,
        "identity initialized"
    );

    let client = Arc::new(client);

    // Build a separate algod client for transaction submission (the first one
    // was moved into AlgoChat). This is cheap — just wraps a reqwest::Client.
    let algod_for_tx = Arc::new(HttpAlgodClient::new(&algod_url, &algod_token));

    // Start the message polling loop in a background task
    let loop_client = Arc::clone(&client);
    let loop_algod = Arc::clone(&algod_for_tx);
    let loop_config = agent::AgentLoopConfig {
        poll_interval_secs: cli.poll_interval,
        hub_url: cli.hub_url.clone(),
        agent_name: cli.name.clone(),
        agent_address: cli.address.clone(),
        signing_key,
    };

    let message_task = tokio::spawn(async move {
        agent::run_message_loop(loop_client, loop_algod, loop_config).await;
    });

    info!("nano agent ready — listening for AlgoChat messages");

    // Wait for Ctrl+C or task failure
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("shutting down (ctrl+c)");
        }
        result = message_task => {
            match result {
                Ok(()) => info!("message loop ended"),
                Err(e) => tracing::error!(error = %e, "message loop panicked"),
            }
        }
    }

    Ok(())
}
