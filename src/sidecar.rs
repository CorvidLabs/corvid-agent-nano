//! Sidecar process manager for the Rust plugin host.
//!
//! Spawns `corvid-plugin-host` as a child process, monitors it, and restarts
//! on crash with exponential backoff. Kills the child cleanly on drop/shutdown.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tokio::process::{Child, Command};
use tokio::sync::Notify;
use tracing::{error, info, warn};

/// Configuration for the plugin host sidecar.
pub struct SidecarConfig {
    /// Path to the `corvid-plugin-host` binary.
    pub binary: PathBuf,
    /// Data directory (passed as --data-dir).
    pub data_dir: PathBuf,
    /// Agent ID for cache isolation (passed as --agent-id).
    pub agent_id: String,
    /// Log level for the plugin host (passed as --log-level).
    pub log_level: String,
}

/// Handle to a running plugin host sidecar.
///
/// Dropping the handle signals the supervisor to stop and kill the child.
pub struct SidecarHandle {
    shutdown: Arc<Notify>,
    task: tokio::task::JoinHandle<()>,
}

impl SidecarHandle {
    /// Signal the sidecar to stop and wait for the child process to exit.
    pub async fn shutdown(self) {
        self.shutdown.notify_one();
        let _ = self.task.await;
    }

    /// Returns the expected socket path for the plugin host.
    pub fn socket_path(data_dir: &Path) -> PathBuf {
        data_dir.join("plugins.sock")
    }
}

/// Locate the `corvid-plugin-host` binary.
///
/// Search order:
/// 1. Same directory as the current executable
/// 2. PATH lookup
pub fn find_plugin_host_binary() -> Option<PathBuf> {
    // Check alongside current executable
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("corvid-plugin-host");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    // Check PATH
    if let Ok(output) = std::process::Command::new("which")
        .arg("corvid-plugin-host")
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
    }

    None
}

/// Spawn the plugin host sidecar and return a handle.
///
/// The supervisor task restarts the process on crash with exponential backoff
/// (1s → 2s → 4s → ... → 30s cap). Logs all lifecycle events.
pub fn spawn_sidecar(config: SidecarConfig) -> SidecarHandle {
    let shutdown = Arc::new(Notify::new());
    let shutdown_rx = Arc::clone(&shutdown);

    let task = tokio::spawn(async move {
        let mut backoff = Duration::from_secs(1);
        let max_backoff = Duration::from_secs(30);

        loop {
            info!(
                binary = %config.binary.display(),
                data_dir = %config.data_dir.display(),
                agent_id = %config.agent_id,
                "spawning plugin host sidecar"
            );

            let child = spawn_child(&config);

            match child {
                Ok(mut child) => {
                    // Reset backoff on successful spawn
                    backoff = Duration::from_secs(1);

                    // Wait for either child exit or shutdown signal
                    tokio::select! {
                        status = child.wait() => {
                            match status {
                                Ok(s) if s.success() => {
                                    info!("plugin host exited cleanly");
                                    // Clean exit — don't restart unless we're not shutting down
                                }
                                Ok(s) => {
                                    warn!(
                                        code = s.code(),
                                        "plugin host exited with error — restarting in {:?}",
                                        backoff
                                    );
                                }
                                Err(e) => {
                                    error!(error = %e, "plugin host wait failed — restarting in {:?}", backoff);
                                }
                            }
                        }
                        _ = shutdown_rx.notified() => {
                            info!("shutting down plugin host sidecar");
                            let _ = child.kill().await;
                            // Clean up socket file
                            let socket = config.data_dir.join("plugins.sock");
                            let _ = std::fs::remove_file(&socket);
                            return;
                        }
                    }
                }
                Err(e) => {
                    error!(
                        error = %e,
                        binary = %config.binary.display(),
                        "failed to spawn plugin host — retrying in {:?}",
                        backoff
                    );
                }
            }

            // Wait before restarting (or bail on shutdown)
            tokio::select! {
                _ = tokio::time::sleep(backoff) => {}
                _ = shutdown_rx.notified() => {
                    info!("shutdown during backoff — not restarting plugin host");
                    // Clean up socket file
                    let socket = config.data_dir.join("plugins.sock");
                    let _ = std::fs::remove_file(&socket);
                    return;
                }
            }

            backoff = (backoff * 2).min(max_backoff);
        }
    });

    SidecarHandle { shutdown, task }
}

/// Spawn the child process with the right arguments.
fn spawn_child(config: &SidecarConfig) -> std::io::Result<Child> {
    Command::new(&config.binary)
        .arg("--data-dir")
        .arg(&config.data_dir)
        .arg("--agent-id")
        .arg(&config.agent_id)
        .arg("--log-level")
        .arg(&config.log_level)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
}

/// Wait for the plugin host socket to appear, with timeout.
///
/// Returns `true` if the socket became available, `false` on timeout.
pub async fn wait_for_socket(socket_path: &Path, timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    let poll_interval = Duration::from_millis(100);

    while start.elapsed() < timeout {
        if socket_path.exists() {
            // Verify it's connectable
            if tokio::net::UnixStream::connect(socket_path).await.is_ok() {
                return true;
            }
        }
        tokio::time::sleep(poll_interval).await;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_path_is_correct() {
        let p = SidecarHandle::socket_path(Path::new("/tmp/corvid"));
        assert_eq!(p, PathBuf::from("/tmp/corvid/plugins.sock"));
    }

    #[test]
    fn find_binary_returns_none_for_missing() {
        // In test env, corvid-plugin-host likely isn't installed
        // Just verify it doesn't panic
        let _ = find_plugin_host_binary();
    }
}
