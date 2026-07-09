// packages/fetcher.rs --- Git-binary trampoline + content-addressed cache.

//! Git fetcher with content-addressed cache (T M7.2, spec
//! §sec:packages-future).
//!
//! Wraps the `git` binary in a deterministic-environment trampoline and
//! maintains a per-URL bare-mirror clone cache under
//! `$XDG_CACHE_HOME/pmacs/git/`. The cache is keyed by the normalized
//! URL hash (see [`normalize_url`]) so the same repository addressed
//! two ways (`https://github.com/foo/bar.git` vs
//! `https://github.com/foo/bar`) shares one cache entry.
//!
//! ## Why shell out to `git`
//!
//! Project posture is `forbid(unsafe_code)`; `git2` (libgit2 bindings)
//! is C-with-`unsafe` underneath. Shelling out matches the M6 PTY
//! trampoline pattern and inherits the user's git configuration
//! (credential helpers, signing, hooks, SSH agent) for free.
//! `coreutils`, `/bin/sh`, and `stty` are already documented as
//! runtime requirements; `git` joins that list for package operations.
//!
//! ## Environment hygiene
//!
//! All git invocations go through [`git_command`], which:
//!
//! - Removes inherited `GIT_*` environment variables (parent-set
//!   options that could subtly alter behavior). The deterministic env
//!   matters most for failure-mode reproducibility: if a developer's
//!   `GIT_DIR` leaks into a fetch, the resulting confusion is hard to
//!   debug.
//! - Sets `GIT_TERMINAL_PROMPT=0` so an unauthenticated repo fails
//!   fast instead of hanging on a credential prompt.
//! - Sets `GIT_CONFIG_NOSYSTEM=1` to ignore `/etc/gitconfig`. Corporate
//!   machines occasionally inject HTTP proxies or refspec rewrites
//!   here that would otherwise leak in.
//! - Sets `LC_ALL=C` so error-message parsing is locale-stable.
//!
//! `HOME`, `SSH_AUTH_SOCK`, and `PATH` are kept (so user credentials
//! work transparently).
//!
//! ## Concurrent access
//!
//! Two pmacs processes installing packages from the same upstream
//! must not collide. Each cache entry is gated by a `flock(2)` on a
//! sibling lock file (`<hash>.git.lock`); the lock covers the
//! clone-or-fetch operation. Same approach as M5.5's daemon socket
//! lockfile.
//!
//! ## Timeouts
//!
//! Each git invocation runs under a wall-clock deadline (default 60s,
//! configurable via [`Fetcher::with_timeout`]). On expiry the child
//! is `kill`'d and the call returns [`FetchError::Timeout`]. stdout
//! and stderr are drained from concurrent reader threads to avoid
//! pipe-buffer deadlock.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use thiserror::Error;

// ---------------------------------------------------------------------------
// RefSpec
// ---------------------------------------------------------------------------

/// A reference to resolve against a fetched repository.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum RefSpec {
    /// Tag name (resolved through `refs/tags/<name>` and unwrapped to
    /// the underlying commit via `^{commit}`).
    Tag(String),
    /// Commit-ish: full or short hash. Validated via `rev-parse`.
    Commit(String),
    /// Branch name (resolved through `refs/heads/<name>` to the
    /// branch's current HEAD at fetch time).
    Branch(String),
}

impl RefSpec {
    /// The argument passed to `git rev-parse` to resolve this spec to a
    /// commit hash. The `^{commit}` suffix unwraps annotated tags.
    fn rev_parse_arg(&self) -> String {
        match self {
            Self::Tag(t) => format!("refs/tags/{t}^{{commit}}"),
            Self::Commit(c) => format!("{c}^{{commit}}"),
            Self::Branch(b) => format!("refs/heads/{b}^{{commit}}"),
        }
    }

    /// Human-readable form for error messages.
    fn display(&self) -> String {
        match self {
            Self::Tag(t) => format!("tag `{t}`"),
            Self::Commit(c) => format!("commit `{c}`"),
            Self::Branch(b) => format!("branch `{b}`"),
        }
    }
}

// ---------------------------------------------------------------------------
// Fetcher
// ---------------------------------------------------------------------------

/// Fetch / cache / resolve operations against a content-addressed
/// directory.
///
/// One `Fetcher` per editor instance is sufficient; the on-disk cache
/// is the durable state. Multiple `Fetcher` instances pointing at the
/// same cache directory share its content (the cross-instance reuse
/// test in this module verifies that property).
#[derive(Debug, Clone)]
pub struct Fetcher {
    cache_dir: PathBuf,
    timeout: Duration,
}

const DEFAULT_TIMEOUT: Duration = Duration::from_mins(1);

impl Fetcher {
    /// Construct a fetcher with an explicit cache directory. The
    /// directory is created on demand; callers do not need to mkdir
    /// it themselves.
    #[must_use]
    pub fn with_cache_dir(cache_dir: PathBuf) -> Self {
        Self {
            cache_dir,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Construct a fetcher rooted at `$XDG_CACHE_HOME/pmacs/git/` (or
    /// `$HOME/.cache/pmacs/git/` if `XDG_CACHE_HOME` is unset).
    pub fn from_xdg() -> Result<Self, FetchError> {
        Ok(Self::with_cache_dir(xdg_cache_root()?))
    }

    /// Override the per-invocation wall-clock timeout. Builder-style.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Return the on-disk cache root. Useful for tests and logging.
    #[must_use]
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// Fetch (clone or update) `url` and return the path to the local
    /// bare-mirror clone.
    ///
    /// First call clones; subsequent calls run `git fetch --prune
    /// --tags`. The directory survives across `Fetcher` instances and
    /// processes (durable on-disk cache).
    pub fn fetch(&self, url: &str) -> Result<PathBuf, FetchError> {
        let normalized = normalize_url(url);
        let hash = sha256_hex(&normalized);
        let repo_path = self.cache_dir.join(format!("{hash}.git"));
        let lock_path = self.cache_dir.join(format!("{hash}.git.lock"));

        fs::create_dir_all(&self.cache_dir).map_err(|e| FetchError::CacheIo {
            path: self.cache_dir.clone(),
            source: e,
        })?;

        let _guard = LockGuard::acquire(&lock_path)?;

        if repo_path.join("HEAD").exists() {
            // Cache hit: refresh refs.
            self.run_git_in(
                &repo_path,
                &[
                    OsStr::new("fetch"),
                    OsStr::new("--prune"),
                    OsStr::new("--tags"),
                    OsStr::new("origin"),
                ],
                url,
            )?;
        } else {
            // Cache miss: mirror clone. The output directory must not
            // exist (git refuses to clone into a non-empty directory).
            if repo_path.exists() {
                fs::remove_dir_all(&repo_path).map_err(|e| FetchError::CacheIo {
                    path: repo_path.clone(),
                    source: e,
                })?;
            }
            self.run_git(
                &[
                    OsStr::new("clone"),
                    OsStr::new("--mirror"),
                    OsStr::new("--quiet"),
                    OsStr::new(url),
                    repo_path.as_os_str(),
                ],
                url,
            )?;
        }

        Ok(repo_path)
    }

    /// List the tag names present in a previously-fetched bare repo.
    ///
    /// Returns one entry per tag (short name; no `refs/tags/` prefix).
    /// Order matches `git`'s output, which is alphabetic for
    /// `for-each-ref` --- callers that need semver ordering must filter
    /// and sort themselves.
    pub fn list_tags(&self, repo_path: &Path) -> Result<Vec<String>, FetchError> {
        let stdout = self.run_git_in_capturing(
            repo_path,
            &[
                OsStr::new("for-each-ref"),
                OsStr::new("--format=%(refname:short)"),
                OsStr::new("refs/tags"),
            ],
            "",
        )?;
        let s = String::from_utf8_lossy(&stdout);
        Ok(s.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect())
    }

    /// Stream a commit's tree as a tar archive into `out`. The archive
    /// is rooted at the commit's top-level (i.e., entries have no
    /// extra path prefix); pair with a tar extractor whose CWD is the
    /// install destination.
    ///
    /// Used by the installer to materialize a commit without a
    /// `.git`-bearing worktree --- the install dir is self-contained,
    /// so wiping the bare cache later does not break installed packages.
    pub fn archive_commit(&self, repo_path: &Path, commit: &str) -> Result<Vec<u8>, FetchError> {
        self.run_git_in_capturing(
            repo_path,
            &[
                OsStr::new("archive"),
                OsStr::new("--format=tar"),
                OsStr::new(commit),
            ],
            "",
        )
    }

    /// Read a single file's contents at a specific commit, without
    /// materializing the rest of the tree. Used by the installer to
    /// pull `pmacs.toml` for the manifest before deciding the install
    /// directory's name.
    pub fn show_blob(
        &self,
        repo_path: &Path,
        commit: &str,
        path: &str,
    ) -> Result<Vec<u8>, FetchError> {
        let spec = format!("{commit}:{path}");
        self.run_git_in_capturing(repo_path, &[OsStr::new("show"), OsStr::new(&spec)], "")
    }

    /// Resolve a [`RefSpec`] against a previously-fetched repository,
    /// returning the full 40-char commit hash.
    pub fn resolve(&self, repo_path: &Path, refspec: &RefSpec) -> Result<String, FetchError> {
        let arg = refspec.rev_parse_arg();
        let output = self.run_git_in_capturing(
            repo_path,
            &[
                OsStr::new("rev-parse"),
                OsStr::new("--verify"),
                OsStr::new(&arg),
            ],
            // Empty url tag --- this call is local-only.
            "",
        );
        match output {
            Ok(stdout) => {
                let hash = String::from_utf8_lossy(&stdout).trim().to_string();
                if hash.len() != 40 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(FetchError::RefNotFound {
                        repo: repo_path.to_path_buf(),
                        refspec: refspec.display(),
                        cause: format!("rev-parse returned `{hash}` (expected 40-char SHA)"),
                    });
                }
                Ok(hash)
            }
            Err(FetchError::GitInvocation { stderr, .. }) => Err(FetchError::RefNotFound {
                repo: repo_path.to_path_buf(),
                refspec: refspec.display(),
                cause: stderr,
            }),
            Err(e) => Err(e),
        }
    }

    // -- internal git invocation helpers -----------------------------------

    fn run_git(&self, args: &[&OsStr], url_tag: &str) -> Result<(), FetchError> {
        self.run_git_inner(None, args, url_tag).map(|_| ())
    }

    fn run_git_in(&self, cwd: &Path, args: &[&OsStr], url_tag: &str) -> Result<(), FetchError> {
        self.run_git_inner(Some(cwd), args, url_tag).map(|_| ())
    }

    fn run_git_in_capturing(
        &self,
        cwd: &Path,
        args: &[&OsStr],
        url_tag: &str,
    ) -> Result<Vec<u8>, FetchError> {
        self.run_git_inner(Some(cwd), args, url_tag)
    }

    fn run_git_inner(
        &self,
        cwd: Option<&Path>,
        args: &[&OsStr],
        url_tag: &str,
    ) -> Result<Vec<u8>, FetchError> {
        let mut cmd = git_command();
        if let Some(d) = cwd {
            cmd.current_dir(d);
        }
        for a in args {
            cmd.arg(a);
        }

        let CapturedOutput {
            status,
            stdout,
            stderr,
        } = run_with_timeout(cmd, self.timeout, url_tag)?;
        if status.success() {
            Ok(stdout)
        } else {
            let stderr = String::from_utf8_lossy(&stderr).into_owned();
            Err(FetchError::GitInvocation {
                url: url_tag.to_string(),
                stderr,
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors raised by [`Fetcher`] operations. Every variant names either
/// the offending URL or the offending repository path so the caller's
/// error log identifies the work that failed.
#[derive(Debug, Error)]
pub enum FetchError {
    /// `$HOME` and `$XDG_CACHE_HOME` were both unset.
    #[error("cannot resolve XDG cache directory: HOME and XDG_CACHE_HOME are both unset")]
    NoCacheHome,
    /// I/O error creating, reading, or removing a cache directory.
    #[error("cache I/O error at `{path}`: {source}")]
    CacheIo {
        /// Path that the operation was acting on.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// `git` was not found on `PATH`.
    #[error(
        "the `git` binary was not found on PATH. \
         Pmacs's package operations require git as a runtime dependency."
    )]
    GitNotFound,
    /// `git` exited with a non-zero status. `stderr` is captured and
    /// surfaced verbatim to the caller.
    #[error("git invocation failed for `{url}`: {stderr}")]
    GitInvocation {
        /// The URL the operation was acting on (empty if N/A).
        url: String,
        /// Verbatim stderr from the failed git invocation.
        stderr: String,
    },
    /// Spawning `git` itself failed for a reason other than `NotFound`
    /// (e.g., permissions, ulimit).
    #[error("could not spawn git for `{url}`: {source}")]
    GitSpawn {
        /// The URL the operation was acting on.
        url: String,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// Wall-clock timeout exceeded during a git invocation.
    #[error("git operation for `{url}` timed out after {after:?}")]
    Timeout {
        /// The URL the operation was acting on.
        url: String,
        /// The timeout that was exceeded.
        after: Duration,
    },
    /// A ref was requested but could not be resolved in the cache.
    /// The most common cause: upstream removed the tag/branch since
    /// the last fetch.
    #[error("ref {refspec} not found in `{repo}`: {cause}")]
    RefNotFound {
        /// Path of the repo that was searched.
        repo: PathBuf,
        /// The refspec that failed to resolve, formatted for humans.
        refspec: String,
        /// The underlying cause from `git rev-parse`.
        cause: String,
    },
    /// `flock(2)` on the cache lockfile failed.
    #[error("could not acquire cache lock at `{path}`: {cause}")]
    LockFailed {
        /// Path of the lockfile.
        path: PathBuf,
        /// Underlying error description.
        cause: String,
    },
}

// ---------------------------------------------------------------------------
// XDG cache directory
// ---------------------------------------------------------------------------

fn xdg_cache_root() -> Result<PathBuf, FetchError> {
    if let Some(dir) = std::env::var_os("XDG_CACHE_HOME") {
        let p: PathBuf = dir.into();
        if !p.as_os_str().is_empty() {
            return Ok(p.join("pmacs").join("git"));
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let p: PathBuf = home.into();
        if !p.as_os_str().is_empty() {
            return Ok(p.join(".cache").join("pmacs").join("git"));
        }
    }
    Err(FetchError::NoCacheHome)
}

// ---------------------------------------------------------------------------
// URL normalization & hashing
// ---------------------------------------------------------------------------

/// Normalize a URL for cache-key derivation.
///
/// Rules:
/// 1. Strip trailing `/` characters.
/// 2. Strip trailing `.git`, but only for URL forms where
///    `<repo>.git` and `<repo>` are conventionally the same upstream:
///    `https://...`, `git://...`, and SSH shorthand
///    (`user@host:path`). For `file://` URLs, `ssh://` URLs, and
///    bare local paths the `.git` is left in place: `/tmp/foo.git`
///    and `/tmp/foo` are typically distinct trees on disk (e.g., a
///    bare mirror next to a working clone), and conflating their
///    cache keys would mix their refs.
/// 3. Lowercase the host portion (between `://` and the next `/` or
///    `:`). Path stays case-sensitive.
///
/// SSH shorthand has no `://` anchor; we still strip its trailing
/// `.git` (the equivalence is real for hosting forges), but we do
/// not lowercase its host portion (less risk of a trailing-token
/// false match in unusual forms).
#[must_use]
pub fn normalize_url(url: &str) -> String {
    let mut u = url.trim_end_matches('/').to_string();
    if dot_git_strip_applies(&u)
        && let Some(s) = u.strip_suffix(".git")
    {
        u = s.to_string();
    }
    if let Some(scheme_end) = u.find("://") {
        let host_start = scheme_end + 3;
        let after_host = u[host_start..]
            .find(['/', ':'])
            .map_or(u.len(), |i| host_start + i);
        let host_lower = u[host_start..after_host].to_ascii_lowercase();
        u = format!("{}{host_lower}{}", &u[..host_start], &u[after_host..]);
    }
    u
}

/// True iff the conventional `<repo>.git` ≡ `<repo>` equivalence
/// applies to this URL form. See [`normalize_url`] for the full
/// rationale.
fn dot_git_strip_applies(u: &str) -> bool {
    if u.starts_with("https://") || u.starts_with("git://") {
        return true;
    }
    // SSH shorthand: `user@host:path`. Distinguished from URL forms
    // by the `@` appearing before any `:` and no `://` prefix.
    if !u.contains("://")
        && let Some(at) = u.find('@')
        && let Some(colon) = u.find(':')
        && at < colon
    {
        return true;
    }
    false
}

/// SHA-256 of the string as 64 lowercase hex characters (audit F-009).
/// The cache dir is keyed by a hash of the (attacker-adjacent) repo URL,
/// so a *cryptographic* digest is used: a non-cryptographic hash like the
/// former 64-bit FNV-1a is trivially collidable, and a deliberate
/// collision would make two URLs share one bare mirror + lock file.
///
/// The implementation now lives in [`crate::hash`] — shared with the
/// desktop session key and the autosave recovery key (Q#AS9).
use crate::hash::sha256_hex;

// ---------------------------------------------------------------------------
// LockGuard --- per-cache-entry flock(2)
// ---------------------------------------------------------------------------

/// Holds an exclusive flock on a per-cache-entry lock file. The lock is
/// released when the guard is dropped.
struct LockGuard {
    _flock: nix::fcntl::Flock<fs::File>,
}

impl LockGuard {
    fn acquire(path: &Path) -> Result<Self, FetchError> {
        let file = fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)
            .map_err(|e| FetchError::LockFailed {
                path: path.to_path_buf(),
                cause: format!("open: {e}"),
            })?;
        let flock = nix::fcntl::Flock::lock(file, nix::fcntl::FlockArg::LockExclusive).map_err(
            |(_, errno)| FetchError::LockFailed {
                path: path.to_path_buf(),
                cause: format!("flock: {errno}"),
            },
        )?;
        Ok(Self { _flock: flock })
    }
}

// ---------------------------------------------------------------------------
// git_command --- deterministic environment trampoline
// ---------------------------------------------------------------------------

fn git_command() -> Command {
    let mut cmd = Command::new("git");
    // Drop inherited GIT_* env vars that could subtly alter behavior.
    let inherited: Vec<OsString> = std::env::vars_os()
        .map(|(k, _)| k)
        .filter(|k| k.to_string_lossy().starts_with("GIT_"))
        .collect();
    for k in inherited {
        cmd.env_remove(k);
    }
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("GIT_CONFIG_NOSYSTEM", "1");
    cmd.env("LC_ALL", "C");
    cmd
}

// ---------------------------------------------------------------------------
// run_with_timeout --- spawn + wall-clock deadline + concurrent pipe drain
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct CapturedOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_with_timeout(
    mut cmd: Command,
    timeout: Duration,
    url_tag: &str,
) -> Result<CapturedOutput, FetchError> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = cmd.spawn().map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => FetchError::GitNotFound,
        _ => FetchError::GitSpawn {
            url: url_tag.to_string(),
            source: e,
        },
    })?;
    let mut child: Child = child;

    // Drain stdout / stderr concurrently to avoid pipe-buffer deadlock
    // on commands that produce more than 64KB of output.
    let stdout_handle = child.stdout.take().expect("piped stdout always present");
    let stderr_handle = child.stderr.take().expect("piped stderr always present");
    let stdout_thread = thread::spawn(move || drain_to_vec(stdout_handle));
    let stderr_thread = thread::spawn(move || drain_to_vec(stderr_handle));

    // Break out of the wait loop with the outcome instead of returning
    // early, so the child is *reaped on every path* (normal exit, timeout
    // kill, or a `try_wait` error) and the drain threads are joined at the
    // single point below. Returning straight from a timeout used to leave
    // the two reader threads detached until their pipe reads happened to
    // finish (audit F-010) — nondeterministic and hard to test.
    let started = Instant::now();
    let outcome: Result<ExitStatus, FetchError> = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Ok(s),
            Ok(None) => {
                if started.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(FetchError::Timeout {
                        url: url_tag.to_string(),
                        after: timeout,
                    });
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                // Reap before bailing so the pipes close and the joins
                // below can't block on a still-running child.
                let _ = child.kill();
                let _ = child.wait();
                break Err(FetchError::GitSpawn {
                    url: url_tag.to_string(),
                    source: e,
                });
            }
        }
    };

    // The child is reaped on every path above, so its stdout/stderr are at
    // EOF and these joins return promptly. Always join before propagating
    // — no detached reader survives a timeout.
    let stdout = stdout_thread.join().unwrap_or_default();
    let stderr = stderr_thread.join().unwrap_or_default();
    let status = outcome?;
    Ok(CapturedOutput {
        status,
        stdout,
        stderr,
    })
}

fn drain_to_vec<R: Read>(mut r: R) -> Vec<u8> {
    let mut buf = Vec::new();
    let _ = r.read_to_end(&mut buf);
    buf
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as StdCommand;
    use tempfile::TempDir;

    // -- URL normalization ---------------------------------------------------

    #[test]
    fn normalize_strips_trailing_dot_git() {
        assert_eq!(
            normalize_url("https://github.com/foo/bar.git"),
            "https://github.com/foo/bar"
        );
    }

    #[test]
    fn normalize_strips_trailing_slash() {
        assert_eq!(
            normalize_url("https://example.com/foo/"),
            "https://example.com/foo"
        );
    }

    #[test]
    fn normalize_lowercases_host_only() {
        assert_eq!(
            normalize_url("https://GitHub.com/Foo/Bar"),
            "https://github.com/Foo/Bar"
        );
    }

    #[test]
    fn normalize_collapses_dual_address_to_one_key() {
        let a = sha256_hex(&normalize_url("https://github.com/foo/bar.git"));
        let b = sha256_hex(&normalize_url("https://github.com/foo/bar"));
        let c = sha256_hex(&normalize_url("https://GitHub.com/foo/bar/"));
        assert_eq!(a, b);
        assert_eq!(b, c);
        // SHA-256 hex: 64 lowercase hex chars, and distinct URLs differ.
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, sha256_hex(&normalize_url("https://github.com/foo/baz")));
    }

    #[test]
    fn normalize_passes_ssh_shorthand_through() {
        // SSH shorthand has no `://` anchor; we leave it alone (and
        // accept that two cache entries result if a user mixes forms).
        let u = "git@github.com:foo/bar.git";
        let n = normalize_url(u);
        // Trailing .git stripped, but host case unchanged.
        assert_eq!(n, "git@github.com:foo/bar");
    }

    #[test]
    fn normalize_keeps_dot_git_for_file_scheme() {
        // `file:///tmp/foo.git` and `file:///tmp/foo` are typically
        // distinct on-disk trees (e.g., a bare mirror next to a
        // working clone). Conflating their cache keys would mix
        // their refs. The `.git` strip applies only to
        // `https://`/`git://`/SSH shorthand --- the URL forms where
        // the equivalence is a real upstream convention.
        let bare = normalize_url("file:///tmp/foo.git");
        let work = normalize_url("file:///tmp/foo");
        assert_ne!(bare, work);
        assert_ne!(sha256_hex(&bare), sha256_hex(&work));
    }

    #[test]
    fn normalize_keeps_dot_git_for_ssh_url_scheme() {
        // `ssh://host/path/foo.git` is *not* the GitHub shorthand
        // form; for self-hosted SSH-over-git there is no convention
        // that `.git` and the bare name are the same upstream. Keep
        // them distinct.
        let with_git = normalize_url("ssh://git@example.org/foo.git");
        let without = normalize_url("ssh://git@example.org/foo");
        assert_ne!(with_git, without);
    }

    #[test]
    fn normalize_keeps_dot_git_for_bare_path() {
        // A bare filesystem path likewise: `/tmp/foo.git` and
        // `/tmp/foo` are different directory entries.
        let bare = normalize_url("/tmp/foo.git");
        let work = normalize_url("/tmp/foo");
        assert_ne!(bare, work);
    }

    // -- Fetcher tests against local file:// repos --------------------------

    /// Build a bare repo with a configurable history. Returns its path.
    /// The repo has commits, a tag `v1.0.0`, and a branch `feature`.
    fn make_bare_repo() -> (TempDir, PathBuf) {
        let td = tempfile::tempdir().unwrap();
        let work = td.path().join("work");
        let bare = td.path().join("upstream.git");

        // Create the working repo and history.
        run_git_test(&[OsStr::new("init"), work.as_os_str()], None);
        run_git_test(
            &[
                OsStr::new("-C"),
                work.as_os_str(),
                OsStr::new("config"),
                OsStr::new("user.email"),
                OsStr::new("test@example.com"),
            ],
            None,
        );
        run_git_test(
            &[
                OsStr::new("-C"),
                work.as_os_str(),
                OsStr::new("config"),
                OsStr::new("user.name"),
                OsStr::new("Tester"),
            ],
            None,
        );
        // Avoid relying on whatever default branch the local git uses.
        run_git_test(
            &[
                OsStr::new("-C"),
                work.as_os_str(),
                OsStr::new("checkout"),
                OsStr::new("-b"),
                OsStr::new("main"),
            ],
            None,
        );
        std::fs::write(work.join("README"), "first\n").unwrap();
        run_git_test(
            &[
                OsStr::new("-C"),
                work.as_os_str(),
                OsStr::new("add"),
                OsStr::new("README"),
            ],
            None,
        );
        run_git_test(
            &[
                OsStr::new("-C"),
                work.as_os_str(),
                OsStr::new("commit"),
                OsStr::new("-m"),
                OsStr::new("first"),
            ],
            None,
        );
        run_git_test(
            &[
                OsStr::new("-C"),
                work.as_os_str(),
                OsStr::new("tag"),
                OsStr::new("v1.0.0"),
            ],
            None,
        );
        run_git_test(
            &[
                OsStr::new("-C"),
                work.as_os_str(),
                OsStr::new("checkout"),
                OsStr::new("-b"),
                OsStr::new("feature"),
            ],
            None,
        );
        std::fs::write(work.join("README"), "second\n").unwrap();
        run_git_test(
            &[
                OsStr::new("-C"),
                work.as_os_str(),
                OsStr::new("commit"),
                OsStr::new("-am"),
                OsStr::new("second"),
            ],
            None,
        );

        // Push into a bare upstream.
        run_git_test(
            &[
                OsStr::new("clone"),
                OsStr::new("--bare"),
                work.as_os_str(),
                bare.as_os_str(),
            ],
            None,
        );

        (td, bare)
    }

    fn run_git_test(args: &[&OsStr], cwd: Option<&Path>) {
        let mut cmd = StdCommand::new("git");
        if let Some(d) = cwd {
            cmd.current_dir(d);
        }
        for a in args {
            cmd.arg(a);
        }
        cmd.env("GIT_TERMINAL_PROMPT", "0");
        cmd.env("LC_ALL", "C");
        let out = cmd.output().unwrap_or_else(|e| panic!("git spawn: {e}"));
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn file_url(p: &Path) -> String {
        format!("file://{}", p.display())
    }

    // ---- Acceptance bullets -----------------------------------------------

    #[test]
    fn fetch_clones_into_cache() {
        let (_td, bare) = make_bare_repo();
        let cache = tempfile::tempdir().unwrap();
        let fetcher = Fetcher::with_cache_dir(cache.path().to_path_buf());

        let url = file_url(&bare);
        let repo = fetcher.fetch(&url).unwrap();
        assert!(repo.join("HEAD").exists(), "bare clone should have HEAD");
        // Cache hash is deterministic.
        let expected_hash = sha256_hex(&normalize_url(&url));
        assert_eq!(
            repo.file_name().unwrap(),
            format!("{expected_hash}.git").as_str()
        );
    }

    #[test]
    fn fetch_twice_does_not_reclone_via_sentinel() {
        let (_td, bare) = make_bare_repo();
        let cache = tempfile::tempdir().unwrap();
        let fetcher = Fetcher::with_cache_dir(cache.path().to_path_buf());

        let url = file_url(&bare);
        let repo1 = fetcher.fetch(&url).unwrap();

        // Drop a sentinel inside the cached bare clone. If the second
        // fetch reclones, it must blow this away to do so.
        let sentinel = repo1.join("PMACS_TEST_SENTINEL");
        std::fs::write(&sentinel, b"hello").unwrap();

        let repo2 = fetcher.fetch(&url).unwrap();
        assert_eq!(repo1, repo2, "same URL should map to same path");
        assert!(
            sentinel.exists(),
            "second fetch must reuse cache, not reclone (sentinel removed)"
        );
    }

    #[test]
    fn cache_survives_across_fetcher_instances() {
        let (_td, bare) = make_bare_repo();
        let cache = tempfile::tempdir().unwrap();
        let url = file_url(&bare);

        // First fetcher clones.
        let f1 = Fetcher::with_cache_dir(cache.path().to_path_buf());
        let repo1 = f1.fetch(&url).unwrap();
        let sentinel = repo1.join("PMACS_TEST_SENTINEL");
        std::fs::write(&sentinel, b"hello").unwrap();
        drop(f1);

        // Second, fresh fetcher pointing at the same cache dir.
        let f2 = Fetcher::with_cache_dir(cache.path().to_path_buf());
        let repo2 = f2.fetch(&url).unwrap();
        assert_eq!(repo1, repo2);
        assert!(
            sentinel.exists(),
            "cache should be durable across Fetcher instances"
        );
    }

    #[test]
    fn resolve_tag_returns_commit_hash() {
        let (_td, bare) = make_bare_repo();
        let cache = tempfile::tempdir().unwrap();
        let fetcher = Fetcher::with_cache_dir(cache.path().to_path_buf());
        let repo = fetcher.fetch(&file_url(&bare)).unwrap();

        let hash = fetcher
            .resolve(&repo, &RefSpec::Tag("v1.0.0".into()))
            .unwrap();
        assert_eq!(hash.len(), 40);
        assert!(hash.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn resolve_branch_returns_branch_head() {
        let (_td, bare) = make_bare_repo();
        let cache = tempfile::tempdir().unwrap();
        let fetcher = Fetcher::with_cache_dir(cache.path().to_path_buf());
        let repo = fetcher.fetch(&file_url(&bare)).unwrap();

        let main_hash = fetcher
            .resolve(&repo, &RefSpec::Branch("main".into()))
            .unwrap();
        let feature_hash = fetcher
            .resolve(&repo, &RefSpec::Branch("feature".into()))
            .unwrap();
        assert_ne!(
            main_hash, feature_hash,
            "different branches should resolve to different commits"
        );
    }

    #[test]
    fn resolve_commit_validates_existing_hash() {
        let (_td, bare) = make_bare_repo();
        let cache = tempfile::tempdir().unwrap();
        let fetcher = Fetcher::with_cache_dir(cache.path().to_path_buf());
        let repo = fetcher.fetch(&file_url(&bare)).unwrap();
        let main = fetcher
            .resolve(&repo, &RefSpec::Branch("main".into()))
            .unwrap();

        let resolved = fetcher
            .resolve(&repo, &RefSpec::Commit(main.clone()))
            .unwrap();
        assert_eq!(resolved, main);
    }

    #[test]
    fn resolve_missing_ref_errors_with_useful_message() {
        let (_td, bare) = make_bare_repo();
        let cache = tempfile::tempdir().unwrap();
        let fetcher = Fetcher::with_cache_dir(cache.path().to_path_buf());
        let repo = fetcher.fetch(&file_url(&bare)).unwrap();

        let err = fetcher
            .resolve(&repo, &RefSpec::Tag("nonexistent".into()))
            .unwrap_err();
        let msg = err.to_string();
        assert!(matches!(err, FetchError::RefNotFound { .. }));
        assert!(
            msg.contains("nonexistent"),
            "error should name the ref: {msg}"
        );
    }

    #[test]
    fn fetch_after_upstream_tag_removed_surfaces_at_resolve() {
        // Test the nasty case: user pinned 0.1.0 last week, upstream
        // rewrote history and removed the tag. Fetch succeeds (the
        // remote is reachable) but the subsequent resolve fails
        // cleanly --- the cache isn't corrupted, the error is clear.
        let (td, bare) = make_bare_repo();
        let cache = tempfile::tempdir().unwrap();
        let fetcher = Fetcher::with_cache_dir(cache.path().to_path_buf());
        let url = file_url(&bare);
        let repo = fetcher.fetch(&url).unwrap();
        // Confirm the tag is initially resolvable.
        fetcher
            .resolve(&repo, &RefSpec::Tag("v1.0.0".into()))
            .unwrap();

        // Delete the tag from the bare upstream.
        run_git_test(
            &[
                OsStr::new("-C"),
                bare.as_os_str(),
                OsStr::new("tag"),
                OsStr::new("-d"),
                OsStr::new("v1.0.0"),
            ],
            None,
        );

        // Re-fetch (prunes the tag locally).
        fetcher.fetch(&url).unwrap();

        // Now resolution fails with a useful message.
        let err = fetcher
            .resolve(&repo, &RefSpec::Tag("v1.0.0".into()))
            .unwrap_err();
        assert!(matches!(err, FetchError::RefNotFound { .. }));
        let _ = td;
    }

    #[test]
    fn fetch_with_invalid_url_errors_naming_address() {
        let cache = tempfile::tempdir().unwrap();
        let fetcher = Fetcher::with_cache_dir(cache.path().to_path_buf());
        let bad_url = "file:///definitely/not/a/real/path/at/all.git";
        let err = fetcher.fetch(bad_url).unwrap_err();
        match &err {
            FetchError::GitInvocation { url, .. } => {
                assert_eq!(url, bad_url);
            }
            other => panic!("expected GitInvocation, got {other:?}"),
        }
    }

    #[test]
    fn timeout_kills_long_running_command() {
        // Exercise run_with_timeout directly against `sleep` so the
        // test is deterministic regardless of how fast git happens to
        // be on the host. `sleep` is in coreutils, already a documented
        // runtime test dependency. A 100ms timeout against a 5-second
        // sleep is comfortably outside flake territory.
        let mut cmd = Command::new("sleep");
        cmd.arg("5");
        let started = Instant::now();
        let err = run_with_timeout(cmd, Duration::from_millis(100), "test://timeout").unwrap_err();
        let elapsed = started.elapsed();
        assert!(
            matches!(err, FetchError::Timeout { .. }),
            "expected Timeout, got {err:?}"
        );
        // Sanity: we exited well before the 5s sleep would have ended.
        assert!(
            elapsed < Duration::from_secs(2),
            "timeout fired but took {elapsed:?} (> 2s)"
        );
    }
}
