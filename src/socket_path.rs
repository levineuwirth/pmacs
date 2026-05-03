// socket_path.rs --- Resolve and prepare the daemon socket path.

//! Socket-path resolution and directory preparation for the M5.5
//! local-attach transport (T M5.5c).
//!
//! # Path semantics
//!
//! - `--socket NAME` (no `/`) resolves to `<runtime>/pmacs/NAME.sock`.
//! - `--socket PATH` (any `/`) is used verbatim — relative paths pass
//!   through; resolution happens at `bind(2)` time, which is the
//!   user's choice.
//! - omitted argument is equivalent to `--socket default`.
//!
//! `<runtime>` is `$XDG_RUNTIME_DIR` when set, else
//! `/tmp/pmacs-<uid>`. The fallback is the standard XDG-spec fallback
//! for environments where the runtime dir isn't provisioned (macOS,
//! minimal containers, some CI configurations).
//!
//! # Permission posture
//!
//! Both the parent directory chain (`<runtime>/pmacs/` or
//! `/tmp/pmacs-<uid>/pmacs/`) and the eventual socket file must be
//! inaccessible to non-owner. [`ensure_runtime_subdir`] enforces this
//! on the directory side; the socket file itself gets mode 0600 via
//! `bind(2)` under `umask(0077)` in M5.5d.

use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Errors from socket-path resolution and directory preparation.
#[derive(Debug)]
pub enum SocketPathError {
    /// Could not create or access the runtime directory.
    Io(std::io::Error),
    /// An existing directory is too permissive — at least one
    /// group-or-other permission bit is set, which would expose the
    /// daemon socket beyond the owning user.
    DirectoryTooPermissive {
        /// The path that failed the check.
        path: PathBuf,
        /// Octal permission bits as returned by `stat(2)` (file-type
        /// bits masked off; e.g. `0o755`).
        mode: u32,
    },
}

impl std::fmt::Display for SocketPathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "socket-path I/O error: {e}"),
            Self::DirectoryTooPermissive { path, mode } => write!(
                f,
                "directory {} is too permissive (mode {mode:#o}); expected mode 0700 or stricter",
                path.display()
            ),
        }
    }
}

impl std::error::Error for SocketPathError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::DirectoryTooPermissive { .. } => None,
        }
    }
}

impl From<std::io::Error> for SocketPathError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Resolve a `--socket` argument to a socket path.
///
/// Reads `$XDG_RUNTIME_DIR` from the environment; falls back to
/// `/tmp/pmacs-<uid>` when unset or empty. The path is constructed —
/// directory creation is in [`ensure_runtime_subdir`].
#[must_use]
pub fn resolve_socket_path(arg: Option<&str>) -> PathBuf {
    resolve_socket_path_with_runtime(arg, &runtime_dir())
}

/// Pure path-construction logic.
///
/// Exposed so tests can vary the runtime dir without mutating the
/// process environment (which is `unsafe` under Rust 2024 and would
/// race with parallel test threads anyway).
#[must_use]
pub fn resolve_socket_path_with_runtime(arg: Option<&str>, runtime: &Path) -> PathBuf {
    match arg {
        None => runtime.join("pmacs").join("default.sock"),
        Some(s) if s.contains('/') => PathBuf::from(s),
        Some(name) => runtime.join("pmacs").join(format!("{name}.sock")),
    }
}

/// Effective runtime directory: `$XDG_RUNTIME_DIR` if set and
/// non-empty, else `/tmp/pmacs-<uid>`.
#[must_use]
pub fn runtime_dir() -> PathBuf {
    if let Some(xdg) = env::var_os("XDG_RUNTIME_DIR") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg);
        }
    }
    PathBuf::from(format!("/tmp/pmacs-{}", current_uid()))
}

/// Create the parent directory chain of `socket_path` if missing, with
/// mode 0700. If the parent already exists, verify that no
/// group-or-other permission bit is set; reject otherwise.
///
/// Called by the daemon at startup to prepare `<runtime>/pmacs/`
/// before `bind(2)`.
pub fn ensure_runtime_subdir(socket_path: &Path) -> Result<(), SocketPathError> {
    let Some(parent) = socket_path.parent() else {
        return Ok(());
    };
    // `Path::parent` returns `Some("")` for paths with no directory
    // component (e.g. `"nope.sock"`); treat that as "no parent."
    if parent.as_os_str().is_empty() {
        return Ok(());
    }
    if !parent.exists() {
        fs::create_dir_all(parent)?;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
        return Ok(());
    }
    let metadata = fs::metadata(parent)?;
    // Mask off file-type bits (S_IFMT, top 4 bits): only the
    // permission bits matter here.
    let mode = metadata.permissions().mode() & 0o7777;
    if (mode & 0o077) != 0 {
        return Err(SocketPathError::DirectoryTooPermissive {
            path: parent.to_path_buf(),
            mode,
        });
    }
    Ok(())
}

#[must_use]
fn current_uid() -> u32 {
    nix::unistd::getuid().as_raw()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn default_arg_resolves_to_default_sock_under_runtime_pmacs() {
        let runtime = Path::new("/run/user/1000");
        let p = resolve_socket_path_with_runtime(None, runtime);
        assert_eq!(p, Path::new("/run/user/1000/pmacs/default.sock"));
    }

    #[test]
    fn bare_name_resolves_to_named_sock_under_runtime_pmacs() {
        let runtime = Path::new("/run/user/1000");
        let p = resolve_socket_path_with_runtime(Some("research"), runtime);
        assert_eq!(p, Path::new("/run/user/1000/pmacs/research.sock"));
    }

    #[test]
    fn absolute_path_passes_through_unchanged() {
        let runtime = Path::new("/run/user/1000");
        let p = resolve_socket_path_with_runtime(Some("/tmp/foo.sock"), runtime);
        assert_eq!(p, Path::new("/tmp/foo.sock"));
    }

    #[test]
    fn relative_path_with_slash_passes_through_unchanged() {
        // The slash detection sends ./relative through verbatim. The
        // user took the leap of typing a slash; we trust them.
        let runtime = Path::new("/run/user/1000");
        let p = resolve_socket_path_with_runtime(Some("./relative.sock"), runtime);
        assert_eq!(p, Path::new("./relative.sock"));
    }

    #[test]
    fn creates_runtime_subdir_with_mode_0700_when_missing() {
        let tempdir = TempDir::new().unwrap();
        let socket_path = tempdir.path().join("pmacs").join("default.sock");

        ensure_runtime_subdir(&socket_path).expect("create");

        let parent = socket_path.parent().unwrap();
        assert!(parent.is_dir());
        let mode = fs::metadata(parent).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o700, "expected mode 0700, got {mode:#o}");
    }

    #[test]
    fn accepts_existing_runtime_subdir_with_mode_0700() {
        let tempdir = TempDir::new().unwrap();
        let parent = tempdir.path().join("pmacs");
        fs::create_dir(&parent).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();

        let socket_path = parent.join("default.sock");
        ensure_runtime_subdir(&socket_path).expect("accept");
    }

    #[test]
    fn accepts_existing_runtime_subdir_with_stricter_mode() {
        // 0500 (read+execute, no write) is stricter than 0700; still
        // owner-only, no group/other bits, accepted.
        let tempdir = TempDir::new().unwrap();
        let parent = tempdir.path().join("pmacs");
        fs::create_dir(&parent).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o500)).unwrap();

        let socket_path = parent.join("default.sock");
        ensure_runtime_subdir(&socket_path).expect("accept stricter");
    }

    #[test]
    fn rejects_existing_runtime_subdir_with_group_readable_mode() {
        let tempdir = TempDir::new().unwrap();
        let parent = tempdir.path().join("pmacs");
        fs::create_dir(&parent).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();

        let socket_path = parent.join("default.sock");
        match ensure_runtime_subdir(&socket_path) {
            Err(SocketPathError::DirectoryTooPermissive { mode, path }) => {
                assert_eq!(mode, 0o755);
                assert_eq!(path, parent);
            }
            other => panic!("expected DirectoryTooPermissive, got {other:?}"),
        }
    }

    #[test]
    fn rejects_existing_runtime_subdir_with_other_readable_mode() {
        let tempdir = TempDir::new().unwrap();
        let parent = tempdir.path().join("pmacs");
        fs::create_dir(&parent).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o704)).unwrap();

        let socket_path = parent.join("default.sock");
        match ensure_runtime_subdir(&socket_path) {
            Err(SocketPathError::DirectoryTooPermissive { mode, .. }) => {
                assert_eq!(mode, 0o704);
            }
            other => panic!("expected DirectoryTooPermissive, got {other:?}"),
        }
    }

    #[test]
    fn ensure_runtime_subdir_with_no_parent_is_ok() {
        // Edge case: a path with no parent (e.g. "default.sock" with
        // no directory component) should not panic. There's nothing
        // to create or check.
        let p = PathBuf::from("nope.sock");
        ensure_runtime_subdir(&p).expect("no parent → no-op");
    }
}
