//! Plugin CLI binary — `corvid plugin <subcommand>`.
//!
//! Subcommands: new, validate, install, list, uninstall.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use corvid_plugin_sdk::TrustTier;

#[derive(Parser, Debug)]
#[command(
    name = "corvid-plugin",
    about = "Plugin management CLI for corvid-agent"
)]
struct Cli {
    /// Base directory for DB and plugin storage.
    #[arg(long, default_value = "~/.corvid", global = true)]
    data_dir: String,

    /// Unix socket path for communicating with the running plugin host.
    #[arg(long, global = true)]
    socket_path: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Scaffold a new plugin project from template.
    New {
        /// Plugin name (lowercase, alphanumeric + hyphens).
        name: String,

        /// Author name or organization.
        #[arg(long, default_value = "CorvidLabs")]
        author: String,

        /// Trust tier for the plugin.
        #[arg(long, default_value = "untrusted")]
        tier: String,
    },

    /// Validate a built plugin WASM or plugin.toml.
    Validate {
        /// Path to .wasm file or plugin.toml.
        #[arg(default_value = "plugin.toml")]
        path: String,
    },

    /// Install a plugin from GitHub release.
    Install {
        /// Source in format `owner/repo@version` (e.g. CorvidLabs/corvid-plugin-algo-oracle@0.3.1).
        source: String,
    },

    /// List installed plugins.
    List {
        /// Output as JSON.
        #[arg(long)]
        json: bool,
    },

    /// Remove an installed plugin.
    Uninstall {
        /// Plugin ID to remove.
        plugin_id: String,
    },
}

fn parse_tier(s: &str) -> Result<TrustTier> {
    match s.to_lowercase().as_str() {
        "trusted" => Ok(TrustTier::Trusted),
        "verified" => Ok(TrustTier::Verified),
        "untrusted" => Ok(TrustTier::Untrusted),
        _ => bail!("invalid tier '{s}' — expected trusted, verified, or untrusted"),
    }
}

fn resolve_data_dir(raw: &str) -> PathBuf {
    if raw.starts_with('~') {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(raw.replacen('~', &home, 1));
        }
    }
    PathBuf::from(raw)
}

fn resolve_socket_path(cli: &Cli) -> PathBuf {
    cli.socket_path
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| resolve_data_dir(&cli.data_dir).join("plugins.sock"))
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    let data_dir = resolve_data_dir(&cli.data_dir);

    match cli.command {
        Command::New { name, author, tier } => {
            let tier = parse_tier(&tier)?;
            let path = corvid_plugin_cli::scaffold::scaffold(&name, &author, tier)?;
            println!("Created plugin project: {}", path.display());
            println!("\nNext steps:");
            println!("  cd {}", path.display());
            println!("  cargo build --target wasm32-wasip1 --release");
        }

        Command::Validate { path } => {
            let path = PathBuf::from(&path);
            let report = if path.extension().map(|e| e == "toml").unwrap_or(false) {
                corvid_plugin_cli::validate::validate_plugin_toml(&path)?
            } else {
                corvid_plugin_cli::validate::validate_plugin(&path)?
            };

            print!("{report}");

            if !report.is_ok() {
                std::process::exit(1);
            }
        }

        Command::Install { ref source } => {
            cmd_install(source, &data_dir, &resolve_socket_path(&cli)).await?;
        }

        Command::List { json } => {
            cmd_list(&data_dir, json, &resolve_socket_path(&cli)).await?;
        }

        Command::Uninstall { ref plugin_id } => {
            cmd_uninstall(plugin_id, &data_dir, &resolve_socket_path(&cli)).await?;
        }
    }

    Ok(())
}

/// Parse `owner/repo@version` source string.
fn parse_source(source: &str) -> Result<(&str, &str, &str)> {
    let (repo_part, version) = source
        .split_once('@')
        .context("source must be in format owner/repo@version")?;

    let (owner, repo) = repo_part
        .split_once('/')
        .context("source must be in format owner/repo@version")?;

    if owner.is_empty() || repo.is_empty() || version.is_empty() {
        bail!("source must be in format owner/repo@version");
    }

    Ok((owner, repo, version))
}

/// Install a plugin from GitHub release.
async fn cmd_install(source: &str, data_dir: &Path, socket_path: &Path) -> Result<()> {
    let (owner, repo, version) = parse_source(source)?;
    let tag = format!("v{version}");

    println!("Installing {owner}/{repo}@{version}...");

    let client = reqwest::Client::new();

    // Step 1: Fetch plugin.toml from release assets
    let plugin_toml_url =
        format!("https://github.com/{owner}/{repo}/releases/download/{tag}/plugin.toml");

    let plugin_toml_resp = client
        .get(&plugin_toml_url)
        .send()
        .await
        .context("failed to fetch plugin.toml from release")?;

    if !plugin_toml_resp.status().is_success() {
        bail!(
            "release not found: {owner}/{repo}@{tag} (HTTP {})",
            plugin_toml_resp.status()
        );
    }

    let plugin_toml_text = plugin_toml_resp.text().await?;
    let plugin_toml: toml::Value =
        toml::from_str(&plugin_toml_text).context("failed to parse plugin.toml")?;

    let plugin_section = plugin_toml
        .get("plugin")
        .context("plugin.toml missing [plugin] section")?;

    let plugin_id = plugin_section
        .get("id")
        .and_then(|v| v.as_str())
        .context("plugin.toml missing id")?;

    let tier_str = plugin_section
        .get("trust-tier")
        .and_then(|v| v.as_str())
        .unwrap_or("untrusted");

    let tier = parse_tier(tier_str)?;

    // Get WASM artifact name from [build] section
    let wasm_artifact = plugin_toml
        .get("build")
        .and_then(|b| b.get("wasm-artifact"))
        .and_then(|v| v.as_str())
        .context("plugin.toml missing [build].wasm-artifact")?;

    // Step 2: Download .wasm artifact
    let wasm_url =
        format!("https://github.com/{owner}/{repo}/releases/download/{tag}/{wasm_artifact}");

    println!("Downloading {wasm_artifact}...");
    let wasm_resp = client
        .get(&wasm_url)
        .send()
        .await
        .context("failed to download WASM artifact")?;

    if !wasm_resp.status().is_success() {
        bail!(
            "WASM artifact not found: {wasm_artifact} (HTTP {})",
            wasm_resp.status()
        );
    }

    let wasm_bytes = wasm_resp.bytes().await?;

    // Step 3: Verify Ed25519 signature for Trusted tier
    if tier == TrustTier::Trusted {
        let sig_url = format!(
            "https://github.com/{owner}/{repo}/releases/download/{tag}/{wasm_artifact}.sig"
        );
        let sig_resp = client.get(&sig_url).send().await;

        match sig_resp {
            Ok(resp) if resp.status().is_success() => {
                // TODO: Verify signature with key registry
                println!("Signature found (verification not yet implemented)");
            }
            _ => {
                bail!("Ed25519 signature required for Trusted tier install — .sig file not found");
            }
        }
    }

    // Step 4: Validate the WASM
    println!("Validating plugin...");
    let plugins_dir = data_dir.join("plugins");
    std::fs::create_dir_all(&plugins_dir)?;

    let wasm_path = plugins_dir.join(wasm_artifact);
    std::fs::write(&wasm_path, &wasm_bytes)?;

    let report = corvid_plugin_cli::validate::validate_plugin(&wasm_path)?;
    if !report.is_ok() {
        // Clean up on validation failure
        let _ = std::fs::remove_file(&wasm_path);
        print!("{report}");
        bail!("plugin validation failed");
    }

    // Step 5: Check manifest ID matches plugin.toml
    if let Some(manifest) = &report.manifest {
        if manifest.id != plugin_id {
            let _ = std::fs::remove_file(&wasm_path);
            bail!(
                "ID mismatch: plugin.toml says '{}' but WASM manifest says '{}'",
                plugin_id,
                manifest.id
            );
        }
    }

    // Step 6: Register in SQLite
    let wasm_hash = {
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(&wasm_bytes);
        hex::encode(hash)
    };

    let db_path = data_dir.join("corvid-agent.db");
    let db = rusqlite::Connection::open(&db_path).context("failed to open plugin database")?;

    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS plugins (
            id           TEXT PRIMARY KEY,
            version      TEXT NOT NULL,
            tier         TEXT NOT NULL,
            wasm_hash    TEXT NOT NULL,
            installed_at INTEGER NOT NULL,
            enabled      INTEGER NOT NULL DEFAULT 1
        )",
    )?;

    // Check for existing installation
    let existing: Option<String> = db
        .query_row(
            "SELECT version FROM plugins WHERE id = ?1",
            [plugin_id],
            |row| row.get(0),
        )
        .ok();

    if let Some(existing_ver) = &existing {
        if existing_ver == version {
            println!("Plugin {plugin_id} v{version} is already installed");
            return Ok(());
        }
        println!("Upgrading {plugin_id} from v{existing_ver} to v{version}");
    }

    let now = chrono::Utc::now().timestamp();
    db.execute(
        "INSERT OR REPLACE INTO plugins (id, version, tier, wasm_hash, installed_at, enabled) VALUES (?1, ?2, ?3, ?4, ?5, 1)",
        rusqlite::params![plugin_id, version, tier_str, wasm_hash, now],
    )?;

    println!("Registered {plugin_id} v{version} ({tier_str})");

    // Step 7: Signal running host to reload
    if socket_path.exists() {
        match signal_host_reload(socket_path, plugin_id, &wasm_path, tier_str).await {
            Ok(()) => println!("Plugin host notified — hot-reload initiated"),
            Err(e) => println!("Warning: could not notify plugin host: {e}"),
        }
    } else {
        println!("Warning: plugin host not running (will load on next start)");
    }

    println!("Installed {plugin_id} v{version}");
    Ok(())
}

/// Send JSON-RPC reload signal to the running plugin host.
async fn signal_host_reload(
    socket_path: &Path,
    plugin_id: &str,
    wasm_path: &Path,
    tier: &str,
) -> Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    let stream = UnixStream::connect(socket_path).await?;
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    let req = serde_json::json!({
        "method": "plugin.reload",
        "params": {
            "id": plugin_id,
            "path": wasm_path.to_string_lossy(),
            "tier": tier,
        },
        "id": 1
    });

    writer
        .write_all(format!("{}\n", serde_json::to_string(&req)?).as_bytes())
        .await?;

    let mut line = String::new();
    reader.read_line(&mut line).await?;

    let resp: serde_json::Value = serde_json::from_str(&line)?;
    if let Some(err) = resp.get("error").and_then(|e| e.as_str()) {
        bail!("host returned error: {err}");
    }

    Ok(())
}

/// List installed plugins.
async fn cmd_list(data_dir: &Path, json: bool, socket_path: &Path) -> Result<()> {
    // First try querying the running host
    if socket_path.exists() {
        match query_host_list(socket_path).await {
            Ok(manifests) => {
                if json {
                    println!("{}", serde_json::to_string_pretty(&manifests)?);
                } else {
                    if manifests.is_empty() {
                        println!("No plugins loaded");
                    } else {
                        println!("{:<30} {:<12} {:<12} STATUS", "ID", "VERSION", "TIER");
                        println!("{}", "-".repeat(70));
                        for m in &manifests {
                            println!(
                                "{:<30} {:<12} {:<12} active",
                                m.get("id").and_then(|v| v.as_str()).unwrap_or("?"),
                                m.get("version").and_then(|v| v.as_str()).unwrap_or("?"),
                                m.get("trust_tier").and_then(|v| v.as_str()).unwrap_or("?"),
                            );
                        }
                    }
                }
                return Ok(());
            }
            Err(e) => {
                tracing::debug!("host query failed, falling back to DB: {e}");
            }
        }
    }

    // Fallback: read from SQLite
    let db_path = data_dir.join("corvid-agent.db");
    if !db_path.exists() {
        println!("No plugins installed");
        return Ok(());
    }

    let db = rusqlite::Connection::open(&db_path)?;

    let mut stmt = db.prepare("SELECT id, version, tier, enabled FROM plugins ORDER BY id")?;

    let rows: Vec<(String, String, String, bool)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, bool>(3)?,
            ))
        })?
        .collect::<Result<_, _>>()?;

    if json {
        let json_rows: Vec<serde_json::Value> = rows
            .iter()
            .map(|(id, ver, tier, enabled)| {
                serde_json::json!({
                    "id": id,
                    "version": ver,
                    "tier": tier,
                    "enabled": enabled,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json_rows)?);
    } else if rows.is_empty() {
        println!("No plugins installed");
    } else {
        println!("{:<30} {:<12} {:<12} ENABLED", "ID", "VERSION", "TIER");
        println!("{}", "-".repeat(70));
        for (id, ver, tier, enabled) in &rows {
            println!(
                "{:<30} {:<12} {:<12} {}",
                id,
                ver,
                tier,
                if *enabled { "yes" } else { "no" }
            );
        }
    }

    Ok(())
}

/// Query the running plugin host for loaded manifests.
async fn query_host_list(socket_path: &Path) -> Result<Vec<serde_json::Value>> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    let stream = UnixStream::connect(socket_path).await?;
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    let req = serde_json::json!({
        "method": "plugin.list",
        "params": {},
        "id": 1
    });

    writer
        .write_all(format!("{}\n", serde_json::to_string(&req)?).as_bytes())
        .await?;

    let mut line = String::new();
    reader.read_line(&mut line).await?;

    let resp: serde_json::Value = serde_json::from_str(&line)?;
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or(serde_json::Value::Array(vec![]));

    let manifests: Vec<serde_json::Value> = serde_json::from_value(result)?;
    Ok(manifests)
}

/// Remove an installed plugin.
async fn cmd_uninstall(plugin_id: &str, data_dir: &Path, socket_path: &Path) -> Result<()> {
    let db_path = data_dir.join("corvid-agent.db");
    if !db_path.exists() {
        bail!("no plugins installed (database not found)");
    }

    let db = rusqlite::Connection::open(&db_path)?;

    // Check plugin exists
    let exists: bool = db
        .query_row(
            "SELECT COUNT(*) FROM plugins WHERE id = ?1",
            [plugin_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|c| c > 0)
        .unwrap_or(false);

    if !exists {
        bail!("plugin '{plugin_id}' is not installed");
    }

    // Signal host to unload first
    if socket_path.exists() {
        match signal_host_unload(socket_path, plugin_id).await {
            Ok(()) => println!("Plugin host notified — unloading {plugin_id}"),
            Err(e) => println!("Warning: could not notify plugin host: {e}"),
        }
    }

    // Remove from DB
    db.execute("DELETE FROM plugins WHERE id = ?1", [plugin_id])?;

    // Remove WASM file(s) if they exist
    let plugins_dir = data_dir.join("plugins");
    if plugins_dir.exists() {
        // Remove any .wasm files that match the plugin ID pattern
        if let Ok(entries) = std::fs::read_dir(&plugins_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let id_underscore = plugin_id.replace('-', "_");
                if name.contains(&id_underscore) && name.ends_with(".wasm") {
                    let _ = std::fs::remove_file(entry.path());
                    println!("Removed {}", entry.path().display());
                }
            }
        }
    }

    println!("Uninstalled {plugin_id}");
    Ok(())
}

/// Send JSON-RPC unload signal to the running plugin host.
async fn signal_host_unload(socket_path: &Path, plugin_id: &str) -> Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    let stream = UnixStream::connect(socket_path).await?;
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);

    let req = serde_json::json!({
        "method": "plugin.unload",
        "params": { "id": plugin_id },
        "id": 1
    });

    writer
        .write_all(format!("{}\n", serde_json::to_string(&req)?).as_bytes())
        .await?;

    let mut line = String::new();
    reader.read_line(&mut line).await?;

    let resp: serde_json::Value = serde_json::from_str(&line)?;
    if let Some(err) = resp.get("error").and_then(|e| e.as_str()) {
        bail!("host returned error: {err}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_source_valid() {
        let (owner, repo, version) =
            parse_source("CorvidLabs/corvid-plugin-algo-oracle@0.3.1").unwrap();
        assert_eq!(owner, "CorvidLabs");
        assert_eq!(repo, "corvid-plugin-algo-oracle");
        assert_eq!(version, "0.3.1");
    }

    #[test]
    fn parse_source_invalid() {
        assert!(parse_source("no-at-sign").is_err());
        assert!(parse_source("no-slash@1.0").is_err());
        assert!(parse_source("/repo@1.0").is_err());
        assert!(parse_source("owner/@1.0").is_err());
        assert!(parse_source("owner/repo@").is_err());
    }

    #[test]
    fn parse_tier_valid() {
        assert_eq!(parse_tier("trusted").unwrap(), TrustTier::Trusted);
        assert_eq!(parse_tier("Verified").unwrap(), TrustTier::Verified);
        assert_eq!(parse_tier("UNTRUSTED").unwrap(), TrustTier::Untrusted);
    }

    #[test]
    fn parse_tier_invalid() {
        assert!(parse_tier("invalid").is_err());
    }

    #[test]
    fn resolve_data_dir_expands_tilde() {
        let dir = resolve_data_dir("~/.corvid");
        // Should not start with ~ if HOME is set
        if std::env::var("HOME").is_ok() {
            assert!(!dir.to_string_lossy().starts_with('~'));
        }
    }
}
