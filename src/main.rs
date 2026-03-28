use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use ed25519_dalek::SigningKey;
use tracing::info;
use zeroize::Zeroize;

use algochat::{AlgoChat, AlgoChatConfig, AlgorandConfig, InMemoryKeyStorage, InMemoryMessageCache};

mod agent;
mod algorand;
mod hub;
mod identity;
mod transaction;
mod vault;

use algorand::{HttpAlgodClient, HttpIndexerClient};
use hub::HubClient;
use vault::{Contact, Vault, VaultContents};

/// Default vault file path.
fn default_vault_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".nano")
        .join("vault.enc")
}

#[derive(Parser)]
#[command(name = "can", about = "Corvid Agent Nano (CAN) — lightweight Rust agent")]
struct Cli {
    /// Path to vault file
    #[arg(long, global = true)]
    vault: Option<std::path::PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize a new identity and vault
    Init,

    /// Run the agent
    Run {
        /// Algorand node URL (default: localnet)
        #[arg(long, default_value = "http://localhost:4001")]
        algod_url: String,

        /// Algorand node token
        #[arg(long, default_value = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")]
        algod_token: String,

        /// Algorand indexer URL
        #[arg(long, default_value = "http://localhost:8980")]
        indexer_url: String,

        /// Algorand indexer token
        #[arg(long, default_value = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")]
        indexer_token: String,

        /// Agent name for discovery
        #[arg(long, default_value = "nano")]
        name: String,

        /// corvid-agent hub URL
        #[arg(long, default_value = "http://localhost:3578")]
        hub_url: String,

        /// Poll interval in seconds
        #[arg(long, default_value = "5")]
        poll_interval: u64,

        /// Skip hub registration (for offline/testing mode)
        #[arg(long, default_value = "false")]
        no_hub: bool,
    },

    /// Add a contact with PSK
    AddContact {
        /// Contact name (e.g. "corvid-agent")
        #[arg(long)]
        name: String,

        /// Contact's Algorand address
        #[arg(long)]
        address: String,

        /// PSK as base64 string (reads from stdin if omitted)
        #[arg(long)]
        psk: Option<String>,

        /// Read PSK from a file (deleted after import is YOUR responsibility)
        #[arg(long)]
        psk_file: Option<std::path::PathBuf>,
    },

    /// Remove a contact
    RemoveContact {
        /// Contact name to remove
        name: String,
    },

    /// Show identity (public info only — never prints secrets)
    ShowIdentity,

    /// List contacts (names and addresses only — never prints PSKs)
    ListContacts,
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
    let vault_path = cli.vault.unwrap_or_else(default_vault_path);

    match cli.command {
        Command::Init => cmd_init(&vault_path)?,
        Command::Run {
            algod_url,
            algod_token,
            indexer_url,
            indexer_token,
            name,
            hub_url,
            poll_interval,
            no_hub,
        } => {
            cmd_run(
                &vault_path,
                &algod_url,
                &algod_token,
                &indexer_url,
                &indexer_token,
                &name,
                &hub_url,
                poll_interval,
                no_hub,
            )
            .await?;
        }
        Command::AddContact {
            name,
            address,
            psk,
            psk_file,
        } => cmd_add_contact(&vault_path, &name, &address, psk, psk_file)?,
        Command::RemoveContact { name } => cmd_remove_contact(&vault_path, &name)?,
        Command::ShowIdentity => cmd_show_identity(&vault_path)?,
        Command::ListContacts => cmd_list_contacts(&vault_path)?,
    }

    Ok(())
}

/// Initialize a new vault with a freshly generated identity.
fn cmd_init(vault_path: &std::path::Path) -> Result<()> {
    if Vault::exists(vault_path) {
        anyhow::bail!(
            "Vault already exists at {}. Delete it first if you want to start fresh.",
            vault_path.display()
        );
    }

    println!("Creating new nano identity...");

    let passphrase = prompt_new_passphrase()?;
    let seed = identity::generate_seed();
    let address = identity::address_from_seed(&seed);
    let seed_hex = hex::encode(seed);

    let contents = VaultContents {
        seed_hex,
        address: address.clone(),
        contacts: vec![],
    };

    Vault::create(vault_path, &contents, &passphrase)?;

    println!("\nIdentity created successfully!");
    println!("Address: {}", address);
    println!("Vault:   {}", vault_path.display());
    println!("\nYour seed is encrypted in the vault. Keep your passphrase safe!");
    println!("Fund this address on localnet before running the agent.");

    Ok(())
}

/// Run the agent with secrets loaded from the vault.
#[allow(clippy::too_many_arguments)]
async fn cmd_run(
    vault_path: &std::path::Path,
    algod_url: &str,
    algod_token: &str,
    indexer_url: &str,
    indexer_token: &str,
    name: &str,
    hub_url: &str,
    poll_interval: u64,
    no_hub: bool,
) -> Result<()> {
    if !Vault::exists(vault_path) {
        anyhow::bail!("No vault found. Run `can init` first.");
    }

    let passphrase = prompt_passphrase("Enter vault passphrase: ")?;
    let mut contents = Vault::open(vault_path, &passphrase).context("Failed to open vault")?;

    info!(
        name = %name,
        algod = %algod_url,
        indexer = %indexer_url,
        hub = %hub_url,
        "starting corvid-agent-nano"
    );

    // Parse seed from vault
    let seed_bytes = hex::decode(&contents.seed_hex)
        .map_err(|e| anyhow::anyhow!("Invalid seed in vault: {}", e))?;
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&seed_bytes);

    // Derive the Ed25519 signing key
    let signing_key = SigningKey::from_bytes(&seed);
    let address = contents.address.clone();

    // Zeroize vault contents — we've extracted what we need
    contents.zeroize();

    // Build Algorand clients
    let algod = Arc::new(HttpAlgodClient::new(algod_url, algod_token));
    let indexer = HttpIndexerClient::new(indexer_url, indexer_token);

    // Build AlgoChat config
    let network = AlgorandConfig::new(algod_url, algod_token)
        .with_indexer(indexer_url, indexer_token);
    let config = AlgoChatConfig::new(network);

    // Initialize AlgoChat client
    let client = AlgoChat::from_seed(
        &seed,
        &address,
        config,
        HttpAlgodClient::new(algod_url, algod_token),
        indexer,
        InMemoryKeyStorage::new(),
        InMemoryMessageCache::new(),
    )
    .await
    .map_err(|e| anyhow::anyhow!("Failed to initialize AlgoChat: {}", e))?;

    // Zeroize the seed copy
    seed.zeroize();

    let pub_key = hex::encode(client.encryption_public_key());
    info!(
        address = %address,
        encryption_key = %pub_key,
        "identity initialized"
    );

    let client = Arc::new(client);

    // Register with hub (Flock Directory)
    let mut hub = HubClient::new(hub_url);
    if !no_hub {
        match hub.register(&address, name, &pub_key).await {
            Ok(id) => info!(agent_id = %id, "registered with hub"),
            Err(e) => {
                tracing::warn!(error = %e, "hub registration failed — running without hub");
            }
        }
    }
    let hub = Arc::new(hub);

    // Start the message polling loop
    let loop_config = agent::AgentLoopConfig {
        poll_interval_secs: poll_interval,
        hub_url: hub_url.to_string(),
        agent_name: name.to_string(),
        address: address.clone(),
    };

    let message_task = tokio::spawn(async move {
        agent::run_message_loop(client, algod, signing_key, hub, loop_config).await;
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

/// Add a contact to the vault.
fn cmd_add_contact(
    vault_path: &std::path::Path,
    name: &str,
    address: &str,
    psk_arg: Option<String>,
    psk_file: Option<std::path::PathBuf>,
) -> Result<()> {
    if !Vault::exists(vault_path) {
        anyhow::bail!("No vault found. Run `can init` first.");
    }

    // Get PSK from argument, file, or interactive prompt
    let psk_b64 = if let Some(psk) = psk_arg {
        psk
    } else if let Some(path) = psk_file {
        std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read PSK file: {}", path.display()))?
            .trim()
            .to_string()
    } else {
        prompt_passphrase(&format!("Enter PSK for {} (base64): ", name))?
    };

    // Validate PSK is valid base64
    let psk_bytes = data_encoding::BASE64
        .decode(psk_b64.trim().as_bytes())
        .context("Invalid base64 PSK")?;

    let passphrase = prompt_passphrase("Enter vault passphrase: ")?;

    Vault::update(vault_path, &passphrase, |contents| {
        // Remove existing contact with same name (update semantics)
        contents.contacts.retain(|c| c.name != name);
        contents.contacts.push(Contact {
            name: name.to_string(),
            address: address.to_string(),
            psk: psk_bytes,
        });
    })?;

    println!("Contact '{}' added to vault.", name);
    Ok(())
}

/// Remove a contact from the vault.
fn cmd_remove_contact(vault_path: &std::path::Path, name: &str) -> Result<()> {
    if !Vault::exists(vault_path) {
        anyhow::bail!("No vault found. Run `can init` first.");
    }

    let passphrase = prompt_passphrase("Enter vault passphrase: ")?;

    let mut found = false;
    Vault::update(vault_path, &passphrase, |contents| {
        let before = contents.contacts.len();
        contents.contacts.retain(|c| c.name != name);
        found = contents.contacts.len() < before;
    })?;

    if found {
        println!("Contact '{}' removed.", name);
    } else {
        println!("Contact '{}' not found.", name);
    }
    Ok(())
}

/// Show public identity info (never prints secrets).
fn cmd_show_identity(vault_path: &std::path::Path) -> Result<()> {
    if !Vault::exists(vault_path) {
        anyhow::bail!("No vault found. Run `can init` first.");
    }

    let passphrase = prompt_passphrase("Enter vault passphrase: ")?;
    let contents = Vault::open(vault_path, &passphrase)?;

    println!("Address: {}", contents.address);
    println!("Vault:   {}", vault_path.display());
    println!("Contacts: {}", contents.contacts.len());

    Ok(())
}

/// List contacts (names and addresses only — never prints PSKs).
fn cmd_list_contacts(vault_path: &std::path::Path) -> Result<()> {
    if !Vault::exists(vault_path) {
        anyhow::bail!("No vault found. Run `can init` first.");
    }

    let passphrase = prompt_passphrase("Enter vault passphrase: ")?;
    let contents = Vault::open(vault_path, &passphrase)?;

    if contents.contacts.is_empty() {
        println!("No contacts configured.");
    } else {
        println!("{:<20} ADDRESS", "NAME");
        println!("{:<20} -------", "----");
        for contact in &contents.contacts {
            println!("{:<20} {}", contact.name, contact.address);
        }
    }

    Ok(())
}

/// Prompt for a passphrase (no echo).
fn prompt_passphrase(prompt: &str) -> Result<String> {
    rpassword::prompt_password(prompt).context("Failed to read passphrase")
}

/// Prompt for a new passphrase with confirmation.
fn prompt_new_passphrase() -> Result<String> {
    loop {
        let pass1 = prompt_passphrase("Set vault passphrase: ")?;
        if pass1.is_empty() {
            println!("Passphrase cannot be empty. Try again.");
            continue;
        }
        let pass2 = prompt_passphrase("Confirm passphrase: ")?;
        if pass1 != pass2 {
            println!("Passphrases do not match. Try again.");
            continue;
        }
        return Ok(pass1);
    }
}
