use anyhow::Result;
use clap::Parser;
use tracing::info;

#[derive(Parser)]
#[command(name = "nano", about = "Corvid Agent Nano — lightweight Rust agent")]
struct Cli {
    /// Algorand node URL (default: localnet)
    #[arg(long, default_value = "http://localhost:4001")]
    algod_url: String,

    /// Algorand node token
    #[arg(long, default_value = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")]
    algod_token: String,

    /// Agent name for discovery
    #[arg(long, default_value = "nano")]
    name: String,

    /// corvid-agent hub URL (for API communication)
    #[arg(long, default_value = "http://localhost:3578")]
    hub_url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();

    info!(
        name = %cli.name,
        algod = %cli.algod_url,
        hub = %cli.hub_url,
        "starting corvid-agent-nano"
    );

    // TODO: Initialize crypto identity (X25519 keypair)
    // TODO: Connect to Algorand node
    // TODO: Register in Flock Directory
    // TODO: Start AlgoChat message loop
    // TODO: Connect to hub API

    info!("nano agent ready — waiting for messages");

    // Keep running
    tokio::signal::ctrl_c().await?;
    info!("shutting down");

    Ok(())
}
