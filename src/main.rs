//! Corvid Agent CAN — lightweight Rust AlgoChat agent.
//!
//! Subcommands: setup (init), import, run, send, inbox, status, contacts, groups, change-password, info, fund, register

use std::fmt;
use std::sync::Arc;

use anyhow::{bail, Result};
use clap::{Parser, Subcommand, ValueEnum};
use ed25519_dalek::SigningKey;
use tracing::{info, warn};
use zeroize::Zeroize;

use algochat::{AlgoChat, AlgoChatConfig, AlgorandConfig};

mod agent;
mod algorand;
mod bridge;
mod contacts;
mod groups;
mod keystore;
mod sidecar;
mod storage;
mod transaction;
mod wallet;
mod wizard;

use storage::{SqliteKeyStorage, SqliteMessageCache};

use algorand::{HttpAlgodClient, HttpIndexerClient};
use contacts::ContactStore;
use groups::GroupStore;

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
    name = "can",
    about = "Corvid Agent CAN — lightweight Rust AlgoChat agent"
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
    /// Interactive setup wizard — generate or import a wallet with guided prompts
    #[command(alias = "init")]
    Setup {
        /// Algorand network preset (skips interactive prompt)
        #[arg(long, env = "CAN_NETWORK")]
        network: Option<Network>,

        /// Generate a new wallet (non-interactive mode)
        #[arg(long, conflicts_with_all = ["mnemonic", "seed"])]
        generate: bool,

        /// Import from 25-word Algorand mnemonic
        #[arg(long, conflicts_with_all = ["generate", "seed"])]
        mnemonic: Option<String>,

        /// Import from hex-encoded 32-byte Ed25519 seed
        #[arg(long, conflicts_with_all = ["generate", "mnemonic"])]
        seed: Option<String>,

        /// Password for keystore encryption (min 8 chars).
        /// If not provided, prompts interactively.
        #[arg(long, env = "CAN_PASSWORD")]
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
        #[arg(long, env = "CAN_PASSWORD")]
        password: Option<String>,
    },

    /// Start the agent and listen for AlgoChat messages
    Run {
        /// Algorand network preset
        #[arg(long, default_value = "localnet", env = "CAN_NETWORK")]
        network: Network,

        /// Override: Algorand node URL
        #[arg(long, env = "CAN_ALGOD_URL")]
        algod_url: Option<String>,

        /// Override: Algorand node token
        #[arg(long, env = "CAN_ALGOD_TOKEN")]
        algod_token: Option<String>,

        /// Override: Algorand indexer URL
        #[arg(long, env = "CAN_INDEXER_URL")]
        indexer_url: Option<String>,

        /// Override: Algorand indexer token
        #[arg(long, env = "CAN_INDEXER_TOKEN")]
        indexer_token: Option<String>,

        /// Agent seed (hex). If not provided, loads from keystore.
        #[arg(long, env = "CAN_SEED")]
        seed: Option<String>,

        /// Agent Algorand address. Required if --seed is provided.
        #[arg(long, env = "CAN_ADDRESS")]
        address: Option<String>,

        /// Keystore password (for loading from keystore)
        #[arg(long, env = "CAN_PASSWORD")]
        password: Option<String>,

        /// Agent name for discovery
        #[arg(long, default_value = "can")]
        name: String,

        /// corvid-agent hub URL
        #[arg(long, default_value = "http://localhost:3578")]
        hub_url: String,

        /// Poll interval in seconds
        #[arg(long, default_value = "5")]
        poll_interval: u64,

        /// Disable the plugin host sidecar
        #[arg(long, default_value = "false")]
        no_plugins: bool,

        /// Run in direct P2P mode (no hub forwarding — receive and store only)
        #[arg(long, default_value = "false")]
        no_hub: bool,
    },

    /// Send an encrypted message to a contact, address, or group
    Send {
        /// Recipient: contact name or Algorand address (mutually exclusive with --group)
        #[arg(long, required_unless_present = "group")]
        to: Option<String>,

        /// Send to all members of a group channel
        #[arg(long, conflicts_with = "to")]
        group: Option<String>,

        /// Message text to send
        #[arg(long)]
        message: String,

        /// Algorand network preset
        #[arg(long, default_value = "localnet", env = "CAN_NETWORK")]
        network: Network,

        /// Override: Algorand node URL
        #[arg(long, env = "CAN_ALGOD_URL")]
        algod_url: Option<String>,

        /// Override: Algorand node token
        #[arg(long, env = "CAN_ALGOD_TOKEN")]
        algod_token: Option<String>,

        /// Override: Algorand indexer URL
        #[arg(long, env = "CAN_INDEXER_URL")]
        indexer_url: Option<String>,

        /// Override: Algorand indexer token
        #[arg(long, env = "CAN_INDEXER_TOKEN")]
        indexer_token: Option<String>,

        /// Agent seed (hex). If not provided, loads from keystore.
        #[arg(long, env = "CAN_SEED")]
        seed: Option<String>,

        /// Agent Algorand address. Required if --seed is provided.
        #[arg(long, env = "CAN_ADDRESS")]
        address: Option<String>,

        /// Keystore password (for loading from keystore)
        #[arg(long, env = "CAN_PASSWORD")]
        password: Option<String>,
    },

    /// Read cached messages from the local inbox
    Inbox {
        /// Filter by sender: contact name or Algorand address
        #[arg(long)]
        from: Option<String>,

        /// Maximum number of messages to display
        #[arg(long, default_value = "20")]
        limit: usize,
    },

    /// Manage contacts
    Contacts {
        #[command(subcommand)]
        action: ContactsAction,
    },

    /// Manage group PSK channels
    Groups {
        #[command(subcommand)]
        action: GroupsAction,
    },

    /// Change the keystore password
    ChangePassword {
        /// Current password
        #[arg(long, env = "CAN_PASSWORD")]
        old_password: Option<String>,

        /// New password
        #[arg(long)]
        new_password: Option<String>,
    },

    /// Show agent identity and status
    Info,

    /// Check health of algod, indexer, hub, balance, contacts, and plugins
    Status {
        /// Algorand network preset
        #[arg(long, default_value = "localnet", env = "CAN_NETWORK")]
        network: Network,

        /// Override: Algorand node URL
        #[arg(long, env = "CAN_ALGOD_URL")]
        algod_url: Option<String>,

        /// Override: Algorand node token
        #[arg(long, env = "CAN_ALGOD_TOKEN")]
        algod_token: Option<String>,

        /// Override: Algorand indexer URL
        #[arg(long, env = "CAN_INDEXER_URL")]
        indexer_url: Option<String>,

        /// Override: Algorand indexer token
        #[arg(long, env = "CAN_INDEXER_TOKEN")]
        indexer_token: Option<String>,

        /// Agent seed (hex). If not provided, loads from keystore.
        #[arg(long, env = "CAN_SEED")]
        seed: Option<String>,

        /// Agent Algorand address. Required if --seed is provided.
        #[arg(long, env = "CAN_ADDRESS")]
        address: Option<String>,

        /// Keystore password (for loading from keystore)
        #[arg(long, env = "CAN_PASSWORD")]
        password: Option<String>,

        /// corvid-agent hub URL
        #[arg(long, default_value = "http://localhost:3578")]
        hub_url: String,
    },

    /// Manage plugins
    Plugin {
        #[command(subcommand)]
        action: PluginAction,
    },

    /// Fund the agent wallet from the localnet faucet (or show instructions for testnet/mainnet)
    Fund {
        /// Algorand network preset
        #[arg(long, default_value = "localnet", env = "CAN_NETWORK")]
        network: Network,

        /// Override: Algorand node URL
        #[arg(long, env = "CAN_ALGOD_URL")]
        algod_url: Option<String>,

        /// Override: Algorand node token
        #[arg(long, env = "CAN_ALGOD_TOKEN")]
        algod_token: Option<String>,

        /// Agent Algorand address. If not provided, reads from keystore.
        #[arg(long, env = "CAN_ADDRESS")]
        address: Option<String>,

        /// KMD URL (localnet only)
        #[arg(long, default_value = "http://localhost:4002")]
        kmd_url: String,

        /// KMD API token (localnet only)
        #[arg(long)]
        kmd_token: Option<String>,

        /// Amount to fund in microAlgos (default: 10 ALGO = 10_000_000)
        #[arg(long, default_value = "10000000")]
        amount: u64,
    },

    /// Register the agent with the hub
    Register {
        /// Agent Algorand address. If not provided, reads from keystore.
        #[arg(long, env = "CAN_ADDRESS")]
        address: Option<String>,

        /// Agent display name
        #[arg(long, default_value = "can")]
        name: String,

        /// corvid-agent hub URL
        #[arg(long, default_value = "http://localhost:3578")]
        hub_url: String,
    },
}

#[derive(Subcommand)]
enum PluginAction {
    /// List loaded plugins
    List,

    /// Invoke a plugin tool
    Invoke {
        /// Plugin ID (e.g. hello-world)
        plugin_id: String,

        /// Tool name (e.g. hello)
        tool: String,

        /// JSON input (e.g. '{"name": "World"}')
        #[arg(default_value = "{}")]
        input: String,
    },

    /// Load a plugin from a WASM file
    Load {
        /// Path to the .wasm file
        path: String,

        /// Trust tier: trusted, verified, untrusted
        #[arg(long, default_value = "untrusted")]
        tier: String,
    },

    /// Unload a plugin by ID
    Unload {
        /// Plugin ID
        plugin_id: String,
    },

    /// Check plugin host health
    Health,
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

#[derive(Subcommand)]
enum GroupsAction {
    /// Create a new group with a random PSK
    Create {
        /// Group name
        #[arg(long)]
        name: String,
    },

    /// List all groups
    List,

    /// Show group details and members
    Show {
        /// Group name
        name: String,
    },

    /// Add a member to a group
    AddMember {
        /// Group name
        #[arg(long)]
        group: String,

        /// Member's Algorand address
        #[arg(long)]
        address: String,

        /// Optional label for the member
        #[arg(long)]
        label: Option<String>,
    },

    /// Remove a member from a group
    RemoveMember {
        /// Group name
        #[arg(long)]
        group: String,

        /// Member's Algorand address
        #[arg(long)]
        address: String,
    },

    /// Remove a group and all its members
    Remove {
        /// Group name
        name: String,
    },

    /// Export groups as JSON
    Export {
        /// Output file (stdout if not specified)
        #[arg(long)]
        output: Option<String>,
    },

    /// Import groups from JSON
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

fn groups_db_path(data_dir: &str) -> std::path::PathBuf {
    std::path::Path::new(data_dir).join("groups.db")
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
        bail!("No wallet found. Run `can init` to create one, or provide --seed/--address.");
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

// cmd_init replaced by wizard::run_wizard — see src/wizard.rs

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
    no_plugins: bool,
    no_hub: bool,
    data_dir: &str,
) -> Result<()> {
    // First-run check: if no keystore and no seed flag, guide the user
    if seed_hex.is_none() && !wizard::check_first_run(data_dir) {
        bail!("Agent not set up. Run `can setup` (or `can init`) to create a wallet first.");
    }

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

    let effective_hub = if no_hub { "disabled (P2P mode)".to_string() } else { hub_url.clone() };

    info!(
        name = %name,
        network = %network,
        algod = %algod_url,
        indexer = %indexer_url,
        hub = %effective_hub,
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

    // Register group PSKs
    let groups_path = groups_db_path(data_dir);
    let group_store = if groups_path.exists() {
        Some(GroupStore::open(&groups_path)?)
    } else {
        None
    };

    let mut group_count = 0;
    if let Some(store) = &group_store {
        let groups = store.list()?;
        for group in &groups {
            let members = store.members(&group.name)?;
            let mut psk = [0u8; 32];
            psk.copy_from_slice(&group.psk);
            for member in &members {
                if member.address == agent_address {
                    continue; // Skip self
                }
                let label = member
                    .label
                    .clone()
                    .unwrap_or_else(|| format!("{}:{}", group.name, &member.address[..8]));
                if let Err(e) = client
                    .add_psk_contact(&member.address, &psk, Some(label))
                    .await
                {
                    tracing::warn!(
                        group = %group.name,
                        member = %member.address,
                        error = %e,
                        "failed to register group PSK contact"
                    );
                }
            }
            group_count += 1;
        }
        if group_count > 0 {
            info!(groups = group_count, "registered group PSK contacts");
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
    println!("  Groups:   {}", group_count);
    println!("  Hub:      {}\n", effective_hub);

    let client = Arc::new(client);

    // ── Plugin host sidecar ──────────────────────────────────────────
    let sidecar_handle = if no_plugins {
        info!("plugin host disabled (--no-plugins)");
        None
    } else {
        match sidecar::find_plugin_host_binary() {
            Some(binary) => {
                // Ensure plugins directory exists
                let plugins_dir = data_path.join("plugins");
                std::fs::create_dir_all(&plugins_dir)?;

                let config = sidecar::SidecarConfig {
                    binary: binary.clone(),
                    data_dir: data_path.to_path_buf(),
                    agent_id: agent_address.clone(),
                    log_level: "info".to_string(),
                };

                let handle = sidecar::spawn_sidecar(config);

                // Wait for socket to become available
                let socket_path = sidecar::SidecarHandle::socket_path(data_path);
                if sidecar::wait_for_socket(&socket_path, std::time::Duration::from_secs(10)).await
                {
                    // Connect the bridge to the plugin host
                    let plugin_bridge = bridge::PluginBridge::new(&socket_path);
                    match plugin_bridge.connect().await {
                        Ok(()) => {
                            // Quick health check + plugin count
                            let plugin_count = match plugin_bridge.list_plugins().await {
                                Ok(list) => {
                                    for p in &list {
                                        info!(id = %p.id, version = %p.version, "plugin loaded");
                                    }
                                    list.len()
                                }
                                Err(_) => 0,
                            };
                            info!(
                                binary = %binary.display(),
                                socket = %socket_path.display(),
                                plugins = plugin_count,
                                "plugin host sidecar ready"
                            );
                            println!(
                                "  Plugins:  active ({} loaded)",
                                plugin_count
                            );
                        }
                        Err(e) => {
                            warn!(error = %e, "plugin host socket ready but bridge connect failed");
                            println!("  Plugins:  active (bridge error)");
                        }
                    }
                } else {
                    warn!(
                        socket = %socket_path.display(),
                        "plugin host sidecar started but socket not ready after 10s"
                    );
                    println!("  Plugins:  starting...");
                }

                Some(handle)
            }
            None => {
                info!("corvid-plugin-host binary not found — plugins disabled");
                println!("  Plugins:  disabled (binary not found)");
                None
            }
        }
    };

    let algod_for_tx = Arc::new(HttpAlgodClient::new(&algod_url, &algod_token));

    let loop_client = Arc::clone(&client);
    let loop_algod = Arc::clone(&algod_for_tx);
    let loop_config = agent::AgentLoopConfig {
        poll_interval_secs: poll_interval,
        hub_url: if no_hub { None } else { Some(hub_url.clone()) },
        agent_name: name.clone(),
        agent_address: agent_address.clone(),
        signing_key,
    };

    let message_task = tokio::spawn(async move {
        agent::run_message_loop(loop_client, loop_algod, loop_config).await;
    });

    info!("can agent ready — listening for AlgoChat messages");

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

    // Shut down plugin host sidecar
    if let Some(handle) = sidecar_handle {
        info!("stopping plugin host sidecar");
        handle.shutdown().await;
        info!("plugin host sidecar stopped");
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
                println!("No contacts. Add one with: can contacts add --name <name> --address <addr> --psk <key>");
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

fn cmd_groups(action: GroupsAction, data_dir: &str) -> Result<()> {
    let data_path = std::path::Path::new(data_dir);
    std::fs::create_dir_all(data_path)?;

    let store = GroupStore::open(groups_db_path(data_dir))?;

    match action {
        GroupsAction::Create { name } => {
            let psk = store.create(&name)?;
            println!("Created group: {}", name);
            println!("  PSK: {}", hex::encode(psk));
            println!("\nShare this PSK with group members so they can add it as a contact.");
        }

        GroupsAction::List => {
            let groups = store.list()?;
            if groups.is_empty() {
                println!("No groups. Create one with: can groups create --name <name>");
                return Ok(());
            }
            println!("{:<20} {:<10} CREATED", "NAME", "MEMBERS");
            println!("{}", "-".repeat(50));
            for g in &groups {
                let member_count = store.members(&g.name)?.len();
                println!("{:<20} {:<10} {}", g.name, member_count, g.created_at);
            }
            println!("\n{} group(s)", groups.len());
        }

        GroupsAction::Show { name } => {
            let group = store
                .get(&name)?
                .ok_or_else(|| anyhow::anyhow!("Group \"{}\" not found", name))?;
            println!("Group: {}", group.name);
            println!("  PSK:     {}", hex::encode(&group.psk));
            println!("  Created: {}", group.created_at);

            let members = store.members(&name)?;
            if members.is_empty() {
                println!("  Members: none");
            } else {
                println!("  Members:");
                for m in &members {
                    let label = m.label.as_deref().unwrap_or("");
                    if label.is_empty() {
                        println!("    {} (added {})", m.address, m.added_at);
                    } else {
                        println!("    {} [{}] (added {})", m.address, label, m.added_at);
                    }
                }
            }
        }

        GroupsAction::AddMember {
            group,
            address,
            label,
        } => {
            wallet::decode_address(&address)?;
            store.add_member(&group, &address, label.as_deref())?;
            println!(
                "Added {} to group \"{}\"",
                label.as_deref().unwrap_or(&address),
                group
            );
        }

        GroupsAction::RemoveMember { group, address } => {
            if store.remove_member(&group, &address)? {
                println!("Removed {} from group \"{}\"", address, group);
            } else {
                println!("Member {} not found in group \"{}\"", address, group);
            }
        }

        GroupsAction::Remove { name } => {
            if store.remove(&name)? {
                println!("Removed group: {}", name);
            } else {
                println!("Group \"{}\" not found", name);
            }
        }

        GroupsAction::Export { output } => {
            let json = store.export_json()?;
            if let Some(path) = output {
                std::fs::write(&path, &json)?;
                println!("Exported {} group(s) to {}", store.count()?, path);
            } else {
                println!("{}", json);
            }
        }

        GroupsAction::Import { file } => {
            let json = std::fs::read_to_string(&file)?;
            let count = store.import_json(&json)?;
            println!("Imported {} group(s) from {}", count, file);
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
        bail!("No wallet found. Run `can init` first.");
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
        println!("Run `can init` to create a new wallet.");
        return Ok(());
    }

    let address = keystore::keystore_address(&ks_path)?;
    println!("Corvid Agent CAN");
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

#[allow(clippy::too_many_arguments)]
async fn cmd_status(
    network: Network,
    algod_url: Option<String>,
    algod_token: Option<String>,
    indexer_url: Option<String>,
    indexer_token: Option<String>,
    seed_hex: Option<String>,
    address: Option<String>,
    password: Option<String>,
    hub_url: String,
    data_dir: &str,
) -> Result<()> {
    use algochat::AlgodClient;

    let http = reqwest::Client::new();

    // Resolve network config
    let net = network.defaults();
    let algod_url = algod_url.unwrap_or(net.algod_url);
    let algod_token = algod_token.unwrap_or(net.algod_token);
    let indexer_url = indexer_url.unwrap_or(net.indexer_url);
    let indexer_token = indexer_token.unwrap_or(net.indexer_token);

    println!("Corvid Agent CAN — Status Check");
    println!("  Network: {}\n", network);

    // 1. Algod health
    let algod = HttpAlgodClient::new(&algod_url, &algod_token);
    print!("  Algod ({})... ", algod_url);
    match algod.get_current_round().await {
        Ok(round) => println!("OK (round {})", round),
        Err(e) => println!("FAIL ({})", e),
    }

    // 2. Indexer health
    print!("  Indexer ({})... ", indexer_url);
    match http
        .get(format!("{}/health", indexer_url.trim_end_matches('/')))
        .header("X-Indexer-API-Token", &indexer_token)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => println!("OK"),
        Ok(resp) => println!("FAIL (HTTP {})", resp.status()),
        Err(e) => println!("FAIL ({})", e),
    }

    // 3. Hub health
    print!("  Hub ({})... ", hub_url);
    match http
        .get(format!("{}/health", hub_url.trim_end_matches('/')))
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => println!("OK"),
        Ok(resp) => println!("FAIL (HTTP {})", resp.status()),
        Err(e) => println!("FAIL ({})", e),
    }

    // 4. Identity + balance
    match load_identity(
        seed_hex.as_deref(),
        address.as_deref(),
        password.as_deref(),
        data_dir,
    ) {
        Ok((_seed, addr)) => {
            println!("\n  Address: {}", addr);
            match algod.get_account_info(&addr).await {
                Ok(info) => {
                    let algo = info.amount as f64 / 1_000_000.0;
                    let min_algo = info.min_balance as f64 / 1_000_000.0;
                    println!("  Balance: {:.6} ALGO (min: {:.6})", algo, min_algo);
                    if info.amount < 100_000 {
                        println!("  WARNING: Balance is very low — may not be able to send messages");
                    }
                }
                Err(e) => println!("  Balance: unknown ({})", e),
            }
        }
        Err(_) => {
            println!("\n  Wallet: not configured (run `can init`)");
        }
    }

    // 5. Contacts
    let contacts_path = contacts_db_path(data_dir);
    if contacts_path.exists() {
        let store = ContactStore::open(&contacts_path)?;
        println!("  Contacts: {}", store.count()?);
    } else {
        println!("  Contacts: 0");
    }

    // 6. Message cache stats
    let data_path = std::path::Path::new(data_dir);
    let messages_db = data_path.join("messages.db");
    if messages_db.exists() {
        let conn = rusqlite::Connection::open(&messages_db)?;
        let msg_count: i64 = conn.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))?;
        let conv_count: i64 = conn.query_row(
            "SELECT COUNT(DISTINCT participant) FROM messages",
            [],
            |r| r.get(0),
        )?;
        println!("  Messages: {} ({} conversations)", msg_count, conv_count);
    } else {
        println!("  Messages: 0 (no cache)");
    }

    // 7. Plugin host
    let socket_path = sidecar::SidecarHandle::socket_path(data_path);
    if socket_path.exists() {
        let plugin_bridge = bridge::PluginBridge::new(&socket_path);
        match plugin_bridge.connect().await {
            Ok(()) => match plugin_bridge.list_plugins().await {
                Ok(plugins) => println!("  Plugins: {} loaded", plugins.len()),
                Err(e) => println!("  Plugins: connected but list failed ({})", e),
            },
            Err(_) => println!("  Plugins: socket exists but not responding"),
        }
    } else {
        println!("  Plugins: not running");
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn cmd_send(
    to: Option<String>,
    group: Option<String>,
    message: String,
    network: Network,
    algod_url: Option<String>,
    algod_token: Option<String>,
    indexer_url: Option<String>,
    indexer_token: Option<String>,
    seed_hex: Option<String>,
    address: Option<String>,
    password: Option<String>,
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

    // Load contacts and register PSKs
    let contacts_path = contacts_db_path(data_dir);
    let contact_store = if contacts_path.exists() {
        Some(ContactStore::open(&contacts_path)?)
    } else {
        None
    };

    if let Some(store) = &contact_store {
        let contacts = store.list()?;
        for contact in &contacts {
            let mut psk = [0u8; 32];
            psk.copy_from_slice(&contact.psk);
            let _ = client
                .add_psk_contact(&contact.address, &psk, Some(contact.name.clone()))
                .await;
        }
    }

    // Build a separate algod client for transaction submission
    let algod_for_tx = HttpAlgodClient::new(&algod_url, &algod_token);

    // Group send: broadcast to all members
    if let Some(group_name) = group {
        let group_store = GroupStore::open(groups_db_path(data_dir))?;
        let grp = group_store
            .get(&group_name)?
            .ok_or_else(|| anyhow::anyhow!("Group \"{}\" not found", group_name))?;

        let members = group_store.members(&group_name)?;
        if members.is_empty() {
            bail!(
                "Group \"{}\" has no members. Add members with: can groups add-member --group {} --address <addr>",
                group_name, group_name
            );
        }

        // Register group PSK for each member
        let mut psk = [0u8; 32];
        psk.copy_from_slice(&grp.psk);
        for member in &members {
            if member.address == agent_address {
                continue; // Skip self
            }
            let _ = client
                .add_psk_contact(
                    &member.address,
                    &psk,
                    member.label.clone().or_else(|| Some(group_name.clone())),
                )
                .await;
        }

        let mut sent = 0;
        for member in &members {
            if member.address == agent_address {
                continue; // Skip self
            }

            match agent::send_reply(
                &client,
                &algod_for_tx,
                &agent_address,
                &member.address,
                &message,
                &signing_key,
            )
            .await
            {
                Ok(txid) => {
                    let label = member.label.as_deref().unwrap_or(&member.address);
                    println!("  Sent to {} ({})", label, txid);
                    sent += 1;
                }
                Err(e) => {
                    warn!(
                        error = %e,
                        member = %member.address,
                        "failed to send group message"
                    );
                    eprintln!("  FAIL {} ({})", member.address, e);
                }
            }
        }

        println!(
            "\nGroup \"{}\" — sent to {}/{} members",
            group_name,
            sent,
            members.len()
        );
        return Ok(());
    }

    // Single recipient send
    let to = to.unwrap(); // Safe: clap ensures --to or --group

    // Resolve recipient: try as contact name first, then as raw address
    let recipient_address = if let Some(store) = &contact_store {
        if let Some(contact) = store.get(&to)? {
            info!(name = %to, address = %contact.address, "resolved contact");
            contact.address
        } else if let Some(contact) = store.get_by_address(&to)? {
            info!(name = %contact.name, address = %to, "matched contact by address");
            to.clone()
        } else {
            // Validate as raw Algorand address
            wallet::decode_address(&to)?;
            to.clone()
        }
    } else {
        wallet::decode_address(&to)?;
        to.clone()
    };

    // Encrypt and send
    let txid = agent::send_reply(
        &client,
        &algod_for_tx,
        &agent_address,
        &recipient_address,
        &message,
        &signing_key,
    )
    .await?;

    println!("Message sent!");
    println!("  To:   {}", recipient_address);
    println!("  TxID: {}", txid);
    println!("  Size: {} chars", message.len());

    Ok(())
}

fn cmd_inbox(from: Option<String>, limit: usize, data_dir: &str) -> Result<()> {
    let data_path = std::path::Path::new(data_dir);
    let messages_db = data_path.join("messages.db");

    if !messages_db.exists() {
        println!("No messages yet. Run `can run` to start receiving messages.");
        return Ok(());
    }

    // Open message cache directly (no async needed for reads)
    let conn = rusqlite::Connection::open(&messages_db)?;

    // Load contacts for name resolution
    let contacts_path = contacts_db_path(data_dir);
    let contact_store = if contacts_path.exists() {
        Some(ContactStore::open(&contacts_path)?)
    } else {
        None
    };

    // Resolve --from filter to an address if it's a contact name
    let from_address = if let Some(ref from_str) = from {
        if let Some(store) = &contact_store {
            if let Some(contact) = store.get(from_str)? {
                Some(contact.address)
            } else {
                Some(from_str.clone())
            }
        } else {
            Some(from_str.clone())
        }
    } else {
        None
    };

    // Query messages
    let (query, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
        if let Some(ref addr) = from_address {
            (
                "SELECT id, participant, sender, recipient, content, timestamp_secs, \
                     confirmed_round, direction, reply_to_id, reply_to_preview \
                     FROM messages WHERE participant = ?1 \
                     ORDER BY timestamp_secs DESC LIMIT ?2"
                    .to_string(),
                vec![
                    Box::new(addr.clone()) as Box<dyn rusqlite::types::ToSql>,
                    Box::new(limit as i64),
                ],
            )
        } else {
            (
                "SELECT id, participant, sender, recipient, content, timestamp_secs, \
                     confirmed_round, direction, reply_to_id, reply_to_preview \
                     FROM messages \
                     ORDER BY timestamp_secs DESC LIMIT ?1"
                    .to_string(),
                vec![Box::new(limit as i64) as Box<dyn rusqlite::types::ToSql>],
            )
        };

    let mut stmt = conn.prepare(&query)?;
    let params_refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|p| &**p).collect();
    let rows = stmt.query_map(params_refs.as_slice(), |row| {
        let sender: String = row.get(2)?;
        let recipient: String = row.get(3)?;
        let content: String = row.get(4)?;
        let timestamp_secs: i64 = row.get(5)?;
        let confirmed_round: u64 = row.get(6)?;
        let direction: String = row.get(7)?;
        Ok((
            sender,
            recipient,
            content,
            timestamp_secs,
            confirmed_round,
            direction,
        ))
    })?;

    let mut messages: Vec<_> = rows.collect::<std::result::Result<Vec<_>, _>>()?;

    if messages.is_empty() {
        if let Some(ref f) = from {
            println!("No messages from {}.", f);
        } else {
            println!("Inbox is empty. Run `can run` to start receiving messages.");
        }
        return Ok(());
    }

    // Reverse so oldest is first (we queried DESC for limit, display ASC)
    messages.reverse();

    // Helper to resolve address to contact name
    let resolve_name = |addr: &str| -> String {
        if let Some(store) = &contact_store {
            if let Ok(Some(contact)) = store.get_by_address(addr) {
                return contact.name;
            }
        }
        // Truncate address for display
        if addr.len() > 12 {
            format!("{}...", &addr[..12])
        } else {
            addr.to_string()
        }
    };

    println!(
        "{:<5} {:<8} {:<16} {:<20} MESSAGE",
        "ROUND", "DIR", "FROM/TO", "TIME"
    );
    println!("{}", "-".repeat(80));

    for (sender, recipient, content, timestamp_secs, confirmed_round, direction) in &messages {
        let dir_label = if direction == "sent" { ">>>" } else { "<<<" };
        let peer = if direction == "sent" {
            resolve_name(recipient)
        } else {
            resolve_name(sender)
        };

        // Format timestamp
        let time_str = chrono::DateTime::from_timestamp(*timestamp_secs, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "unknown".to_string());

        // Truncate content for display
        let display_content = if content.len() > 60 {
            format!("{}...", &content[..57])
        } else {
            content.clone()
        };

        println!(
            "{:<5} {:<8} {:<16} {:<20} {}",
            confirmed_round, dir_label, peer, time_str, display_content
        );
    }

    println!("\n{} message(s)", messages.len());
    Ok(())
}

async fn cmd_plugin(action: PluginAction, data_dir: &str) -> Result<()> {
    let data_path = std::path::Path::new(data_dir);
    let socket_path = sidecar::SidecarHandle::socket_path(data_path);

    let bridge = bridge::PluginBridge::new(&socket_path);

    if let Err(e) = bridge.connect().await {
        eprintln!(
            "Cannot connect to plugin host at {}\n\
             Is the agent running? (can run)\n\
             Error: {}",
            socket_path.display(),
            e
        );
        std::process::exit(1);
    }

    match action {
        PluginAction::List => {
            let plugins = bridge.list_plugins().await?;
            if plugins.is_empty() {
                println!("No plugins loaded.");
                println!("Place .wasm files in {}/plugins/ and restart.", data_dir);
                return Ok(());
            }
            println!("{:<20} {:<10} {:<12} DESCRIPTION", "ID", "VERSION", "TIER");
            println!("{}", "-".repeat(70));
            for p in &plugins {
                println!(
                    "{:<20} {:<10} {:<12} {}",
                    p.id, p.version, p.trust_tier, p.description
                );
            }
            println!("\n{} plugin(s) loaded", plugins.len());
        }

        PluginAction::Invoke {
            plugin_id,
            tool,
            input,
        } => {
            let input_value: serde_json::Value = serde_json::from_str(&input)
                .map_err(|e| anyhow::anyhow!("Invalid JSON input: {e}"))?;

            let result = bridge.invoke(&plugin_id, &tool, input_value).await?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }

        PluginAction::Load { path, tier } => {
            // Resolve to absolute path
            let abs_path = std::fs::canonicalize(&path)
                .map_err(|e| anyhow::anyhow!("Cannot find file '{}': {}", path, e))?;

            let result = bridge
                .load_plugin(&abs_path.display().to_string(), &tier)
                .await?;
            println!("Loaded: {}", serde_json::to_string_pretty(&result)?);
        }

        PluginAction::Unload { plugin_id } => {
            bridge.unload_plugin(&plugin_id).await?;
            println!("Unloaded plugin: {}", plugin_id);
        }

        PluginAction::Health => {
            let status = bridge.health().await?;
            println!("Plugin Host Health");
            println!("  Uptime: {:.1}s", status.uptime_ms as f64 / 1000.0);
            if let Some(plugins) = status.plugins.as_object() {
                if plugins.is_empty() {
                    println!("  Plugins: none loaded");
                } else {
                    println!("  Plugins:");
                    for (id, state) in plugins {
                        println!("    {}: {}", id, state);
                    }
                }
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------
// Fund command — KMD faucet for localnet
// ---------------------------------------------------------------------------

/// KMD API response types.
mod kmd {
    use serde::Deserialize;

    #[derive(Deserialize)]
    pub struct WalletsResponse {
        pub wallets: Option<Vec<Wallet>>,
    }

    #[derive(Deserialize)]
    pub struct Wallet {
        pub id: String,
        pub name: String,
    }

    #[derive(Deserialize)]
    pub struct InitResponse {
        pub wallet_handle_token: String,
    }

    #[derive(Deserialize)]
    pub struct KeysResponse {
        pub addresses: Option<Vec<String>>,
    }

    #[derive(Deserialize)]
    pub struct SignResponse {
        pub signed_transaction: String, // base64
    }
}

struct FundParams {
    network: Network,
    algod_url: Option<String>,
    algod_token: Option<String>,
    address: Option<String>,
    kmd_url: String,
    kmd_token: Option<String>,
    amount: u64,
}

async fn cmd_fund(params: FundParams, data_dir: &str) -> Result<()> {
    let FundParams {
        network,
        algod_url,
        algod_token,
        address,
        kmd_url,
        kmd_token,
        amount,
    } = params;
    use algochat::AlgodClient;
    use base64::Engine;

    // Resolve agent address (no password needed — just read from keystore)
    let agent_address = match address {
        Some(a) => a,
        None => {
            let ks_path = keystore_path(data_dir);
            if !keystore::keystore_exists(&ks_path) {
                bail!("No wallet found. Run `can init` first, or provide --address.");
            }
            keystore::keystore_address(&ks_path)?
        }
    };

    match network {
        Network::Testnet => {
            println!("Fund your agent on TestNet:");
            println!("  Address:   {}", agent_address);
            println!("  Dispenser: https://bank.testnet.algorand.network");
            return Ok(());
        }
        Network::Mainnet => {
            println!("Send ALGO to your agent address on MainNet:");
            println!("  Address: {}", agent_address);
            return Ok(());
        }
        Network::Localnet => {} // continue below
    }

    let algo = amount as f64 / 1_000_000.0;
    println!("Funding agent on localnet ({:.6} ALGO)...", algo);
    println!("  Recipient: {}", agent_address);

    // Resolve network config
    let net = network.defaults();
    let algod_url = algod_url.unwrap_or(net.algod_url);
    let algod_token = algod_token.unwrap_or(net.algod_token);
    let kmd_token = kmd_token.unwrap_or_else(|| "a".repeat(64));

    let http = reqwest::Client::new();

    // 1. List wallets — find "unencrypted-default-wallet"
    let wallets_resp: kmd::WalletsResponse = http
        .get(format!("{}/v1/wallets", kmd_url))
        .header("X-KMD-API-Token", &kmd_token)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Cannot connect to KMD at {}: {}", kmd_url, e))?
        .json()
        .await?;

    let wallets = wallets_resp.wallets.unwrap_or_default();
    let default_wallet = wallets
        .iter()
        .find(|w| w.name == "unencrypted-default-wallet")
        .ok_or_else(|| {
            anyhow::anyhow!("No default wallet found in KMD. Is the localnet running?")
        })?;

    // 2. Init wallet handle
    let init_resp: kmd::InitResponse = http
        .post(format!("{}/v1/wallet/init", kmd_url))
        .header("X-KMD-API-Token", &kmd_token)
        .json(&serde_json::json!({
            "wallet_id": default_wallet.id,
            "wallet_password": ""
        }))
        .send()
        .await?
        .json()
        .await?;

    let handle = &init_resp.wallet_handle_token;

    // 3. List keys — pick first funded address
    let keys_resp: kmd::KeysResponse = http
        .post(format!("{}/v1/key/list", kmd_url))
        .header("X-KMD-API-Token", &kmd_token)
        .json(&serde_json::json!({ "wallet_handle_token": handle }))
        .send()
        .await?
        .json()
        .await?;

    let addresses = keys_resp.addresses.unwrap_or_default();
    let faucet_address = addresses
        .first()
        .ok_or_else(|| anyhow::anyhow!("No accounts found in KMD default wallet"))?;

    println!("  Faucet:    {}", faucet_address);

    // 4. Build unsigned payment transaction
    let algod = HttpAlgodClient::new(&algod_url, &algod_token);
    let params = algod
        .get_suggested_params()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to get suggested params: {}", e))?;

    let raw_txn = transaction::build_payment_transaction_with_amount(
        faucet_address,
        &agent_address,
        amount,
        &params,
    )?;

    // 5. Sign via KMD
    let b64 = base64::engine::general_purpose::STANDARD;
    let txn_b64 = b64.encode(&raw_txn);

    let sign_resp: kmd::SignResponse = http
        .post(format!("{}/v1/transaction/sign", kmd_url))
        .header("X-KMD-API-Token", &kmd_token)
        .json(&serde_json::json!({
            "wallet_handle_token": handle,
            "wallet_password": "",
            "transaction": txn_b64
        }))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("KMD signing failed: {}", e))?
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to parse KMD sign response: {}", e))?;

    let signed_txn = b64
        .decode(&sign_resp.signed_transaction)
        .map_err(|e| anyhow::anyhow!("Invalid base64 in KMD response: {}", e))?;

    // 6. Submit via algod
    let txid = algod
        .submit_transaction(&signed_txn)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to submit transaction: {}", e))?;

    println!("  TxID:      {}", txid);

    // 7. Wait for confirmation
    algod
        .wait_for_confirmation(&txid, 5)
        .await
        .map_err(|e| anyhow::anyhow!("Confirmation failed: {}", e))?;

    // Check new balance
    match algod.get_account_info(&agent_address).await {
        Ok(info) => {
            let balance = info.amount as f64 / 1_000_000.0;
            println!("\n  Funded! New balance: {:.6} ALGO", balance);
        }
        Err(_) => println!("\n  Funded! (could not fetch new balance)"),
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Register command — register agent with the hub
// ---------------------------------------------------------------------------

async fn cmd_register(
    address: Option<String>,
    name: String,
    hub_url: String,
    data_dir: &str,
) -> Result<()> {
    // Resolve agent address (no password needed)
    let agent_address = match address {
        Some(a) => a,
        None => {
            let ks_path = keystore_path(data_dir);
            if !keystore::keystore_exists(&ks_path) {
                bail!("No wallet found. Run `can init` first, or provide --address.");
            }
            keystore::keystore_address(&ks_path)?
        }
    };

    println!("Registering agent with hub...");
    println!("  Address: {}", agent_address);
    println!("  Name:    {}", name);
    println!("  Hub:     {}", hub_url);

    let http = reqwest::Client::new();
    let url = format!("{}/a2a/agents/register", hub_url.trim_end_matches('/'));

    let payload = serde_json::json!({
        "address": agent_address,
        "name": name,
    });

    let resp = http
        .post(&url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("Cannot connect to hub at {}: {}", hub_url, e))?;

    if resp.status().is_success() {
        let body: serde_json::Value = resp.json().await.unwrap_or_else(|_| serde_json::json!({}));
        println!("\nRegistered successfully!");
        if let Some(id) = body.get("id").and_then(|v| v.as_str()) {
            println!("  Agent ID: {}", id);
        }
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("Hub registration failed (HTTP {}): {}", status, body);
    }

    Ok(())
}

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
        Command::Setup {
            network,
            generate,
            mnemonic,
            seed,
            password,
        } => {
            let config = wizard::WizardConfig {
                network,
                generate,
                import_mnemonic: mnemonic,
                import_seed: seed,
                password,
                data_dir: data_dir.to_string(),
            };
            wizard::run_wizard(config)?;
            Ok(())
        }

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
            no_plugins,
            no_hub,
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
                no_plugins,
                no_hub,
                data_dir,
            )
            .await
        }

        Command::Send {
            to,
            group,
            message,
            network,
            algod_url,
            algod_token,
            indexer_url,
            indexer_token,
            seed,
            address,
            password,
        } => {
            cmd_send(
                to,
                group,
                message,
                network,
                algod_url,
                algod_token,
                indexer_url,
                indexer_token,
                seed,
                address,
                password,
                data_dir,
            )
            .await
        }

        Command::Inbox { from, limit } => cmd_inbox(from, limit, data_dir),

        Command::Contacts { action } => cmd_contacts(action, data_dir),

        Command::Groups { action } => cmd_groups(action, data_dir),

        Command::ChangePassword {
            old_password,
            new_password,
        } => cmd_change_password(old_password, new_password, data_dir),

        Command::Info => cmd_info(data_dir),

        Command::Status {
            network,
            algod_url,
            algod_token,
            indexer_url,
            indexer_token,
            seed,
            address,
            password,
            hub_url,
        } => {
            cmd_status(
                network,
                algod_url,
                algod_token,
                indexer_url,
                indexer_token,
                seed,
                address,
                password,
                hub_url,
                data_dir,
            )
            .await
        }

        Command::Plugin { action } => cmd_plugin(action, data_dir).await,

        Command::Fund {
            network,
            algod_url,
            algod_token,
            address,
            kmd_url,
            kmd_token,
            amount,
        } => {
            cmd_fund(
                FundParams {
                    network,
                    algod_url,
                    algod_token,
                    address,
                    kmd_url,
                    kmd_token,
                    amount,
                },
                data_dir,
            )
            .await
        }

        Command::Register {
            address,
            name,
            hub_url,
        } => cmd_register(address, name, hub_url, data_dir).await,
    }
}
