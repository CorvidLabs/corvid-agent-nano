//! Template generation for new plugin projects.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use corvid_plugin_sdk::TrustTier;

/// Generate a complete plugin project directory from template.
pub fn scaffold(name: &str, author: &str, tier: TrustTier) -> Result<PathBuf> {
    // Validate name: lowercase, alphanumeric + hyphens
    let name_re = regex::Regex::new(r"^[a-z][a-z0-9-]{0,49}$").unwrap();
    if !name_re.is_match(name) {
        bail!("plugin name must match ^[a-z][a-z0-9-]{{0,49}}$, got '{name}'");
    }

    let dir_name = format!("corvid-plugin-{name}");
    let dir = PathBuf::from(&dir_name);

    if dir.exists() {
        bail!("directory '{dir_name}' already exists");
    }

    fs::create_dir_all(dir.join("src")).context("failed to create project directory")?;
    fs::create_dir_all(dir.join(".github/workflows"))
        .context("failed to create .github/workflows")?;

    // Cargo.toml
    let crate_name = name.replace('-', "_");
    let tier_str = match tier {
        TrustTier::Trusted => "Trusted",
        TrustTier::Verified => "Verified",
        TrustTier::Untrusted => "Untrusted",
    };

    fs::write(
        dir.join("Cargo.toml"),
        generate_cargo_toml(name, &crate_name),
    )?;

    // plugin.toml
    fs::write(
        dir.join("plugin.toml"),
        generate_plugin_toml(name, &crate_name, tier_str),
    )?;

    // src/lib.rs
    fs::write(
        dir.join("src/lib.rs"),
        generate_lib_rs(name, author, tier_str),
    )?;

    // .github/workflows/release.yml
    fs::write(
        dir.join(".github/workflows/release.yml"),
        generate_release_yml(name, &crate_name),
    )?;

    Ok(dir)
}

fn generate_cargo_toml(name: &str, _crate_name: &str) -> String {
    format!(
        r#"[package]
name = "corvid-plugin-{name}"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]

[dependencies]
corvid-plugin-sdk    = "0.1"
corvid-plugin-macros = "0.1"

[profile.release]
opt-level = "z"
lto       = true
strip     = true

[features]
dev-mode = []
"#
    )
}

fn generate_plugin_toml(name: &str, crate_name: &str, tier: &str) -> String {
    format!(
        r#"[plugin]
id          = "corvid-{name}"
version     = "0.1.0"
trust-tier  = "{tier}"
sdk-version = "^0.1"

[build]
target        = "wasm32-wasip1"
wasm-artifact = "corvid_{crate_name}.wasm"
"#
    )
}

fn generate_lib_rs(name: &str, author: &str, tier: &str) -> String {
    let struct_name = name
        .split('-')
        .map(|s| {
            let mut c = s.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().to_string() + c.as_str(),
            }
        })
        .collect::<String>();

    format!(
        r#"use corvid_plugin_sdk::{{CorvidPlugin, InitContext, PluginError, PluginManifest, TrustTier}};
use corvid_plugin_macros::corvid_plugin;

#[corvid_plugin]
pub struct {struct_name};

impl CorvidPlugin for {struct_name} {{
    fn manifest() -> PluginManifest {{
        PluginManifest {{
            id: "corvid-{name}".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            author: "{author}".into(),
            description: "A corvid-agent plugin".into(),
            capabilities: vec![],
            event_filter: vec![],
            trust_tier: TrustTier::{tier},
            min_host_version: "0.1.0".into(),
            dependencies: vec![],
        }}
    }}

    fn tools(&self) -> &[Box<dyn corvid_plugin_sdk::PluginTool>] {{
        &[]
    }}

    fn init(&mut self, _ctx: InitContext) -> Result<(), PluginError> {{
        Ok(())
    }}
}}
"#
    )
}

fn generate_release_yml(_name: &str, crate_name: &str) -> String {
    format!(
        r#"name: Release

on:
  push:
    tags: ["v*"]

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: wasm32-wasip1

      - name: Build WASM
        run: cargo build --release --target wasm32-wasip1

      - name: Upload Release
        uses: softprops/action-gh-release@v2
        with:
          files: |
            target/wasm32-wasip1/release/corvid_{crate_name}.wasm
            plugin.toml
"#
    )
}

/// Scaffold into a specific parent directory (for testing).
pub fn scaffold_in(parent: &Path, name: &str, author: &str, tier: TrustTier) -> Result<PathBuf> {
    let name_re = regex::Regex::new(r"^[a-z][a-z0-9-]{0,49}$").unwrap();
    if !name_re.is_match(name) {
        bail!("plugin name must match ^[a-z][a-z0-9-]{{0,49}}$, got '{name}'");
    }

    let dir_name = format!("corvid-plugin-{name}");
    let dir = parent.join(&dir_name);

    if dir.exists() {
        bail!("directory '{dir_name}' already exists");
    }

    fs::create_dir_all(dir.join("src"))?;
    fs::create_dir_all(dir.join(".github/workflows"))?;

    let crate_name = name.replace('-', "_");
    let tier_str = match tier {
        TrustTier::Trusted => "Trusted",
        TrustTier::Verified => "Verified",
        TrustTier::Untrusted => "Untrusted",
    };

    fs::write(
        dir.join("Cargo.toml"),
        generate_cargo_toml(name, &crate_name),
    )?;
    fs::write(
        dir.join("plugin.toml"),
        generate_plugin_toml(name, &crate_name, tier_str),
    )?;
    fs::write(
        dir.join("src/lib.rs"),
        generate_lib_rs(name, author, tier_str),
    )?;
    fs::write(
        dir.join(".github/workflows/release.yml"),
        generate_release_yml(name, &crate_name),
    )?;

    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn scaffold_creates_expected_structure() {
        let tmp = TempDir::new().unwrap();
        let dir = scaffold_in(
            tmp.path(),
            "algo-watcher",
            "CorvidLabs",
            TrustTier::Verified,
        )
        .unwrap();

        assert!(dir.join("Cargo.toml").exists());
        assert!(dir.join("plugin.toml").exists());
        assert!(dir.join("src/lib.rs").exists());
        assert!(dir.join(".github/workflows/release.yml").exists());

        // Verify Cargo.toml content
        let cargo = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap();
        assert!(cargo.contains("corvid-plugin-algo-watcher"));
        assert!(cargo.contains("cdylib"));
        assert!(cargo.contains("corvid-plugin-sdk"));

        // Verify plugin.toml
        let plugin = std::fs::read_to_string(dir.join("plugin.toml")).unwrap();
        assert!(plugin.contains("corvid-algo-watcher"));
        assert!(plugin.contains("Verified"));

        // Verify lib.rs
        let lib = std::fs::read_to_string(dir.join("src/lib.rs")).unwrap();
        assert!(lib.contains("AlgoWatcher"));
        assert!(lib.contains("CorvidLabs"));
    }

    #[test]
    fn scaffold_rejects_invalid_name() {
        let tmp = TempDir::new().unwrap();
        assert!(scaffold_in(tmp.path(), "INVALID", "author", TrustTier::Untrusted).is_err());
        assert!(scaffold_in(tmp.path(), "0bad", "author", TrustTier::Untrusted).is_err());
        assert!(scaffold_in(tmp.path(), "-bad", "author", TrustTier::Untrusted).is_err());
    }

    #[test]
    fn scaffold_rejects_existing_dir() {
        let tmp = TempDir::new().unwrap();
        scaffold_in(tmp.path(), "my-plugin", "author", TrustTier::Untrusted).unwrap();
        // Second time should fail
        assert!(scaffold_in(tmp.path(), "my-plugin", "author", TrustTier::Untrusted).is_err());
    }
}
