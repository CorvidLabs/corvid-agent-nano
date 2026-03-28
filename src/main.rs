use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use tracing::info;

use algochat::{AlgoChat, AlgoChatConfig, AlgorandConfig};
use corvid_core::storage::{SqliteKeyStorage, SqliteMessageCache};

mod agent;
mod algorand;

use algorand::{HttpAlgodClient, HttpIndexerClient};

#[derive(Parser)]
#[command(name = "nano", about = "Corvid Agent Nano — lightweight Rust agent")]
struct Cli {
    /// Algorand node URL (default: localnet)
    #[arg(long, default_value = "http://localhost:4001")]
    algod_url: String,

    /// Algorand node token
    #[arg(
        long,
        default_value = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    )]
    algod_token: String,

    /// Algorand indexer URL
    #[arg(long, default_value = "http://localhost:8980")]
    indexer_url: String,

    /// Algorand indexer token
    #[arg(
        long,
        default_value = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    )]
    indexer_token: String,

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

    info!(
        name = %cli.name,
        algod = %cli.algod_url,
        indexer = %cli.indexer_url,
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

    // Ensure data directory exists
    let data_dir = std::path::Path::new(&cli.data_dir);
    std::fs::create_dir_all(data_dir)?;

    // Build Algorand clients
    let algod = HttpAlgodClient::new(&cli.algod_url, &cli.algod_token);
    let indexer = HttpIndexerClient::new(&cli.indexer_url, &cli.indexer_token);

    // Build AlgoChat config
    let network = AlgorandConfig::new(&cli.algod_url, &cli.algod_token)
        .with_indexer(&cli.indexer_url, &cli.indexer_token);
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

    // Start the message polling loop in a background task
    let loop_client = Arc::clone(&client);
    let loop_config = agent::AgentLoopConfig {
        poll_interval_secs: cli.poll_interval,
        hub_url: cli.hub_url.clone(),
        agent_name: cli.name.clone(),
    };

    let message_task = tokio::spawn(async move {
        agent::run_message_loop(loop_client, loop_config).await;
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
