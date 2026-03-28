//! Manifest + capability validation for built plugins.
//!
//! Performs the same checks as the host loader but offline (no running host required).

use std::collections::HashSet;
use std::fmt;
use std::path::Path;

use anyhow::{Context, Result};
use corvid_plugin_sdk::{
    Capability, PluginManifest, TrustTier, ABI_MIN_COMPATIBLE, ABI_VERSION,
};

/// Result of validating a plugin.
#[derive(Debug)]
pub struct ValidationReport {
    pub manifest: Option<PluginManifest>,
    pub abi_version: Option<u32>,
    pub checks: Vec<Check>,
}

/// A single validation check result.
#[derive(Debug)]
pub struct Check {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

impl ValidationReport {
    /// Returns true if all checks passed.
    pub fn is_ok(&self) -> bool {
        self.checks.iter().all(|c| c.passed)
    }

    /// Count of failed checks.
    pub fn error_count(&self) -> usize {
        self.checks.iter().filter(|c| !c.passed).count()
    }
}

impl fmt::Display for ValidationReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(m) = &self.manifest {
            writeln!(f, "Plugin: {} v{}", m.id, m.version)?;
            writeln!(f, "Author: {}", m.author)?;
            writeln!(f, "Tier:   {:?}", m.trust_tier)?;
            if !m.capabilities.is_empty() {
                writeln!(
                    f,
                    "Caps:   {}",
                    m.capabilities
                        .iter()
                        .map(|c| c.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )?;
            }
            writeln!(f)?;
        }

        for check in &self.checks {
            let icon = if check.passed { "OK" } else { "FAIL" };
            writeln!(f, "  [{icon}] {}: {}", check.name, check.detail)?;
        }

        if self.is_ok() {
            writeln!(f, "\nValidation passed ({} checks)", self.checks.len())?;
        } else {
            writeln!(
                f,
                "\nValidation FAILED ({} errors / {} checks)",
                self.error_count(),
                self.checks.len()
            )?;
        }
        Ok(())
    }
}

/// Validate a built WASM plugin file.
///
/// This performs offline validation — it parses the WASM custom sections
/// to extract manifest data without instantiating the module.
/// For full ABI checking (calling `__corvid_abi_version`), the host is needed.
/// This function validates manifest structure, capabilities, and tool schemas.
pub fn validate_plugin(wasm_path: &Path) -> Result<ValidationReport> {
    let wasm_bytes = std::fs::read(wasm_path)
        .with_context(|| format!("failed to read {}", wasm_path.display()))?;

    let mut report = ValidationReport {
        manifest: None,
        abi_version: None,
        checks: Vec::new(),
    };

    // Check 1: File is valid WASM (starts with \0asm magic)
    let is_wasm = wasm_bytes.len() >= 4 && &wasm_bytes[0..4] == b"\0asm";
    report.checks.push(Check {
        name: "WASM format".into(),
        passed: is_wasm,
        detail: if is_wasm {
            format!("{} bytes, valid WASM header", wasm_bytes.len())
        } else {
            "not a valid WASM file (missing \\0asm header)".into()
        },
    });

    if !is_wasm {
        return Ok(report);
    }

    // Try to compile and extract manifest via the host's loader
    // We use a minimal Wasmtime engine for this
    match extract_and_validate(&wasm_bytes, &mut report) {
        Ok(()) => {}
        Err(e) => {
            report.checks.push(Check {
                name: "WASM compilation".into(),
                passed: false,
                detail: format!("failed: {e}"),
            });
        }
    }

    Ok(report)
}

fn extract_and_validate(wasm_bytes: &[u8], report: &mut ValidationReport) -> Result<()> {
    use corvid_plugin_host::engine::build_engine;
    use corvid_plugin_host::loader;

    let tmp_cache = std::env::temp_dir().join("corvid-plugin-validate-cache");
    let engine = build_engine(&tmp_cache)?;

    // Try loading via the host's pipeline (ABI check + manifest extraction)
    match loader::load_plugin(&engine, wasm_bytes, TrustTier::Untrusted) {
        Ok(loaded) => {
            report.abi_version = Some(ABI_VERSION); // passed ABI check
            report.checks.push(Check {
                name: "ABI version".into(),
                passed: true,
                detail: format!("compatible (host range [{}, {}])", ABI_MIN_COMPATIBLE, ABI_VERSION),
            });

            validate_manifest_fields(&loaded.manifest, report);
            report.manifest = Some(loaded.manifest);
        }
        Err(e) => {
            let err_str = format!("{e}");
            // Try to distinguish ABI vs manifest errors
            if err_str.contains("ABI") || err_str.contains("abi") {
                report.checks.push(Check {
                    name: "ABI version".into(),
                    passed: false,
                    detail: err_str,
                });
            } else if err_str.contains("manifest") || err_str.contains("Manifest") {
                report.checks.push(Check {
                    name: "ABI version".into(),
                    passed: true,
                    detail: "OK".into(),
                });
                report.checks.push(Check {
                    name: "Manifest".into(),
                    passed: false,
                    detail: err_str,
                });
            } else {
                report.checks.push(Check {
                    name: "WASM load".into(),
                    passed: false,
                    detail: err_str,
                });
            }
        }
    }

    Ok(())
}

fn validate_manifest_fields(m: &PluginManifest, report: &mut ValidationReport) {
    // Check 2: Plugin ID format
    let id_re = regex::Regex::new(r"^[a-z][a-z0-9-]{0,49}$").unwrap();
    report.checks.push(Check {
        name: "Plugin ID".into(),
        passed: id_re.is_match(&m.id),
        detail: if id_re.is_match(&m.id) {
            format!("'{}'", m.id)
        } else {
            format!("'{}' does not match ^[a-z][a-z0-9-]{{0,49}}$", m.id)
        },
    });

    // Check 3: Version is valid semver
    let ver_ok = semver::Version::parse(&m.version).is_ok();
    report.checks.push(Check {
        name: "Version".into(),
        passed: ver_ok,
        detail: if ver_ok {
            m.version.clone()
        } else {
            format!("'{}' is not valid semver", m.version)
        },
    });

    // Check 4: min_host_version is valid semver
    let min_ok = semver::Version::parse(&m.min_host_version).is_ok();
    report.checks.push(Check {
        name: "Min host version".into(),
        passed: min_ok,
        detail: if min_ok {
            m.min_host_version.clone()
        } else {
            format!("'{}' is not valid semver", m.min_host_version)
        },
    });

    // Check 5: No duplicate tool names (checked via capabilities for now)
    // Tools come from the PluginTool trait at runtime — we can't check them offline
    // without instantiating. We check capabilities instead.

    // Check 6: Capabilities within tier limits
    validate_capabilities_for_tier(&m.capabilities, m.trust_tier, report);
}

fn validate_capabilities_for_tier(
    capabilities: &[Capability],
    tier: TrustTier,
    report: &mut ValidationReport,
) {
    // Check for duplicate capabilities
    let mut seen = HashSet::new();
    let mut duplicates = Vec::new();
    for cap in capabilities {
        let key = cap.to_string();
        if !seen.insert(key.clone()) {
            duplicates.push(key);
        }
    }

    report.checks.push(Check {
        name: "Capability uniqueness".into(),
        passed: duplicates.is_empty(),
        detail: if duplicates.is_empty() {
            format!("{} capabilities declared", capabilities.len())
        } else {
            format!("duplicates: {}", duplicates.join(", "))
        },
    });

    // Untrusted plugins cannot have Network or FsProjectDir
    if tier == TrustTier::Untrusted {
        let has_restricted = capabilities.iter().any(|c| {
            matches!(c, Capability::Network { .. } | Capability::FsProjectDir)
        });

        report.checks.push(Check {
            name: "Tier capability check".into(),
            passed: !has_restricted,
            detail: if has_restricted {
                "Untrusted plugins cannot declare Network or FsProjectDir".into()
            } else {
                format!("{:?} tier — all capabilities allowed", tier)
            },
        });
    } else {
        report.checks.push(Check {
            name: "Tier capability check".into(),
            passed: true,
            detail: format!("{:?} tier — all capabilities allowed", tier),
        });
    }
}

/// Validate a manifest from a plugin.toml file (offline, no WASM needed).
pub fn validate_plugin_toml(toml_path: &Path) -> Result<ValidationReport> {
    let content = std::fs::read_to_string(toml_path)
        .with_context(|| format!("failed to read {}", toml_path.display()))?;

    let parsed: toml::Value = toml::from_str(&content)
        .with_context(|| format!("failed to parse {}", toml_path.display()))?;

    let mut report = ValidationReport {
        manifest: None,
        abi_version: None,
        checks: Vec::new(),
    };

    // Extract [plugin] section
    let plugin = parsed.get("plugin");
    report.checks.push(Check {
        name: "TOML structure".into(),
        passed: plugin.is_some(),
        detail: if plugin.is_some() {
            "[plugin] section found".into()
        } else {
            "missing [plugin] section".into()
        },
    });

    if let Some(plugin) = plugin {
        // Check required fields
        for field in &["id", "version", "trust-tier", "sdk-version"] {
            let present = plugin.get(field).is_some();
            report.checks.push(Check {
                name: format!("Field: {field}"),
                passed: present,
                detail: if present {
                    format!("{}", plugin.get(field).unwrap())
                } else {
                    format!("missing required field '{field}'")
                },
            });
        }

        // Validate ID format if present
        if let Some(id) = plugin.get("id").and_then(|v| v.as_str()) {
            let id_re = regex::Regex::new(r"^[a-z][a-z0-9-]{0,49}$").unwrap();
            report.checks.push(Check {
                name: "ID format".into(),
                passed: id_re.is_match(id),
                detail: if id_re.is_match(id) {
                    format!("'{id}' OK")
                } else {
                    format!("'{id}' does not match ^[a-z][a-z0-9-]{{0,49}}$")
                },
            });
        }

        // Validate version is semver
        if let Some(ver) = plugin.get("version").and_then(|v| v.as_str()) {
            let ok = semver::Version::parse(ver).is_ok();
            report.checks.push(Check {
                name: "Version semver".into(),
                passed: ok,
                detail: if ok {
                    format!("{ver} OK")
                } else {
                    format!("'{ver}' is not valid semver")
                },
            });
        }
    }

    // Check [build] section
    let build = parsed.get("build");
    report.checks.push(Check {
        name: "Build section".into(),
        passed: build.is_some(),
        detail: if build.is_some() {
            "[build] section found".into()
        } else {
            "missing [build] section (optional)".into()
        },
    });

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn validate_rejects_non_wasm_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("not-wasm.wasm");
        std::fs::write(&path, b"this is not wasm").unwrap();

        let report = validate_plugin(&path).unwrap();
        assert!(!report.is_ok());
        assert!(report.checks[0].name == "WASM format");
        assert!(!report.checks[0].passed);
    }

    #[test]
    fn validate_plugin_toml_valid() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("plugin.toml");
        std::fs::write(
            &path,
            r#"[plugin]
id = "corvid-test"
version = "0.1.0"
trust-tier = "Untrusted"
sdk-version = "^0.1"

[build]
target = "wasm32-wasip1"
wasm-artifact = "corvid_test.wasm"
"#,
        )
        .unwrap();

        let report = validate_plugin_toml(&path).unwrap();
        assert!(report.is_ok());
    }

    #[test]
    fn validate_plugin_toml_missing_fields() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("plugin.toml");
        std::fs::write(&path, "[plugin]\nid = \"test\"\n").unwrap();

        let report = validate_plugin_toml(&path).unwrap();
        assert!(!report.is_ok());
    }

    #[test]
    fn validate_plugin_toml_bad_id() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("plugin.toml");
        std::fs::write(
            &path,
            r#"[plugin]
id = "INVALID"
version = "0.1.0"
trust-tier = "Untrusted"
sdk-version = "^0.1"
"#,
        )
        .unwrap();

        let report = validate_plugin_toml(&path).unwrap();
        // ID format check should fail
        let id_check = report.checks.iter().find(|c| c.name == "ID format").unwrap();
        assert!(!id_check.passed);
    }

    #[test]
    fn capability_tier_validation() {
        let mut report = ValidationReport {
            manifest: None,
            abi_version: None,
            checks: Vec::new(),
        };

        // Untrusted with Network should fail
        validate_capabilities_for_tier(
            &[Capability::Network {
                allowlist: vec!["api.example.com".into()],
            }],
            TrustTier::Untrusted,
            &mut report,
        );
        let tier_check = report.checks.iter().find(|c| c.name == "Tier capability check").unwrap();
        assert!(!tier_check.passed);
    }

    #[test]
    fn capability_tier_verified_allows_network() {
        let mut report = ValidationReport {
            manifest: None,
            abi_version: None,
            checks: Vec::new(),
        };

        validate_capabilities_for_tier(
            &[Capability::Network {
                allowlist: vec!["api.example.com".into()],
            }],
            TrustTier::Verified,
            &mut report,
        );
        let tier_check = report.checks.iter().find(|c| c.name == "Tier capability check").unwrap();
        assert!(tier_check.passed);
    }

    #[test]
    fn report_display() {
        let report = ValidationReport {
            manifest: Some(PluginManifest {
                id: "test-plugin".into(),
                version: "1.0.0".into(),
                author: "corvid".into(),
                description: "test".into(),
                capabilities: vec![Capability::AlgoRead],
                event_filter: vec![],
                trust_tier: TrustTier::Verified,
                min_host_version: "0.1.0".into(),
            }),
            abi_version: Some(1),
            checks: vec![
                Check { name: "ABI".into(), passed: true, detail: "v1".into() },
                Check { name: "ID".into(), passed: true, detail: "test-plugin".into() },
            ],
        };

        let output = format!("{report}");
        assert!(output.contains("test-plugin"));
        assert!(output.contains("[OK]"));
        assert!(output.contains("Validation passed"));
    }
}
