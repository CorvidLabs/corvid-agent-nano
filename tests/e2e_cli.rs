//! CLI binary smoke tests for `can`.
//!
//! These tests build and invoke the `can` binary as a subprocess to verify
//! that commands parse correctly and produce expected output.

use std::process::Command;

#[test]
fn cli_help_exits_zero() {
    let output = Command::new(env!("CARGO_BIN_EXE_can"))
        .arg("--help")
        .output()
        .expect("failed to run can --help");

    assert!(output.status.success(), "can --help should exit 0");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Corvid Agent CAN"),
        "help output should contain program name"
    );
}

#[test]
fn cli_unknown_command_exits_nonzero() {
    let output = Command::new(env!("CARGO_BIN_EXE_can"))
        .arg("nonexistent-command")
        .output()
        .expect("failed to run can");

    assert!(
        !output.status.success(),
        "unknown command should exit non-zero"
    );
}

#[test]
fn cli_setup_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_can"))
        .args(["setup", "--help"])
        .output()
        .expect("failed to run can setup --help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("setup") || stdout.contains("wizard") || stdout.contains("wallet"));
}

#[test]
fn cli_send_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_can"))
        .args(["send", "--help"])
        .output()
        .expect("failed to run can send --help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("send") || stdout.contains("message"));
}

#[test]
fn cli_contacts_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_can"))
        .args(["contacts", "--help"])
        .output()
        .expect("failed to run can contacts --help");

    assert!(output.status.success());
}

#[test]
fn cli_run_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_can"))
        .args(["run", "--help"])
        .output()
        .expect("failed to run can run --help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("runtime") || stdout.contains("listen") || stdout.contains("poll"),
        "run --help should mention runtime/listen/poll"
    );
}

#[test]
fn cli_config_help() {
    let output = Command::new(env!("CARGO_BIN_EXE_can"))
        .args(["config", "--help"])
        .output()
        .expect("failed to run can config --help");

    assert!(output.status.success());
}

#[test]
fn cli_info_without_keystore_shows_friendly_message() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_can"))
        .arg("--data-dir")
        .arg(dir.path())
        .arg("info")
        .output()
        .expect("failed to run can info");

    // Should not panic — should show a friendly message
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !combined.contains("panicked"),
        "should not panic: {}",
        combined
    );
    // Should mention wallet/init/setup
    assert!(
        combined.contains("wallet") || combined.contains("init") || combined.contains("setup"),
        "should guide user to set up wallet: {}",
        combined
    );
}

#[test]
fn cli_inbox_without_keystore_shows_friendly_message() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_can"))
        .arg("--data-dir")
        .arg(dir.path())
        .arg("inbox")
        .output()
        .expect("failed to run can inbox");

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !combined.contains("panicked"),
        "should not panic: {}",
        combined
    );
}
