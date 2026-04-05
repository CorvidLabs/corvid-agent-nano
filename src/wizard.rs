//! Interactive setup wizard for first-run experience.
//!
//! Guides users through network selection, wallet creation/import,
//! and provides next-step instructions. All steps can be driven by
//! CLI flags for non-interactive (CI) use.

use anyhow::{bail, Result};
use dialoguer::{theme::ColorfulTheme, Password, Select};
use zeroize::Zeroize;

use crate::keystore;
use crate::wallet;

/// Configuration for the setup wizard, populated from CLI flags.
/// `None` values trigger interactive prompts.
pub struct WizardConfig {
    pub network: Option<crate::Network>,
    pub generate: bool,
    pub import_mnemonic: Option<String>,
    pub import_seed: Option<String>,
    pub password: Option<String>,
    pub data_dir: String,
}

/// Result of a completed wizard run.
#[derive(Debug)]
#[allow(dead_code)]
pub struct WizardResult {
    pub network: crate::Network,
    pub address: String,
    pub keystore_path: String,
}

/// Run the interactive setup wizard.
///
/// If all required fields are provided in `config`, runs non-interactively.
/// Otherwise, prompts the user for missing information.
pub fn run_wizard(config: WizardConfig) -> Result<WizardResult> {
    let data_path = std::path::Path::new(&config.data_dir);
    std::fs::create_dir_all(data_path)?;

    let ks_path = crate::keystore_path(&config.data_dir);
    if keystore::keystore_exists(&ks_path) {
        bail!(
            "Wallet already exists at {}. Delete it first or use `can import`.",
            ks_path.display()
        );
    }

    // ── Step 1: Network selection ──────────────────────────────────
    let network = select_network(config.network)?;

    // ── Step 2: Wallet generation or import ────────────────────────
    let (mut seed, address, mnemonic) = create_or_import_wallet(
        config.generate,
        config.import_mnemonic,
        config.import_seed,
    )?;

    // Display wallet info
    if let Some(ref m) = mnemonic {
        println!("\n  Generated new Algorand wallet");
        println!("  Network: {}", network);
        println!("  Address: {}\n", address);

        println!("  IMPORTANT: Write down your recovery phrase and store it safely.");
        println!("  ---");
        let words: Vec<&str> = m.split_whitespace().collect();
        for (i, word) in words.iter().enumerate() {
            print!("  {:>2}. {:<12}", i + 1, word);
            if (i + 1) % 5 == 0 {
                println!();
            }
        }
        println!("  ---\n");
    } else {
        println!("\n  Imported wallet");
        println!("  Network: {}", network);
        println!("  Address: {}\n", address);
    }

    // ── Step 3: Password ───────────────────────────────────────────
    let pw = get_password(config.password)?;

    // ── Step 4: Save keystore ──────────────────────────────────────
    keystore::create_keystore(&seed, &address, &pw, &ks_path)?;
    seed.zeroize();

    println!("  Wallet encrypted and saved to {}", ks_path.display());

    // ── Step 5: Next steps ─────────────────────────────────────────
    print_next_steps(network, &address);

    Ok(WizardResult {
        network,
        address,
        keystore_path: ks_path.display().to_string(),
    })
}

/// Select network interactively or from CLI flag.
fn select_network(preset: Option<crate::Network>) -> Result<crate::Network> {
    if let Some(n) = preset {
        return Ok(n);
    }

    let selections = &[
        "Localnet  — local sandbox (default for development)",
        "Testnet   — Algorand TestNet (recommended for getting started)",
        "Mainnet   — Algorand MainNet (real ALGO)",
    ];

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select network")
        .items(selections)
        .default(0)
        .interact()?;

    Ok(match selection {
        0 => crate::Network::Localnet,
        1 => crate::Network::Testnet,
        2 => crate::Network::Mainnet,
        _ => unreachable!(),
    })
}

/// Create a new wallet or import an existing one.
/// Returns (seed, address, Option<mnemonic>). Mnemonic is Some for generated wallets.
fn create_or_import_wallet(
    generate: bool,
    import_mnemonic: Option<String>,
    import_seed: Option<String>,
) -> Result<([u8; 32], String, Option<String>)> {
    // Non-interactive: explicit flags
    if generate {
        let seed = wallet::generate_seed();
        let address = wallet::address_from_seed(&seed);
        let mnemonic = wallet::seed_to_mnemonic(&seed);
        return Ok((seed, address, Some(mnemonic)));
    }

    if let Some(m) = import_mnemonic {
        let seed = wallet::mnemonic_to_seed(&m)?;
        let address = wallet::address_from_seed(&seed);
        return Ok((seed, address, None));
    }

    if let Some(s) = import_seed {
        let bytes = hex::decode(&s).map_err(|e| anyhow::anyhow!("Invalid hex: {}", e))?;
        if bytes.len() != 32 {
            bail!("Seed must be 32 bytes (64 hex chars), got {}", bytes.len());
        }
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&bytes);
        let address = wallet::address_from_seed(&seed);
        return Ok((seed, address, None));
    }

    // Interactive: ask user
    let selections = &[
        "Generate new wallet",
        "Import from mnemonic (25 words)",
        "Import from hex seed",
    ];

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Generate a new wallet or import existing?")
        .items(selections)
        .default(0)
        .interact()?;

    match selection {
        0 => {
            let seed = wallet::generate_seed();
            let address = wallet::address_from_seed(&seed);
            let mnemonic = wallet::seed_to_mnemonic(&seed);
            Ok((seed, address, Some(mnemonic)))
        }
        1 => {
            let mnemonic: String = dialoguer::Input::with_theme(&ColorfulTheme::default())
                .with_prompt("Enter your 25-word mnemonic")
                .interact_text()?;
            let seed = wallet::mnemonic_to_seed(&mnemonic)?;
            let address = wallet::address_from_seed(&seed);
            Ok((seed, address, None))
        }
        2 => {
            let seed_hex: String = dialoguer::Input::with_theme(&ColorfulTheme::default())
                .with_prompt("Enter hex-encoded seed (64 chars)")
                .interact_text()?;
            let bytes =
                hex::decode(&seed_hex).map_err(|e| anyhow::anyhow!("Invalid hex: {}", e))?;
            if bytes.len() != 32 {
                bail!("Seed must be 32 bytes (64 hex chars), got {}", bytes.len());
            }
            let mut seed = [0u8; 32];
            seed.copy_from_slice(&bytes);
            let address = wallet::address_from_seed(&seed);
            Ok((seed, address, None))
        }
        _ => unreachable!(),
    }
}

/// Get password interactively or from CLI flag.
fn get_password(password: Option<String>) -> Result<String> {
    if let Some(p) = password {
        if p.len() < 8 {
            bail!("Password must be at least 8 characters");
        }
        return Ok(p);
    }

    loop {
        let p1 = Password::with_theme(&ColorfulTheme::default())
            .with_prompt("Enter a password to encrypt your wallet (min 8 chars)")
            .interact()?;

        if p1.len() < 8 {
            eprintln!("  Password must be at least 8 characters. Try again.");
            continue;
        }

        let p2 = Password::with_theme(&ColorfulTheme::default())
            .with_prompt("Confirm password")
            .interact()?;

        if p1 != p2 {
            eprintln!("  Passwords don't match. Try again.");
            continue;
        }

        return Ok(p1);
    }
}

/// Print network-specific next steps.
fn print_next_steps(network: crate::Network, address: &str) {
    println!();
    match network {
        crate::Network::Localnet => {
            println!("  Next steps:");
            println!("    1. Fund your agent:     can fund");
            println!("    2. Register with hub:   can register");
            println!("    3. Add a contact:       can contacts add --name <name> --address <addr> --psk <key>");
            println!("    4. Start the agent:     can run");
        }
        crate::Network::Testnet => {
            println!("  Fund your agent:");
            println!("    Send ALGO to: {}", address);
            println!("    Testnet dispenser: https://bank.testnet.algorand.network");
            println!();
            println!("  Then:");
            println!("    1. Register with hub:   can register");
            println!("    2. Add a contact:       can contacts add --name <name> --address <addr> --psk <key>");
            println!("    3. Start the agent:     can run --network testnet");
        }
        crate::Network::Mainnet => {
            println!("  Fund your agent:");
            println!("    Send ALGO to: {}", address);
            println!();
            println!("  Then:");
            println!("    1. Register with hub:   can register");
            println!("    2. Add a contact:       can contacts add --name <name> --address <addr> --psk <key>");
            println!("    3. Start the agent:     can run --network mainnet");
        }
    }
}

/// Check if the agent is set up and print guidance if not.
/// Returns true if setup is complete, false if missing.
pub fn check_first_run(data_dir: &str) -> bool {
    let ks_path = crate::keystore_path(data_dir);
    if !keystore::keystore_exists(&ks_path) {
        eprintln!("No wallet configured.");
        eprintln!("Run `can init` to set up your agent.\n");
        return false;
    }
    true
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn wizard_generate_non_interactive() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().to_str().unwrap().to_string();

        let config = WizardConfig {
            network: Some(crate::Network::Localnet),
            generate: true,
            import_mnemonic: None,
            import_seed: None,
            password: Some("testpassword123".to_string()),
            data_dir: data_dir.clone(),
        };

        let result = run_wizard(config).unwrap();

        // Verify result
        assert_eq!(result.address.len(), 58, "Algorand address should be 58 chars");
        assert!(
            result.keystore_path.contains("keystore.enc"),
            "Keystore path should contain keystore.enc"
        );

        // Verify keystore was actually created and is readable
        let ks_path = crate::keystore_path(&data_dir);
        assert!(keystore::keystore_exists(&ks_path));

        // Verify we can decrypt with the password
        let (seed, addr) = keystore::load_keystore(&ks_path, "testpassword123").unwrap();
        assert_eq!(addr, result.address);
        assert_eq!(seed.len(), 32);

        // Verify wrong password fails
        assert!(keystore::load_keystore(&ks_path, "wrongpassword1").is_err());
    }

    #[test]
    fn wizard_import_mnemonic_non_interactive() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().to_str().unwrap().to_string();

        // Generate a mnemonic to import
        let seed = wallet::generate_seed();
        let address = wallet::address_from_seed(&seed);
        let mnemonic = wallet::seed_to_mnemonic(&seed);

        let config = WizardConfig {
            network: Some(crate::Network::Testnet),
            generate: false,
            import_mnemonic: Some(mnemonic),
            import_seed: None,
            password: Some("importpass123".to_string()),
            data_dir: data_dir.clone(),
        };

        let result = run_wizard(config).unwrap();
        assert_eq!(result.address, address, "Imported address should match original");

        // Verify keystore roundtrip
        let ks_path = crate::keystore_path(&data_dir);
        let (recovered_seed, recovered_addr) =
            keystore::load_keystore(&ks_path, "importpass123").unwrap();
        assert_eq!(recovered_seed, seed);
        assert_eq!(recovered_addr, address);
    }

    #[test]
    fn wizard_import_hex_seed_non_interactive() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().to_str().unwrap().to_string();

        let seed = wallet::generate_seed();
        let address = wallet::address_from_seed(&seed);
        let seed_hex = hex::encode(seed);

        let config = WizardConfig {
            network: Some(crate::Network::Mainnet),
            generate: false,
            import_mnemonic: None,
            import_seed: Some(seed_hex),
            password: Some("hexseedpass1".to_string()),
            data_dir: data_dir.clone(),
        };

        let result = run_wizard(config).unwrap();
        assert_eq!(result.address, address);

        // Verify keystore roundtrip
        let ks_path = crate::keystore_path(&data_dir);
        let (recovered_seed, _) = keystore::load_keystore(&ks_path, "hexseedpass1").unwrap();
        assert_eq!(recovered_seed, seed);
    }

    #[test]
    fn wizard_rejects_existing_keystore() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().to_str().unwrap().to_string();

        // First run succeeds
        let config = WizardConfig {
            network: Some(crate::Network::Localnet),
            generate: true,
            import_mnemonic: None,
            import_seed: None,
            password: Some("testpassword123".to_string()),
            data_dir: data_dir.clone(),
        };
        run_wizard(config).unwrap();

        // Second run fails
        let config2 = WizardConfig {
            network: Some(crate::Network::Localnet),
            generate: true,
            import_mnemonic: None,
            import_seed: None,
            password: Some("testpassword123".to_string()),
            data_dir,
        };
        let err = run_wizard(config2).unwrap_err();
        assert!(
            err.to_string().contains("already exists"),
            "Should reject when keystore already exists: {}",
            err
        );
    }

    #[test]
    fn wizard_rejects_short_password() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().to_str().unwrap().to_string();

        let config = WizardConfig {
            network: Some(crate::Network::Localnet),
            generate: true,
            import_mnemonic: None,
            import_seed: None,
            password: Some("short".to_string()), // < 8 chars
            data_dir,
        };
        let err = run_wizard(config).unwrap_err();
        assert!(
            err.to_string().contains("8 characters"),
            "Should reject short password: {}",
            err
        );
    }

    #[test]
    fn wizard_rejects_invalid_hex_seed() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().to_str().unwrap().to_string();

        let config = WizardConfig {
            network: Some(crate::Network::Localnet),
            generate: false,
            import_mnemonic: None,
            import_seed: Some("not_valid_hex".to_string()),
            password: Some("testpassword123".to_string()),
            data_dir,
        };
        assert!(run_wizard(config).is_err());
    }

    #[test]
    fn wizard_rejects_wrong_length_seed() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().to_str().unwrap().to_string();

        // 16 bytes instead of 32
        let config = WizardConfig {
            network: Some(crate::Network::Localnet),
            generate: false,
            import_mnemonic: None,
            import_seed: Some("aa".repeat(16)), // 16 bytes = 32 hex chars, but we need 64
            password: Some("testpassword123".to_string()),
            data_dir,
        };
        let err = run_wizard(config).unwrap_err();
        assert!(
            err.to_string().contains("32 bytes"),
            "Should reject wrong-length seed: {}",
            err
        );
    }

    #[test]
    fn wizard_rejects_invalid_mnemonic() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().to_str().unwrap().to_string();

        let config = WizardConfig {
            network: Some(crate::Network::Localnet),
            generate: false,
            import_mnemonic: Some("not a valid mnemonic phrase".to_string()),
            import_seed: None,
            password: Some("testpassword123".to_string()),
            data_dir,
        };
        assert!(run_wizard(config).is_err());
    }

    #[test]
    fn wizard_creates_data_directory() {
        let tmp = TempDir::new().unwrap();
        let nested_dir = tmp.path().join("nested").join("deep").join("data");
        let data_dir = nested_dir.to_str().unwrap().to_string();

        let config = WizardConfig {
            network: Some(crate::Network::Localnet),
            generate: true,
            import_mnemonic: None,
            import_seed: None,
            password: Some("testpassword123".to_string()),
            data_dir: data_dir.clone(),
        };

        run_wizard(config).unwrap();

        // Verify nested directories were created
        assert!(nested_dir.exists());
        assert!(crate::keystore_path(&data_dir).exists());
    }

    #[test]
    fn wizard_no_generate_no_import_fails() {
        let tmp = TempDir::new().unwrap();

        // Non-interactive mode with no generate/import flags should fail
        // because it would try to open interactive prompts which don't work in tests
        let config = WizardConfig {
            network: Some(crate::Network::Localnet),
            generate: false,
            import_mnemonic: None,
            import_seed: None,
            password: Some("testpassword123".to_string()),
            data_dir: tmp.path().to_str().unwrap().to_string(),
        };
        // This will try to open interactive prompts and fail in a test context
        assert!(run_wizard(config).is_err());
    }

    #[test]
    fn check_first_run_no_keystore() {
        let tmp = TempDir::new().unwrap();
        assert!(!check_first_run(tmp.path().to_str().unwrap()));
    }

    #[test]
    fn check_first_run_with_keystore() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path().to_str().unwrap();

        // Create a keystore
        let seed = wallet::generate_seed();
        let address = wallet::address_from_seed(&seed);
        let ks_path = crate::keystore_path(data_dir);
        keystore::create_keystore(&seed, &address, "testpassword123", &ks_path).unwrap();

        assert!(check_first_run(data_dir));
    }

    #[test]
    fn select_network_with_preset() {
        assert!(matches!(
            select_network(Some(crate::Network::Localnet)).unwrap(),
            crate::Network::Localnet
        ));
        assert!(matches!(
            select_network(Some(crate::Network::Testnet)).unwrap(),
            crate::Network::Testnet
        ));
        assert!(matches!(
            select_network(Some(crate::Network::Mainnet)).unwrap(),
            crate::Network::Mainnet
        ));
    }

    #[test]
    fn get_password_with_preset() {
        let pw = get_password(Some("validpassword".to_string())).unwrap();
        assert_eq!(pw, "validpassword");
    }

    #[test]
    fn get_password_rejects_short() {
        let err = get_password(Some("short".to_string())).unwrap_err();
        assert!(err.to_string().contains("8 characters"));
    }

    #[test]
    fn create_wallet_generate() {
        let (seed, address, mnemonic) = create_or_import_wallet(true, None, None).unwrap();
        assert_eq!(seed.len(), 32);
        assert_eq!(address.len(), 58);
        assert!(mnemonic.is_some());
        let mnemonic_str = mnemonic.unwrap();
        let words: Vec<&str> = mnemonic_str.split_whitespace().collect();
        assert_eq!(words.len(), 25, "Algorand mnemonic should be 25 words");
    }

    #[test]
    fn create_wallet_import_mnemonic() {
        let original_seed = wallet::generate_seed();
        let original_addr = wallet::address_from_seed(&original_seed);
        let mnemonic = wallet::seed_to_mnemonic(&original_seed);

        let (seed, address, m) =
            create_or_import_wallet(false, Some(mnemonic), None).unwrap();
        assert_eq!(seed, original_seed);
        assert_eq!(address, original_addr);
        assert!(m.is_none(), "Import should not return mnemonic");
    }

    #[test]
    fn create_wallet_import_hex_seed() {
        let original_seed = wallet::generate_seed();
        let original_addr = wallet::address_from_seed(&original_seed);
        let seed_hex = hex::encode(original_seed);

        let (seed, address, m) =
            create_or_import_wallet(false, None, Some(seed_hex)).unwrap();
        assert_eq!(seed, original_seed);
        assert_eq!(address, original_addr);
        assert!(m.is_none());
    }
}
