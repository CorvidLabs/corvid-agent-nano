//! Corvid Agent Nano — lightweight Rust AlgoChat agent.
//!
//! Subcommands: init, import, run, contacts, change-password, info

use std::fmt;
use std::sync::Arc;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand, ValueEnum};
use ed25519_dalek::SigningKey;
use tracing::info;
use zeroize::Zeroize;

use algochat::{AlgoChat, AlgoChatConfig, AlgorandConfig};
use corvid_core::storage::{SqliteKeyStorage, SqliteMessageCache};

mod agent;
mod algorand;
mod contacts;
mod keystore;
mod transaction;
mod wallet;

use algorand::{HttpAlgodClient, HttpIndexerClient};
use contacts::ContactStore;

// ---------------------------------------------------------------------------
// Network presets
// ---------------------------------------------------------------------------

/// Algorand network presets.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum Network {
    /// Local sandbox (default) — localhost:4001/8980
    Localnet,
    /// Algorand TestNet via Nodely public API
    Testnet,
    /// Algorand MainNet via Nodely public API
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

// ---------------------------------------------------------------------------
// CLI structure
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(
    name = "nano",
    about = "Corvid Agent Nano — lightweight Rust AlgoChat agent"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Data directory for persistent storage
    #[arg(long, default_value = "./data", global = true)]
    data_dir: String,
}

#[derive(Subcommand)]
enum Command {
    /// Generate a new wallet and save it encrypted
    Init {
        /// Algorand network preset
        #[arg(long, default_value = "localnet", env = "NANO_NETWORK")]
        network: Network,

        /// Password for keystore encryption (min 8 chars).
        /// If not provided, prompts interactively.
        #[arg(long, env = "NANO_PASSWORD")]
        password: Option<String>,
    },

    /// Import an existing wallet from mnemonic or hex seed
    Import {
        /// 25-word Algorand mnemonic
        #[arg(long, conflicts_with = "seed")]
        mnemonic: Option<String>,

        /// Hex-encoded 32-byte Ed25519 seed
        #[arg(long, conflicts_with = "mnemonic")]
        seed: Option<String>,

        /// Password for keystore encryption
        #[arg(long, env = "NANO_PASSWORD")]
        password: Option<String>,
    },

    /// Start the agent and listen for AlgoChat messages
    Run {
        /// Algorand network preset
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

        /// Agent seed (hex). If not provided, loads from keystore.
        #[arg(long, env = "NANO_SEED")]
        seed: Option<String>,

        /// Agent Algorand address. Required if --seed is provided.
        #[arg(long, env = "NANO_ADDRESS")]
        address: Option<String>,

        /// Keystore password (for loading from keystore)
        #[arg(long, env = "NANO_PASSWORD")]
        password: Option<String>,

        /// Agent name for discovery
        #[arg(long, default_value = "nano")]
        name: String,

        /// corvid-agent hub URL
        #[arg(long, default_value = "http://localhost:3578")]
        hub_url: String,

        /// Poll interval in seconds
        #[arg(long, default_value = "5")]
        poll_interval: u64,
    },

    /// Manage contacts
    Contacts {
        #[command(subcommand)]
        action: ContactsAction,
    },

    /// Change the keystore password
    ChangePassword {
        /// Current password
        #[arg(long, env = "NANO_PASSWORD")]
        old_password: Option<String>,

        /// New password
        #[arg(long)]
        new_password: Option<String>,
    },

    /// Show agent identity and status
    Info,
}

#[derive(Subcommand)]
enum ContactsAction {
    /// List all contacts
    List,

    /// Add a new contact
    Add {
        /// Contact name
        #[arg(long)]
        name: String,

        /// Algorand address
        #[arg(long)]
        address: String,

        /// Pre-shared key (hex or base64)
        #[arg(long)]
        psk: String,

        /// Overwrite if contact exists
        #[arg(long)]
        force: bool,
    },

    /// Remove a contact
    Remove {
        /// Contact name
        name: String,
    },

    /// Export contacts as JSON
    Export {
        /// Output file (stdout if not specified)
        #[arg(long)]
        output: Option<String>,
    },

    /// Import contacts from JSON
    Import {
        /// Input file
        file: String,
    },
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn keystore_path(data_dir: &str) -> std::path::PathBuf {
    std::path::Path::new(data_dir).join("keystore.enc")
}

fn contacts_db_path(data_dir: &str) -> std::path::PathBuf {
    std::path::Path::new(data_dir).join("contacts.db")
}

/// Prompt for a password interactively (no echo).
fn prompt_password(prompt: &str) -> Result<String> {
    rpassword::prompt_password(prompt).map_err(|e| anyhow::anyhow!("Password prompt failed: {}", e))
}

/// Prompt for a new password with confirmation.
fn prompt_new_password() -> Result<String> {
    loop {
        let p1 = prompt_password("Enter a password to encrypt your wallet: ")?;
        if p1.len() < 8 {
            eprintln!("Password must be at least 8 characters. Try again.");
            continue;
        }
        let p2 = prompt_password("Confirm password: ")?;
        if p1 != p2 {
            eprintln!("Passwords don't match. Try again.");
            continue;
        }
        return Ok(p1);
    }
}

/// Load seed + address, either from CLI flags or from the encrypted keystore.
fn load_identity(
    seed_hex: Option<&str>,
    address: Option<&str>,
    password: Option<&str>,
    data_dir: &str,
) -> Result<([u8; 32], String)> {
    if let Some(seed_str) = seed_hex {
        // Direct seed from CLI/env
        let seed_bytes =
            hex::decode(seed_str).map_err(|e| anyhow::anyhow!("Invalid seed hex: {}", e))?;
        if seed_bytes.len() != 32 {
            bail!(
                "Seed must be 32 bytes (64 hex chars), got {}",
                seed_bytes.len()
            );
        }
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&seed_bytes);

        let addr = match address {
            Some(a) => a.to_string(),
            None => wallet::address_from_seed(&seed),
        };

        return Ok((seed, addr));
    }

    // Load from keystore
    let ks_path = keystore_path(data_dir);
    if !keystore::keystore_exists(&ks_path) {
        bail!("No wallet found. Run `nano init` to create one, or provide --seed/--address.");
    }

    let pw = match password {
        Some(p) => p.to_string(),
        None => prompt_password("Enter wallet password: ")?,
    };

    let (seed, addr) = keystore::load_keystore(&ks_path, &pw)?;
    Ok((seed, addr))
}

// ---------------------------------------------------------------------------
// Command handlers
// ---------------------------------------------------------------------------

fn cmd_init(network: Network, password: Option<String>, data_dir: &str) -> Result<()> {
    let data_path = std::path::Path::new(data_dir);
    std::fs::create_dir_all(data_path)?;

    let ks_path = keystore_path(data_dir);
    if keystore::keystore_exists(&ks_path) {
        bail!(
            "Wallet already exists at {}. Delete it first or use `nano import`.",
            ks_path.display()
        );
    }

    // Generate wallet
    let mut seed = wallet::generate_seed();
    let address = wallet::address_from_seed(&seed);
    let mnemonic = wallet::seed_to_mnemonic(&seed);

    println!("\nGenerated new Algorand wallet");
    println!("  Network: {}", network);
    println!("  Address: {}\n", address);

    println!("IMPORTANT: Write down your recovery phrase and store it safely.");
    println!("---");
    let words: Vec<&str> = mnemonic.split_whitespace().collect();
    for (i, word) in words.iter().enumerate() {
        print!("{:>2}. {:<12}", i + 1, word);
        if (i + 1) % 5 == 0 {
            println!();
        }
    }
    println!("---\n");

    // Get password
    let pw = match password {
        Some(p) => {
            if p.len() < 8 {
                bail!("Password must be at least 8 characters");
            }
            p
        }
        None => prompt_new_password()?,
    };

    // Encrypt and save
    keystore::create_keystore(&seed, &address, &pw, &ks_path)?;
    seed.zeroize();

    println!("\nWallet encrypted and saved to {}", ks_path.display());

    // Network-specific funding instructions
    match network {
        Network::Testnet => {
            println!("\nFund your agent:");
            println!("  Send ALGO to: {}", address);
            println!("  Testnet dispenser: https://bank.testnet.algorand.network");
            println!("\nOnce funded, run: nano run --network testnet");
        }
        Network::Mainnet => {
            println!("\nFund your agent:");
            println!("  Send ALGO to: {}", address);
            println!("\nOnce funded, run: nano run --network mainnet");
        }
        Network::Localnet => {
            println!("\nRun your agent: nano run --network localnet");
        }
    }

    Ok(())
}

fn cmd_import(
    mnemonic: Option<String>,
    seed_hex: Option<String>,
    password: Option<String>,
    data_dir: &str,
) -> Result<()> {
    let data_path = std::path::Path::new(data_dir);
    std::fs::create_dir_all(data_path)?;

    let ks_path = keystore_path(data_dir);
    if keystore::keystore_exists(&ks_path) {
        bail!(
            "Wallet already exists at {}. Delete it first to reimport.",
            ks_path.display()
        );
    }

    let mut seed = if let Some(m) = mnemonic {
        wallet::mnemonic_to_seed(&m)?
    } else if let Some(s) = seed_hex {
        let bytes = hex::decode(&s).map_err(|e| anyhow::anyhow!("Invalid hex: {}", e))?;
        if bytes.len() != 32 {
            bail!("Seed must be 32 bytes (64 hex chars), got {}", bytes.len());
        }
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&bytes);
        seed
    } else {
        bail!("Provide either --mnemonic or --seed");
    };

    let address = wallet::address_from_seed(&seed);
    println!("Imported wallet");
    println!("  Address: {}", address);

    let pw = match password {
        Some(p) => {
            if p.len() < 8 {
                bail!("Password must be at least 8 characters");
            }
            p
        }
        None => prompt_new_password()?,
    };

    keystore::create_keystore(&seed, &address, &pw, &ks_path)?;
    seed.zeroize();

    println!("Wallet encrypted and saved to {}", ks_path.display());
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn cmd_run(
    network: Network,
    algod_url: Option<String>,
    algod_token: Option<String>,
    indexer_url: Option<String>,
    indexer_token: Option<String>,
    seed_hex: Option<String>,
    address: Option<String>,
    password: Option<String>,
    name: String,
    hub_url: String,
    poll_interval: u64,
    data_dir: &str,
) -> Result<()> {
    // Resolve network config
    let net = network.defaults();
    let algod_url = algod_url.unwrap_or(net.algod_url);
    let algod_token = algod_token.unwrap_or(net.algod_token);
    let indexer_url = indexer_url.unwrap_or(net.indexer_url);
    let indexer_token = indexer_token.unwrap_or(net.indexer_token);

    // Load identity
    let (seed, agent_address) = load_identity(
        seed_hex.as_deref(),
        address.as_deref(),
        password.as_deref(),
        data_dir,
    )?;

    info!(
        name = %name,
        network = %network,
        algod = %algod_url,
        indexer = %indexer_url,
        hub = %hub_url,
        address = %agent_address,
        "starting corvid-agent-nano"
    );

    let signing_key = SigningKey::from_bytes(&seed);

    // Ensure data directory exists
    let data_path = std::path::Path::new(data_dir);
    std::fs::create_dir_all(data_path)?;

    // Build Algorand clients
    let algod = HttpAlgodClient::new(&algod_url, &algod_token);
    let indexer = HttpIndexerClient::new(&indexer_url, &indexer_token);

    let algo_config =
        AlgorandConfig::new(&algod_url, &algod_token).with_indexer(&indexer_url, &indexer_token);
    let config = AlgoChatConfig::new(algo_config);

    // Initialize persistent SQLite storage
    let key_storage = SqliteKeyStorage::open(data_path.join("keys.db"))
        .map_err(|e| anyhow::anyhow!("Failed to open key storage: {}", e))?;
    let message_cache = SqliteMessageCache::open(data_path.join("messages.db"))
        .map_err(|e| anyhow::anyhow!("Failed to open message cache: {}", e))?;

    info!(data_dir = %data_dir, "persistent storage initialized");

    // Load contacts and register PSKs with AlgoChat
    let contacts_path = contacts_db_path(data_dir);
    let contact_store = if contacts_path.exists() {
        Some(ContactStore::open(&contacts_path)?)
    } else {
        None
    };

    // Initialize AlgoChat client
    let client = AlgoChat::from_seed(
        &seed,
        &agent_address,
        config,
        algod,
        indexer,
        key_storage,
        message_cache,
    )
    .await
    .map_err(|e| anyhow::anyhow!("Failed to initialize AlgoChat: {}", e))?;

    // Register PSK contacts
    if let Some(store) = &contact_store {
        let contacts = store.list()?;
        for contact in &contacts {
            let mut psk = [0u8; 32];
            psk.copy_from_slice(&contact.psk);
            if let Err(e) = client
                .add_psk_contact(&contact.address, &psk, Some(contact.name.clone()))
                .await
            {
                tracing::warn!(
                    name = %contact.name,
                    error = %e,
                    "failed to register PSK contact"
                );
            } else {
                info!(name = %contact.name, address = %contact.address, "registered PSK contact");
            }
        }
    }

    let pub_key = hex::encode(client.encryption_public_key());
    info!(
        address = %agent_address,
        encryption_key = %pub_key,
        "identity initialized"
    );

    // Print startup summary
    let contact_count = contact_store
        .as_ref()
        .map(|s| s.count().unwrap_or(0))
        .unwrap_or(0);
    println!("\n  Agent:    {}", name);
    println!("  Network:  {}", network);
    println!("  Address:  {}", agent_address);
    println!("  Enc Key:  {}", &pub_key[..16]);
    println!("  Contacts: {}", contact_count);
    println!("  Hub:      {}\n", hub_url);

    let client = Arc::new(client);

    let algod_for_tx = Arc::new(HttpAlgodClient::new(&algod_url, &algod_token));

    let loop_client = Arc::clone(&client);
    let loop_algod = Arc::clone(&algod_for_tx);
    let loop_config = agent::AgentLoopConfig {
        poll_interval_secs: poll_interval,
        hub_url: hub_url.clone(),
        agent_name: name.clone(),
        agent_address: agent_address.clone(),
        signing_key,
    };

    let message_task = tokio::spawn(async move {
        agent::run_message_loop(loop_client, loop_algod, loop_config).await;
    });

    info!("nano agent ready — listening for AlgoChat messages");

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

fn cmd_contacts(action: ContactsAction, data_dir: &str) -> Result<()> {
    let data_path = std::path::Path::new(data_dir);
    std::fs::create_dir_all(data_path)?;

    let store = ContactStore::open(contacts_db_path(data_dir))?;

    match action {
        ContactsAction::List => {
            let contacts = store.list()?;
            if contacts.is_empty() {
                println!("No contacts. Add one with: nano contacts add --name <name> --address <addr> --psk <key>");
                return Ok(());
            }
            println!("{:<16} {:<60} ADDED", "NAME", "ADDRESS");
            println!("{}", "-".repeat(90));
            for c in &contacts {
                println!("{:<16} {:<60} {}", c.name, c.address, c.added_at);
            }
            println!("\n{} contact(s)", contacts.len());
        }

        ContactsAction::Add {
            name,
            address,
            psk,
            force,
        } => {
            // Validate address format
            wallet::decode_address(&address)?;
            let psk_bytes = contacts::parse_psk(&psk)?;

            if force {
                store.upsert(&name, &address, &psk_bytes)?;
            } else {
                store.add(&name, &address, &psk_bytes)?;
            }
            println!("Added contact: {} ({})", name, address);
        }

        ContactsAction::Remove { name } => {
            if store.remove(&name)? {
                println!("Removed contact: {}", name);
            } else {
                println!("Contact \"{}\" not found", name);
            }
        }

        ContactsAction::Export { output } => {
            let json = store.export_json()?;
            if let Some(path) = output {
                std::fs::write(&path, &json)?;
                println!("Exported {} contact(s) to {}", store.count()?, path);
            } else {
                println!("{}", json);
            }
        }

        ContactsAction::Import { file } => {
            let json = std::fs::read_to_string(&file)?;
            let count = store.import_json(&json)?;
            println!("Imported {} contact(s) from {}", count, file);
        }
    }

    Ok(())
}

fn cmd_change_password(
    old_password: Option<String>,
    new_password: Option<String>,
    data_dir: &str,
) -> Result<()> {
    let ks_path = keystore_path(data_dir);
    if !keystore::keystore_exists(&ks_path) {
        bail!("No wallet found. Run `nano init` first.");
    }

    let old_pw = match old_password {
        Some(p) => p,
        None => prompt_password("Enter current password: ")?,
    };

    let (mut seed, address) = keystore::load_keystore(&ks_path, &old_pw)?;

    let new_pw = match new_password {
        Some(p) => {
            if p.len() < 8 {
                bail!("Password must be at least 8 characters");
            }
            p
        }
        None => prompt_new_password()?,
    };

    keystore::create_keystore(&seed, &address, &new_pw, &ks_path)?;
    seed.zeroize();

    println!("Password changed successfully.");
    Ok(())
}

fn cmd_info(data_dir: &str) -> Result<()> {
    let ks_path = keystore_path(data_dir);

    if !keystore::keystore_exists(&ks_path) {
        println!("No wallet configured.");
        println!("Run `nano init` to create a new wallet.");
        return Ok(());
    }

    let address = keystore::keystore_address(&ks_path)?;
    println!("Corvid Agent Nano");
    println!("  Wallet:   {}", ks_path.display());
    println!("  Address:  {}", address);

    // Show contact count if contacts DB exists
    let contacts_path = contacts_db_path(data_dir);
    if contacts_path.exists() {
        let store = ContactStore::open(&contacts_path)?;
        println!("  Contacts: {}", store.count()?);
    } else {
        println!("  Contacts: 0");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();
    let data_dir = &cli.data_dir;

    match cli.command {
        Command::Init { network, password } => cmd_init(network, password, data_dir),

        Command::Import {
            mnemonic,
            seed,
            password,
        } => cmd_import(mnemonic, seed, password, data_dir),

        Command::Run {
            network,
            algod_url,
            algod_token,
            indexer_url,
            indexer_token,
            seed,
            address,
            password,
            name,
            hub_url,
            poll_interval,
        } => {
            cmd_run(
                network,
                algod_url,
                algod_token,
                indexer_url,
                indexer_token,
                seed,
                address,
                password,
                name,
                hub_url,
                poll_interval,
                data_dir,
            )
            .await
        }

        Command::Contacts { action } => cmd_contacts(action, data_dir),

        Command::ChangePassword {
            old_password,
            new_password,
        } => cmd_change_password(old_password, new_password, data_dir),

        Command::Info => cmd_info(data_dir),
    }
}
