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
    walk_for_marker(start, markers, None)
}

/// Like [`detect_project`], but halts the upward walk after examining
/// `stop_root`. Used by tests so a stray marker in a temp-dir's
/// ancestor (e.g. a developer's `/tmp/.git`) can't leak into a
/// fixture that lives below it. The walk still examines `stop_root`
/// itself; only its parent and beyond are skipped.
#[must_use]
pub fn detect_project_within(
    start: &Path,
    markers: &[ProjectMarker],
    stop_root: &Path,
) -> Option<(PathBuf, ProjectKind)> {
    walk_for_marker(start, markers, Some(stop_root))
}

fn walk_for_marker(
    start: &Path,
    markers: &[ProjectMarker],
    stop_root: Option<&Path>,
) -> Option<(PathBuf, ProjectKind)> {
    let start_dir: &Path = if start.is_file() {
        start.parent().unwrap_or(start)
    } else {
        start
    };
    for ancestor in start_dir.ancestors() {
        if let Some(kind) = match_marker(ancestor, markers) {
            return Some((ancestor.to_path_buf(), kind));
        }
        if let Some(stop) = stop_root
            && ancestor == stop
        {
            break;
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
    /// Optional clamp on [`Self::detect`]'s upward marker walk.
    /// When `None` (the default), detection walks ancestors all the
    /// way to the filesystem root (matching `git rev-parse
    /// --show-toplevel` semantics). When `Some(boundary)`, the walk
    /// halts after examining `boundary`; ancestors above `boundary`
    /// are not consulted.
    ///
    /// Stored canonicalized so the symlinked-workspace case behaves
    /// predictably (see [`Self::set_search_boundary`]).
    search_boundary: Option<PathBuf>,
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new()
    }
}

impl Workspace {
    /// Empty workspace with the [`default_markers`] rule set and no
    /// search boundary (detection walks to filesystem root).
    #[must_use]
    pub fn new() -> Self {
        Self {
            by_id: HashMap::new(),
            by_root: HashMap::new(),
            active: None,
            markers: default_markers(),
            search_boundary: None,
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

    /// Set (or clear) the upward-walk boundary for [`Self::detect`].
    ///
    /// `Some(path)` clamps detection so it never examines ancestors
    /// above `path`. `None` restores the default walk-to-filesystem-root
    /// behavior. Use case: a user whose code lives under `~/code` can
    /// set `search_boundary = "~/code"` in `init.lua` so a stray
    /// marker higher up the tree (e.g. `/tmp/.git`, an orphaned `.git`
    /// in `~`) cannot capture unrelated files.
    ///
    /// # Symlink handling
    ///
    /// The boundary is canonicalized at set time, and start paths are
    /// canonicalized at detect time, so a search starting from a
    /// symlinked path that resolves under the boundary still respects
    /// the boundary. If `path` does not exist on disk we store it
    /// as-is; later detection then compares against the literal value
    /// (which matches the upstream behavior of
    /// [`canonicalize_or_passthrough`]).
    ///
    /// # Inclusivity
    ///
    /// The boundary is *inclusive*: a marker located at the boundary
    /// path itself is found; only strict ancestors of the boundary
    /// are skipped. Set the boundary to the directory that *contains*
    /// your projects, not to one level above.
    pub fn set_search_boundary(&mut self, path: Option<PathBuf>) {
        self.search_boundary = path.map(|p| canonicalize_or_passthrough(&p));
    }

    /// Read-only view of the configured search boundary.
    #[must_use]
    pub fn search_boundary(&self) -> Option<&Path> {
        self.search_boundary.as_deref()
    }

    /// Detect the project root for `file_path`, honoring the
    /// workspace's [`Self::set_search_boundary`] clamp if any.
    ///
    /// `file_path` is canonicalized before the walk if possible so
    /// that boundary comparison works correctly under symlinks (e.g.,
    /// `/tmp/sandbox/foo` symlinked to `/home/user/code/foo`).
    #[must_use]
    pub fn detect(&self, file_path: &Path) -> Option<(PathBuf, ProjectKind)> {
        let canonical = canonicalize_or_passthrough(file_path);
        match self.search_boundary.as_deref() {
            Some(boundary) => detect_project_within(&canonical, &self.markers, boundary),
            None => detect_project(&canonical, &self.markers),
        }
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
        // Bound the walk at the tempdir so a marker in a real
        // ancestor (a developer's `/tmp/.git`, a CI runner's repo
        // root above the test fixture, etc.) can't masquerade as a
        // hit. Production callers walk to the filesystem root; the
        // bound is a test-only correctness aid.
        let dir = tempfile::tempdir().expect("tempdir");
        let f = dir.path().join("a/b.rs");
        touch(&f);
        assert!(detect_project_within(&f, &default_markers(), dir.path()).is_none());
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
        // No git → no detection. Bound the walk at the tempdir so
        // that a `.git` in any real ancestor cannot satisfy the
        // (now sole) git marker; we are testing that `set_markers`
        // chose the marker set, not what lives above the test.
        assert!(detect_project_within(&f, ws.markers(), root).is_none());
    }

    // ----------------------------------------------------------------------
    // Reviewer-flagged item 2: search-boundary clamp on detection.
    // ----------------------------------------------------------------------

    #[test]
    fn search_boundary_default_is_none() {
        let ws = Workspace::new();
        assert!(ws.search_boundary().is_none());
    }

    #[test]
    fn search_boundary_clamps_walk_above_boundary() {
        // Stage a fake "outer marker" above the boundary (the reviewer's
        // /tmp/.git case). Without the boundary, `detect` would walk up
        // to the outer marker. With it, the walk halts at the
        // boundary directory and returns None.
        let outer = tempfile::tempdir().expect("outer");
        // The outer dir gets a git marker.
        mkdir(&outer.path().join(".git"));
        // The "boundary" directory is a child of outer; the file
        // lives below the boundary.
        let boundary = outer.path().join("workspace");
        let f = boundary.join("src/main.rs");
        touch(&f);

        let mut ws = Workspace::new();
        // Without the boundary, detect walks up and finds the outer
        // marker.
        assert!(
            ws.detect(&f).is_some(),
            "sanity: outer .git is detectable without the boundary"
        );

        ws.set_search_boundary(Some(boundary.clone()));
        // With the boundary set to the workspace dir, the outer marker
        // is above the boundary and thus invisible.
        assert!(
            ws.detect(&f).is_none(),
            "with boundary at {boundary:?}, the outer marker must be excluded"
        );
    }

    #[test]
    fn search_boundary_is_inclusive_examines_boundary_itself() {
        // The boundary semantics: the boundary path *itself* is
        // examined for markers; only strict ancestors are skipped.
        // Documented inclusivity: "set boundary to the directory
        // containing your projects, not one level above."
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        touch(&root.join("Cargo.toml"));
        let f = root.join("src/lib.rs");
        touch(&f);

        let mut ws = Workspace::new();
        ws.set_search_boundary(Some(root.to_path_buf()));
        let detected = ws.detect(&f).expect("marker at the boundary must match");
        // Path equality after canonicalization (macOS /tmp → /private/tmp).
        assert_eq!(
            detected.0.canonicalize().expect("canon found"),
            root.canonicalize().expect("canon root")
        );
    }

    #[test]
    fn search_boundary_clearable_back_to_none() {
        let outer = tempfile::tempdir().expect("outer");
        mkdir(&outer.path().join(".git"));
        let boundary = outer.path().join("workspace");
        let f = boundary.join("src/main.rs");
        touch(&f);

        let mut ws = Workspace::new();
        ws.set_search_boundary(Some(boundary.clone()));
        assert!(ws.detect(&f).is_none());
        ws.set_search_boundary(None);
        assert!(
            ws.detect(&f).is_some(),
            "clearing the boundary must restore the unbounded walk"
        );
    }

    #[test]
    fn search_boundary_resolves_under_symlinked_start() {
        // The symlinked-workspace case: corporate /home mounts and
        // user-organized symlink farms put the file path under a
        // symlink that resolves into the boundary. The walk
        // canonicalizes both the start path and the boundary, so the
        // boundary applies after symlink resolution.
        //
        // Skip on platforms / sandboxes that disallow symlink
        // creation. Symlinks under tempfile dirs are normally allowed,
        // but a paranoid sandbox may reject EPERM.
        let real_dir = tempfile::tempdir().expect("real");
        let real = real_dir.path().to_path_buf();
        touch(&real.join("Cargo.toml"));
        let real_file = real.join("src/lib.rs");
        touch(&real_file);

        let link_dir = tempfile::tempdir().expect("link");
        let link = link_dir.path().join("via-link");
        if let Err(e) = std::os::unix::fs::symlink(&real, &link) {
            eprintln!("test skipped: symlink {link:?} → {real:?} failed: {e}");
            return;
        }

        let linked_file = link.join("src/lib.rs");

        let mut ws = Workspace::new();
        ws.set_search_boundary(Some(real.clone()));
        // Walking via the symlinked path: after canonicalization both
        // the start and the boundary live under `real`. The marker at
        // `real/Cargo.toml` is at the boundary and is examined.
        let (found, kind) = ws
            .detect(&linked_file)
            .expect("symlinked walk must still find the marker at the boundary");
        assert_eq!(kind, ProjectKind::Rust);
        assert_eq!(
            found.canonicalize().expect("canon"),
            real.canonicalize().expect("canon real")
        );
    }
}
