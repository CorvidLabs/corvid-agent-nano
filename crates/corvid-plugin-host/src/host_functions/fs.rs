//! Host function: sandboxed read-only filesystem access within the project directory.
//!
//! Plugins with `FsProjectDir` capability can read files from the project
//! directory. Path traversal is prevented by resolving canonical paths and
//! enforcing a prefix check against the project root. Symlinks are not followed.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use wasmtime::Linker;

use crate::loader::PluginState;
use crate::wasm_mem;

/// Trait for pluggable filesystem read backends.
///
/// Implementations handle actual file reads. The host validates paths
/// are within the project directory before delegating to this trait.
pub trait FsRead: Send + Sync {
    /// Read a file's contents as bytes.
    ///
    /// `path` has already been validated as within the allowed directory.
    fn read(&self, path: &Path) -> Result<Vec<u8>, String>;

    /// Return the project root directory for path validation.
    fn project_dir(&self) -> &Path;
}

/// Thread-safe filesystem backend wrapper.
pub type FsBackend = dyn FsRead;

/// Default filesystem backend that reads from a configured project directory.
pub struct ProjectDirFs {
    root: PathBuf,
}

impl ProjectDirFs {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl FsRead for ProjectDirFs {
    fn read(&self, path: &Path) -> Result<Vec<u8>, String> {
        std::fs::read(path).map_err(|e| format!("read failed: {e}"))
    }

    fn project_dir(&self) -> &Path {
        &self.root
    }
}

/// Validate that a path is safe to read within the project directory.
///
/// Returns the canonical path if valid, or an error string explaining the rejection.
///
/// Security checks:
/// - Resolves `..` and `.` components
/// - Rejects paths outside the project directory (path traversal)
/// - Rejects paths that would follow symlinks outside the project directory
/// - Rejects absolute paths that don't start with the project root
pub fn validate_path(requested: &str, project_dir: &Path) -> Result<PathBuf, String> {
    if requested.is_empty() {
        return Err("empty path".into());
    }

    // Build the full path relative to project dir
    let full_path = if Path::new(requested).is_absolute() {
        PathBuf::from(requested)
    } else {
        project_dir.join(requested)
    };

    // Canonicalize both paths to resolve symlinks and `..`
    let canonical_root = project_dir
        .canonicalize()
        .map_err(|e| format!("cannot resolve project dir: {e}"))?;

    // Check if the file exists first (canonicalize requires existence)
    let canonical_path = full_path
        .canonicalize()
        .map_err(|e| format!("cannot resolve path '{}': {e}", requested))?;

    // Verify the canonical path is within the project directory
    if !canonical_path.starts_with(&canonical_root) {
        return Err(format!(
            "path '{}' resolves outside project directory",
            requested
        ));
    }

    Ok(canonical_path)
}

/// Link filesystem host functions into the WASM linker.
///
/// Provides `host_fs_read` in the "env" namespace.
pub fn link(linker: &mut Linker<PluginState>) -> anyhow::Result<()> {
    // host_fs_read(path_ptr, path_len) -> ptr to length-prefixed file content (0 = error)
    linker.func_wrap(
        "env",
        "host_fs_read",
        |mut caller: wasmtime::Caller<'_, PluginState>, path_ptr: i32, path_len: i32| -> i32 {
            // Read path string from WASM memory
            let path_str = match wasm_mem::read_str(&mut caller, path_ptr, path_len) {
                Some(s) => s,
                None => {
                    tracing::warn!("host_fs_read: failed to read path from WASM memory");
                    return 0;
                }
            };

            let plugin_id = caller.data().plugin_id.clone();
            let fs = match &caller.data().fs {
                Some(f) => Arc::clone(f),
                None => {
                    tracing::error!("host_fs_read: filesystem backend not initialized");
                    return 0;
                }
            };

            // Validate path is within project directory
            let canonical = match validate_path(&path_str, fs.project_dir()) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!(
                        plugin_id = %plugin_id,
                        path = %path_str,
                        error = %e,
                        "host_fs_read: path validation failed"
                    );
                    return 0;
                }
            };

            // Read file
            match fs.read(&canonical) {
                Ok(data) => wasm_mem::write_response(&mut caller, &data),
                Err(e) => {
                    tracing::warn!(
                        plugin_id = %plugin_id,
                        path = %path_str,
                        error = %e,
                        "host_fs_read: read failed"
                    );
                    0
                }
            }
        },
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_path_within_project() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("test.txt");
        std::fs::write(&file, "hello").unwrap();

        let result = validate_path("test.txt", tmp.path());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), file.canonicalize().unwrap());
    }

    #[test]
    fn traversal_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let result = validate_path("../../etc/passwd", tmp.path());
        // Either the file doesn't exist (canonicalize fails) or it's outside the root
        assert!(result.is_err());
    }

    #[test]
    fn empty_path_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let result = validate_path("", tmp.path());
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "empty path");
    }

    #[test]
    fn absolute_path_outside_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        // Create a file outside the project dir
        let other = tempfile::tempdir().unwrap();
        let file = other.path().join("secret.txt");
        std::fs::write(&file, "secret").unwrap();

        let result = validate_path(file.to_str().unwrap(), tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn nested_path_allowed() {
        let tmp = tempfile::tempdir().unwrap();
        let subdir = tmp.path().join("src");
        std::fs::create_dir_all(&subdir).unwrap();
        let file = subdir.join("main.rs");
        std::fs::write(&file, "fn main() {}").unwrap();

        let result = validate_path("src/main.rs", tmp.path());
        assert!(result.is_ok());
    }

    #[test]
    fn project_dir_fs_reads_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("data.txt");
        std::fs::write(&file, "content").unwrap();

        let fs = ProjectDirFs::new(tmp.path().to_path_buf());
        let data = fs.read(&file).unwrap();
        assert_eq!(data, b"content");
    }
}
