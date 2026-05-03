// project.rs --- T M4.9 project model: roots, workspace, scoping.

//! Project root detection, workspace state, and project-scoped LSP
//! reuse.
//!
//! Per spec §M4.9: "opening a file in a known project type identifies
//! the root", "project switch is a first-class operation", "LSP
//! servers run per-project, not per-buffer".
//!
//! # Detection
//!
//! [`detect_project`] walks upward from a file path looking for a
//! known marker. Markers are ordered: a deeper match (closer to the
//! file) wins, but among siblings the order in [`PROJECT_MARKERS`]
//! ranks language-specific markers above generic VCS roots so that
//! a `Cargo.toml` next to a `.git` is treated as a Rust project, not
//! a generic git repo. Walks stop at filesystem roots.
//!
//! # Workspace
//!
//! [`Workspace`] is the editor-wide registry: a map of canonical
//! project roots to [`Project`] values, plus an `active` pointer.
//! Opening the same root twice returns the existing id (idempotent).
//! Closing drops the project and any per-project LSP scoping built
//! on top of it.
//!
//! # LSP scoping
//!
//! [`crate::lsp::LspManager`] holds a `(project_root, language_id) →
//! LspServerId` table; `ensure_server_for_project` returns the
//! existing server if one is already serving that pair *and* is in a
//! healthy state, else spawns one. This is the mechanism by which
//! "LSP runs per-project, not per-buffer".

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

/// Stable identifier for one open project. Allocated in monotonic order.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct ProjectId(u64);

impl ProjectId {
    /// Mint a fresh id.
    #[must_use]
    pub fn next() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// Raw counter value. Used at the Lua boundary.
    #[must_use]
    pub fn raw(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for ProjectId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ProjectId({})", self.0)
    }
}

// ---------------------------------------------------------------------------
// Project kinds + markers
// ---------------------------------------------------------------------------

/// What kind of project a marker file identifies. Drives default LSP
/// language ids and labelling.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum ProjectKind {
    /// A Cargo workspace (`Cargo.toml`).
    Rust,
    /// A Lua project (`.luarc.json`).
    Lua,
    /// A Node.js / JavaScript project (`package.json`).
    Node,
    /// A Python project (`pyproject.toml`).
    Python,
    /// A Go module (`go.mod`).
    Go,
    /// A Deno project (`deno.json` / `deno.jsonc`).
    Deno,
    /// A bare git repository (no language marker found inside).
    Git,
    /// Detected via a custom marker registered at runtime.
    Custom(String),
}

impl ProjectKind {
    /// Stable lower-case tag (`"rust"`, `"lua"`, …) used by the Lua
    /// surface and the `*lsp*` buffer.
    #[must_use]
    pub fn tag(&self) -> &str {
        match self {
            Self::Rust => "rust",
            Self::Lua => "lua",
            Self::Node => "node",
            Self::Python => "python",
            Self::Go => "go",
            Self::Deno => "deno",
            Self::Git => "git",
            Self::Custom(s) => s.as_str(),
        }
    }

    /// Default LSP `languageId` for this project. Buffers in this
    /// kind of project use this string in `didOpen`/`didChange` so
    /// that one LSP per `(root, language_id)` is enough.
    #[must_use]
    pub fn default_language_id(&self) -> &str {
        match self {
            Self::Rust => "rust",
            Self::Lua => "lua",
            Self::Node => "javascript",
            Self::Python => "python",
            Self::Go => "go",
            Self::Deno => "typescript",
            Self::Git | Self::Custom(_) => "plaintext",
        }
    }
}

/// One marker rule: a file or directory name and the kind it
/// identifies.
#[derive(Clone, Debug)]
pub struct ProjectMarker {
    /// File or directory name to look for at each ancestor level.
    pub name: &'static str,
    /// Kind to report when this marker matches.
    pub kind: ProjectKind,
    /// `true` if the marker should be a directory (e.g. `.git`).
    /// `false` matches files (e.g. `Cargo.toml`).
    pub is_directory: bool,
}

/// Built-in marker rules, ranked so language-specific markers beat
/// the generic `.git` when both exist at the same ancestor level.
#[must_use]
pub fn default_markers() -> Vec<ProjectMarker> {
    vec![
        ProjectMarker {
            name: "Cargo.toml",
            kind: ProjectKind::Rust,
            is_directory: false,
        },
        ProjectMarker {
            name: ".luarc.json",
            kind: ProjectKind::Lua,
            is_directory: false,
        },
        ProjectMarker {
            name: "pyproject.toml",
            kind: ProjectKind::Python,
            is_directory: false,
        },
        ProjectMarker {
            name: "go.mod",
            kind: ProjectKind::Go,
            is_directory: false,
        },
        ProjectMarker {
            name: "deno.json",
            kind: ProjectKind::Deno,
            is_directory: false,
        },
        ProjectMarker {
            name: "deno.jsonc",
            kind: ProjectKind::Deno,
            is_directory: false,
        },
        ProjectMarker {
            name: "package.json",
            kind: ProjectKind::Node,
            is_directory: false,
        },
        // Generic VCS root: lowest priority among siblings.
        ProjectMarker {
            name: ".git",
            kind: ProjectKind::Git,
            is_directory: true,
        },
    ]
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// Walk upward from `start` looking for any marker in `markers`.
/// Returns the first ancestor where a marker matches, with the
/// marker's kind. Returns `None` if no marker is found before the
/// filesystem root.
///
/// `start` may be a file or a directory. If `start` is a file, the
/// search begins at its parent.
#[must_use]
pub fn detect_project(start: &Path, markers: &[ProjectMarker]) -> Option<(PathBuf, ProjectKind)> {
    let start_dir: &Path = if start.is_file() {
        start.parent().unwrap_or(start)
    } else {
        start
    };
    for ancestor in start_dir.ancestors() {
        if let Some(kind) = match_marker(ancestor, markers) {
            return Some((ancestor.to_path_buf(), kind));
        }
    }
    None
}

/// Return the highest-priority marker that matches this directory,
/// if any.
fn match_marker(dir: &Path, markers: &[ProjectMarker]) -> Option<ProjectKind> {
    for m in markers {
        let candidate = dir.join(m.name);
        let matches = if m.is_directory {
            candidate.is_dir()
        } else {
            candidate.is_file()
        };
        if matches {
            return Some(m.kind.clone());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Project + Workspace
// ---------------------------------------------------------------------------

/// One open project.
#[derive(Clone, Debug)]
pub struct Project {
    /// Stable identifier.
    pub id: ProjectId,
    /// Canonical absolute project root.
    pub root: PathBuf,
    /// Detected kind. May be `Custom` for projects opened with an
    /// explicit kind override.
    pub kind: ProjectKind,
    /// Display name. Defaults to the basename of `root`.
    pub name: String,
    /// When [`Workspace::open`] inserted this project.
    pub opened_at: Instant,
}

/// Editor-wide project registry. Owns one [`Project`] per unique
/// canonical root and tracks the active project (the one that
/// project-scoped commands target).
pub struct Workspace {
    by_id: HashMap<ProjectId, Project>,
    by_root: HashMap<PathBuf, ProjectId>,
    active: Option<ProjectId>,
    markers: Vec<ProjectMarker>,
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new()
    }
}

impl Workspace {
    /// Empty workspace with the [`default_markers`] rule set.
    #[must_use]
    pub fn new() -> Self {
        Self {
            by_id: HashMap::new(),
            by_root: HashMap::new(),
            active: None,
            markers: default_markers(),
        }
    }

    /// Replace the marker rule set. Useful for tests or for
    /// runtime-registered language packs (M5+).
    pub fn set_markers(&mut self, markers: Vec<ProjectMarker>) {
        self.markers = markers;
    }

    /// Read-only view of the configured markers.
    #[must_use]
    pub fn markers(&self) -> &[ProjectMarker] {
        &self.markers
    }

    /// Detect the project root for `file_path`. Convenience wrapper
    /// around [`detect_project`] using the workspace's marker rules.
    #[must_use]
    pub fn detect(&self, file_path: &Path) -> Option<(PathBuf, ProjectKind)> {
        detect_project(file_path, &self.markers)
    }

    /// Open a project at `root`. Idempotent: if a project for the
    /// same canonical root is already open, returns its id without
    /// allocating a new one. The first opened project becomes
    /// active by default.
    pub fn open(
        &mut self,
        root: impl Into<PathBuf>,
        kind: ProjectKind,
        name: Option<String>,
    ) -> ProjectId {
        let root = canonicalize_or_passthrough(&root.into());
        if let Some(id) = self.by_root.get(&root) {
            return *id;
        }
        let id = ProjectId::next();
        let project = Project {
            id,
            name: name.unwrap_or_else(|| {
                root.file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("project")
                    .to_owned()
            }),
            root: root.clone(),
            kind,
            opened_at: Instant::now(),
        };
        self.by_root.insert(root, id);
        self.by_id.insert(id, project);
        if self.active.is_none() {
            self.active = Some(id);
        }
        id
    }

    /// Open by detecting the kind from `root` itself. Returns `None`
    /// if no marker matches at `root` (callers can still
    /// [`Self::open`] explicitly with a kind override).
    pub fn open_detected(&mut self, root: impl Into<PathBuf>) -> Option<ProjectId> {
        let root = root.into();
        let kind = match_marker(&root, &self.markers)?;
        Some(self.open(root, kind, None))
    }

    /// Open the project containing `file_path`. Returns `None` if no
    /// marker is found between `file_path` and the filesystem root.
    pub fn open_for_file(&mut self, file_path: &Path) -> Option<ProjectId> {
        let (root, kind) = self.detect(file_path)?;
        Some(self.open(root, kind, None))
    }

    /// Close `id`. Returns `Ok(())` even if `id` was unknown
    /// (idempotent). If the closed project was active, the active
    /// pointer falls back to the most-recently-opened remaining
    /// project, or `None` if the workspace becomes empty.
    pub fn close(&mut self, id: ProjectId) {
        let Some(p) = self.by_id.remove(&id) else {
            return;
        };
        self.by_root.remove(&p.root);
        if self.active == Some(id) {
            self.active = self
                .by_id
                .values()
                .max_by_key(|p| p.opened_at)
                .map(|p| p.id);
        }
    }

    /// Set the active project. Returns `Err` if `id` is unknown.
    pub fn set_active(&mut self, id: ProjectId) -> Result<(), String> {
        if !self.by_id.contains_key(&id) {
            return Err(format!("unknown project: {id}"));
        }
        self.active = Some(id);
        Ok(())
    }

    /// Currently active project, if any.
    #[must_use]
    pub fn active(&self) -> Option<&Project> {
        self.active.and_then(|id| self.by_id.get(&id))
    }

    /// Active project id.
    #[must_use]
    pub fn active_id(&self) -> Option<ProjectId> {
        self.active
    }

    /// Look up a project by id.
    #[must_use]
    pub fn get(&self, id: ProjectId) -> Option<&Project> {
        self.by_id.get(&id)
    }

    /// Look up a project by canonical root.
    #[must_use]
    pub fn by_root(&self, root: &Path) -> Option<&Project> {
        let canon = canonicalize_or_passthrough(root);
        self.by_root.get(&canon).and_then(|id| self.by_id.get(id))
    }

    /// All projects, in id order.
    pub fn projects(&self) -> impl Iterator<Item = &Project> {
        let mut ids: Vec<ProjectId> = self.by_id.keys().copied().collect();
        ids.sort_by_key(|id| id.raw());
        ids.into_iter().filter_map(move |id| self.by_id.get(&id))
    }

    /// Total project count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// True iff no projects are open.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

/// Canonicalise `p` if possible; otherwise return it as-is. Falling
/// back on the passthrough lets tests use synthetic paths that don't
/// exist on disk.
fn canonicalize_or_passthrough(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(path, b"").expect("touch");
    }

    fn mkdir(path: &Path) {
        std::fs::create_dir_all(path).expect("mkdir");
    }

    #[test]
    fn project_kind_tags_are_stable() {
        for (k, tag) in [
            (ProjectKind::Rust, "rust"),
            (ProjectKind::Lua, "lua"),
            (ProjectKind::Node, "node"),
            (ProjectKind::Python, "python"),
            (ProjectKind::Go, "go"),
            (ProjectKind::Deno, "deno"),
            (ProjectKind::Git, "git"),
        ] {
            assert_eq!(k.tag(), tag);
        }
        assert_eq!(ProjectKind::Custom("zig".into()).tag(), "zig");
    }

    #[test]
    fn detect_finds_cargo_toml_above_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        touch(&root.join("Cargo.toml"));
        let nested = root.join("src/foo/bar.rs");
        touch(&nested);
        let (found, kind) = detect_project(&nested, &default_markers()).expect("detect");
        // Compare canonicalised paths so symlinked /tmp on macOS
        // still matches.
        assert_eq!(
            found.canonicalize().expect("canon"),
            root.canonicalize().expect("canon")
        );
        assert_eq!(kind, ProjectKind::Rust);
    }

    #[test]
    fn detect_prefers_language_marker_over_git_at_same_level() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        mkdir(&root.join(".git"));
        touch(&root.join("Cargo.toml"));
        let f = root.join("src/main.rs");
        touch(&f);
        let (_, kind) = detect_project(&f, &default_markers()).expect("detect");
        assert_eq!(kind, ProjectKind::Rust);
    }

    #[test]
    fn detect_walks_up_through_intermediates() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        touch(&root.join("Cargo.toml"));
        let deep = root.join("a/b/c/d/e.rs");
        touch(&deep);
        let (found, _) = detect_project(&deep, &default_markers()).expect("detect");
        assert_eq!(
            found.canonicalize().expect("canon"),
            root.canonicalize().expect("canon")
        );
    }

    #[test]
    fn detect_returns_none_with_no_markers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let f = dir.path().join("a/b.rs");
        touch(&f);
        assert!(detect_project(&f, &default_markers()).is_none());
    }

    #[test]
    fn detect_picks_innermost_root_when_nested() {
        let dir = tempfile::tempdir().expect("tempdir");
        let outer = dir.path();
        let inner = outer.join("crates/inner");
        touch(&outer.join("Cargo.toml"));
        touch(&inner.join("Cargo.toml"));
        let f = inner.join("src/lib.rs");
        touch(&f);
        let (found, kind) = detect_project(&f, &default_markers()).expect("detect");
        assert_eq!(
            found.canonicalize().expect("canon"),
            inner.canonicalize().expect("canon")
        );
        assert_eq!(kind, ProjectKind::Rust);
    }

    #[test]
    fn workspace_open_is_idempotent_per_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        touch(&root.join("Cargo.toml"));
        let mut ws = Workspace::new();
        let id1 = ws.open(root, ProjectKind::Rust, None);
        let id2 = ws.open(root, ProjectKind::Rust, None);
        assert_eq!(id1, id2);
        assert_eq!(ws.len(), 1);
    }

    #[test]
    fn workspace_first_opened_becomes_active() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut ws = Workspace::new();
        assert!(ws.active().is_none());
        let id = ws.open(dir.path(), ProjectKind::Git, None);
        assert_eq!(ws.active_id(), Some(id));
    }

    #[test]
    fn workspace_set_active_switches() {
        let d1 = tempfile::tempdir().expect("d1");
        let d2 = tempfile::tempdir().expect("d2");
        let mut ws = Workspace::new();
        let a = ws.open(d1.path(), ProjectKind::Git, None);
        let b = ws.open(d2.path(), ProjectKind::Git, None);
        assert_eq!(ws.active_id(), Some(a));
        ws.set_active(b).expect("switch");
        assert_eq!(ws.active_id(), Some(b));
    }

    #[test]
    fn workspace_close_drops_and_falls_back_active() {
        let d1 = tempfile::tempdir().expect("d1");
        let d2 = tempfile::tempdir().expect("d2");
        let mut ws = Workspace::new();
        let a = ws.open(d1.path(), ProjectKind::Git, None);
        let b = ws.open(d2.path(), ProjectKind::Git, None);
        ws.set_active(b).expect("switch");
        ws.close(b);
        assert_eq!(ws.active_id(), Some(a));
        ws.close(a);
        assert!(ws.active_id().is_none());
        assert!(ws.is_empty());
    }

    #[test]
    fn workspace_open_for_file_runs_detection() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        touch(&root.join("Cargo.toml"));
        let f = root.join("src/lib.rs");
        touch(&f);
        let mut ws = Workspace::new();
        let id = ws.open_for_file(&f).expect("open");
        assert_eq!(ws.get(id).unwrap().kind, ProjectKind::Rust);
    }

    #[test]
    fn workspace_set_active_unknown_errors() {
        let mut ws = Workspace::new();
        assert!(ws.set_active(ProjectId::next()).is_err());
    }

    #[test]
    fn workspace_set_markers_overrides_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        touch(&root.join("Cargo.toml"));
        let f = root.join("src/lib.rs");
        touch(&f);
        let mut ws = Workspace::new();
        // Replace markers with a set that ignores Cargo.
        ws.set_markers(vec![ProjectMarker {
            name: ".git",
            kind: ProjectKind::Git,
            is_directory: true,
        }]);
        // No git → no detection.
        assert!(ws.detect(&f).is_none());
    }
}
