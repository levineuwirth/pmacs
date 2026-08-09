// tests/lsp_latex_acceptance.rs --- LSP language coverage: LaTeX.

//! `docs/lsp-language-coverage-framing.md` §6, one test per bullet.
//!
//! The lane ships exactly one thing: `pmacs.lsp.config.latex`, command
//! `texlab`, with a function-valued `root` that walks up for texlab's
//! own project markers and stops at the document directory. Two pins
//! are load-bearing and the rest guard the boundary around them:
//!
//!   * the resolver returns the MARKER directory for a thesis whose
//!     chapters live in a subdirectory — the case a file-directory root
//!     gets wrong; and
//!   * `.git` NEVER becomes the root. This is the one entry where
//!     copying the other fourteen's instinct is actively wrong, and the
//!     failure mode is subtle: the resolver does not exclude `.git` by
//!     omitting it from its marker list, it excludes it by never
//!     declining, because `project_root_for` falls through to
//!     `pmacs.project.detect` on a nil and *that* walk includes `.git`.
//!     So the pin is end to end through attach, not just on the
//!     resolver's return.
//!
//! **Every fixture calls `pmacs.project.set_search_boundary` at its own
//! tempdir root.** R8 was a fixture letting detection escape into the
//! developer's environment, and a LaTeX root fixture is precisely that
//! hazard's shape: a stray `latexmkrc` or `.git` anywhere above the
//! temp directory would otherwise turn the markerless cases into marked
//! ones, and the assertions would still pass while testing nothing.
//!
//! **Attach fixtures point the command at `pmacs_fake_lsp`, and the
//! missing-server fixture at a path asserted not to exist.** The shipped
//! default is `texlab`, which is genuinely installed on the development
//! machine — a suite that relied on either its presence or its absence
//! would behave differently here and in CI.

use std::path::{Path, PathBuf};
use std::time::Duration;

use pmacs::editor::EditorState;

fn exec(state: &EditorState, source: &str) {
    state.lua_host.lua().load(source.to_owned()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(state: &EditorState, source: &str) -> T {
    state.lua_host.lua().load(source.to_owned()).eval().unwrap()
}

fn fake_lsp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_lsp").to_owned()
}

/// A fresh editor with the SHIPPED configs intact — this suite is about
/// the shipped `latex` entry, so it cannot clear the table the way the
/// multi-root suite does.
fn editor() -> EditorState {
    EditorState::new_with_roots(&crate::iso::roots())
}

fn lua_str(path: &Path) -> String {
    path.display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

/// Mirror of `file_uri_for` in `builtin/runtime/lsp.lua`. Reimplemented
/// rather than imported so the test states the expected encoding
/// independently of the code under test.
fn file_uri(path: &Path) -> String {
    let mut out = String::from("file://");
    for ch in path.display().to_string().chars() {
        match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '/' | '-' | '_' | '.' | '~' | ':' => out.push(ch),
            _ => {
                use std::fmt::Write as _;
                let mut buf = [0u8; 4];
                for byte in ch.encode_utf8(&mut buf).as_bytes() {
                    let _ = write!(out, "%{byte:02X}");
                }
            }
        }
    }
    out
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Fixture {
    /// Canonicalized, because the resolver canonicalizes before walking
    /// (`/var` is a symlink to `/private/var` on macOS) and the expected
    /// roots below have to compare equal to what it returns.
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        Self { _dir: dir, root }
    }

    fn write(&self, rel: &str, contents: &str) -> PathBuf {
        let path = self.root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn mkdir(&self, rel: &str) -> PathBuf {
        let path = self.root.join(rel);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn dir(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    fn bind(&self, state: &EditorState) {
        exec(
            state,
            &format!(
                "pmacs.project.set_search_boundary(\"{}\")",
                lua_str(&self.root)
            ),
        );
        // The boundary is the whole hermeticity story for this suite, so
        // assert it took rather than trusting the call.
        let seen: String = eval(state, "return pmacs.project.search_boundary() or \"\"");
        assert_eq!(
            seen,
            self.root.display().to_string(),
            "fixture precondition: the search boundary must be this tempdir"
        );
    }
}

/// Call the SHIPPED resolver directly.
fn resolve_root(state: &EditorState, file: &Path) -> Option<String> {
    let got: Option<String> = eval(
        state,
        &format!("return pmacs.lsp.config.latex.root(\"{}\")", lua_str(file)),
    );
    got
}

/// Repoint only the command, preserving the shipped `root` resolver —
/// which is the thing under test.
fn point_command_at(state: &EditorState, command: &str) {
    exec(
        state,
        &format!("pmacs.lsp.config.latex.command = {command:?}"),
    );
}

fn open(state: &EditorState, path: &Path) {
    exec(
        state,
        &format!("pmacs.buffer.find_or_open(\"{}\")", lua_str(path)),
    );
}

fn settle(state: &mut EditorState) {
    for _ in 0..8 {
        state.tick_processes();
        state.tick_lsp();
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// One `language_id|root_uri|cwd|state` row per live server.
fn rows(state: &EditorState) -> Vec<String> {
    let joined: String = eval(
        state,
        r#"
        local out = {}
        for _, s in ipairs(pmacs.lsp.list()) do
          out[#out + 1] = table.concat({
            s.language_id or "",
            s.root_uri or "",
            s.cwd or "",
            (s.state and s.state.kind) or "",
          }, "|")
        end
        table.sort(out)
        return table.concat(out, "\n")
        "#,
    );
    if joined.is_empty() {
        Vec::new()
    } else {
        joined.lines().map(str::to_owned).collect()
    }
}

fn status(state: &EditorState) -> String {
    state.core.borrow().status.clone()
}

const DOC: &str = "\\documentclass{article}\n\\begin{document}\nhi\n\\end{document}\n";

// ---------------------------------------------------------------------------
// §6 — the shipped entry. Command `texlab`, and NOTHING opinionated
// (Q#LX1: no `settings`, no `init_options`).
// ---------------------------------------------------------------------------

#[test]
fn latex_entry_ships_texlab_with_a_resolver_and_no_opinionated_config() {
    let state = editor();
    let command: String = eval(&state, "return pmacs.lsp.config.latex.command");
    assert_eq!(
        command, "texlab",
        "the shipped LaTeX server is texlab, invoked bare — the binary \
         serves LSP over stdio with no subcommand"
    );

    // Q#LX1. Build-on-save and forward-search are both opinionated and
    // forward-search needs a configured viewer, so an empty section
    // takes texlab's defaults through the `workspace/configuration`
    // answer pmacs already gives.
    let has_settings: bool = eval(&state, "return pmacs.lsp.config.latex.settings ~= nil");
    assert!(!has_settings, "Q#LX1: no `settings` may ship");
    let has_init: bool = eval(&state, "return pmacs.lsp.config.latex.init_options ~= nil");
    assert!(!has_init, "Q#LX1: no `init_options` may ship");

    let root_kind: String = eval(&state, "return type(pmacs.lsp.config.latex.root)");
    assert_eq!(
        root_kind, "function",
        "the root must be a resolver — the shared marker walk cannot \
         express a LaTeX root, because it would include .git"
    );
}

// ---------------------------------------------------------------------------
// §6 — detection is unchanged: `.tex`/`.latex`/`.sty`/`.cls` resolve to
// `latex` through the GRAMMAR path, ahead of the LSP filetype map.
//
// Pinned so that a later "helpful" filetype-map addition cannot be
// mistaken for the thing that made attach work. Revision 2 of the
// framing exists because revision 1 proposed exactly that addition.
// ---------------------------------------------------------------------------

#[test]
fn latex_extensions_resolve_through_the_grammar_not_the_lsp_filetype_map() {
    let state = editor();
    for ext in ["tex", "latex", "sty", "cls"] {
        let language: Option<String> = eval(
            &state,
            &format!("return pmacs.parse.language_for_path(\"/tmp/doc.{ext}\")"),
        );
        assert_eq!(
            language.as_deref(),
            Some("latex"),
            ".{ext} must resolve to `latex` via the bundled grammar"
        );

        // And the map is empty for it, so the assertion above cannot be
        // being satisfied by a filetype entry.
        let mapped: Option<String> =
            eval(&state, &format!("return pmacs.lsp.filetypes[\"{ext}\"]"));
        assert_eq!(
            mapped, None,
            "no `pmacs.lsp.filetypes.{ext}` ships: the grammar already \
             carries the extension and sits ahead of this map in \
             detect_buffer_language"
        );
    }
}

// ---------------------------------------------------------------------------
// §6 — LOAD-BEARING: the resolver returns the MARKER directory, on the
// thesis shape (marker above a `chapters/` subdirectory). This is
// exactly the case a file-directory root gets wrong.
// ---------------------------------------------------------------------------

#[test]
fn latex_root_is_the_marker_directory_for_a_thesis_with_chapters() {
    // texlab's own marker set, from `crates/distro/src/language.rs` at
    // v5.25.1: `.texlabroot`/`texlabroot` -> Root, `Tectonic.toml` ->
    // Tectonic, `.latexmkrc`/`latexmkrc` -> Latexmkrc.
    for marker in [
        ".texlabroot",
        "texlabroot",
        "Tectonic.toml",
        ".latexmkrc",
        "latexmkrc",
    ] {
        let fx = Fixture::new();
        let state = editor();
        fx.bind(&state);
        // Empty, because `.texlabroot` is normally written empty and
        // existence — not content — is the marker semantics.
        fx.write(&format!("thesis/{marker}"), "");
        fx.write("thesis/thesis.tex", DOC);
        let chapter = fx.write("thesis/chapters/one.tex", "\\section{One}\n");

        assert_eq!(
            resolve_root(&state, &chapter).as_deref(),
            Some(fx.dir("thesis").display().to_string().as_str()),
            "{marker}: the root must be the marker directory, not the \
             chapter's own directory"
        );
    }
}

#[test]
fn latex_root_takes_the_innermost_marker_when_markers_nest() {
    let fx = Fixture::new();
    let state = editor();
    fx.bind(&state);
    fx.write("outer/latexmkrc", "");
    fx.write("outer/inner/Tectonic.toml", "");
    let doc = fx.write("outer/inner/chapters/one.tex", "\\section{One}\n");

    assert_eq!(
        resolve_root(&state, &doc).as_deref(),
        Some(fx.dir("outer/inner").display().to_string().as_str()),
        "innermost ancestor wins, matching texlab's own \
         ProjectRoot::walk_and_find"
    );
}

#[test]
fn latex_root_ignores_a_marker_that_is_a_directory() {
    // `io.open` succeeds on a directory, so a bare truthiness test would
    // accept `latexmkrc/` as a marker. The read-error discriminator is
    // what rejects it; without this pin that subtlety is unguarded.
    let fx = Fixture::new();
    let state = editor();
    fx.bind(&state);
    fx.mkdir("proj/latexmkrc");
    let doc = fx.write("proj/chapters/one.tex", "\\section{One}\n");

    assert_eq!(
        resolve_root(&state, &doc).as_deref(),
        Some(fx.dir("proj/chapters").display().to_string().as_str()),
        "a DIRECTORY named latexmkrc is not a marker"
    );
}

// ---------------------------------------------------------------------------
// §6 — it falls back to the file's own directory with no marker present.
// ---------------------------------------------------------------------------

#[test]
fn latex_root_falls_back_to_the_files_own_directory() {
    let fx = Fixture::new();
    let state = editor();
    fx.bind(&state);
    let doc = fx.write("loose/note.tex", DOC);

    assert_eq!(
        resolve_root(&state, &doc).as_deref(),
        Some(fx.dir("loose").display().to_string().as_str()),
        "a markerless document roots at its own directory"
    );
}

// ---------------------------------------------------------------------------
// §6 — LOAD-BEARING: `.git` does NOT become the root.
//
// Both halves matter. The resolver must not return the repository root,
// AND it must not DECLINE — a nil falls through to
// `pmacs.project.detect`, whose marker walk does include `.git`, so a
// declining resolver would hand texlab the monorepo by the back door.
// The second assertion is therefore end to end through attach.
// ---------------------------------------------------------------------------

#[test]
fn latex_root_is_never_a_git_repository_root() {
    let fx = Fixture::new();
    let state = editor();
    fx.bind(&state);
    // A repository ABOVE a document directory — the thesis-inside-a-
    // monorepo shape.
    fx.mkdir("repo/.git");
    fx.write("repo/README.md", "monorepo\n");
    let doc = fx.write("repo/paper/paper.tex", DOC);

    assert_eq!(
        resolve_root(&state, &doc).as_deref(),
        Some(fx.dir("repo/paper").display().to_string().as_str()),
        "the document directory wins: texlab wants the DOCUMENT root, \
         and a thesis in a monorepo must not get the monorepo"
    );

    // The same fixture proves `pmacs.project.detect` really would have
    // answered the repository root, so the assertion above is not
    // vacuous.
    let detected: Option<String> = eval(
        &state,
        &format!(
            "local ok, d = pcall(pmacs.project.detect, \"{}\")\n\
             if ok and d then return d.root end\n\
             return nil",
            lua_str(&doc)
        ),
    );
    assert_eq!(
        detected.as_deref(),
        Some(fx.dir("repo").display().to_string().as_str()),
        "fixture precondition: the shared detector DOES answer the \
         repository root here — that is what the resolver must avoid"
    );
}

#[test]
fn a_tex_buffer_in_a_git_repo_attaches_at_the_document_directory() {
    let fx = Fixture::new();
    let mut state = editor();
    fx.bind(&state);
    point_command_at(&state, &fake_lsp_path());
    fx.mkdir("repo/.git");
    let doc = fx.write("repo/paper/paper.tex", DOC);
    open(&state, &doc);
    settle(&mut state);

    let rows = rows(&state);
    assert_eq!(rows.len(), 1, "one latex server: {rows:?}");
    let fields: Vec<&str> = rows[0].split('|').collect();
    assert_eq!(fields[0], "latex");
    assert_eq!(
        fields[1],
        file_uri(&fx.dir("repo/paper")),
        "root_uri must be the document directory, NOT the repository root"
    );
    assert_eq!(
        fields[2],
        fx.dir("repo/paper").display().to_string(),
        "cwd must be the document directory"
    );
}

// ---------------------------------------------------------------------------
// §6 — a `.tex` buffer attaches the LaTeX server, witnessed end to end
// rather than by asserting the config table's contents.
// ---------------------------------------------------------------------------

#[test]
fn a_tex_buffer_attaches_the_latex_server_at_the_marker_root() {
    let fx = Fixture::new();
    let mut state = editor();
    fx.bind(&state);
    point_command_at(&state, &fake_lsp_path());
    fx.write("thesis/latexmkrc", "");
    let chapter = fx.write("thesis/chapters/one.tex", "\\section{One}\n");
    open(&state, &chapter);
    settle(&mut state);

    let rows = rows(&state);
    assert_eq!(rows.len(), 1, "expected one latex server: {rows:?}");
    let fields: Vec<&str> = rows[0].split('|').collect();
    assert_eq!(
        fields[0], "latex",
        "the buffer must resolve to language `latex` and attach"
    );
    assert_eq!(
        fields[1],
        file_uri(&fx.dir("thesis")),
        "the attached server's root is the marker directory"
    );
}

#[test]
fn two_chapters_of_one_thesis_share_a_single_server() {
    // The marker walk's whole point: without it each chapter directory
    // would be its own root and texlab would serve isolated files.
    let fx = Fixture::new();
    let mut state = editor();
    fx.bind(&state);
    point_command_at(&state, &fake_lsp_path());
    fx.write("thesis/latexmkrc", "");
    let one = fx.write("thesis/chapters/one.tex", "\\section{One}\n");
    let two = fx.write("thesis/appendix/two.tex", "\\section{Two}\n");
    open(&state, &one);
    settle(&mut state);
    open(&state, &two);
    settle(&mut state);

    let rows = rows(&state);
    assert_eq!(
        rows.len(),
        1,
        "both chapters share the thesis root, so one server: {rows:?}"
    );
    assert_eq!(
        rows[0].split('|').nth(1).unwrap(),
        file_uri(&fx.dir("thesis"))
    );
}

#[test]
fn two_chapters_share_one_server_under_a_root_search_boundary() {
    // A `/` boundary is "clamp nothing", spelled as a path — and it used
    // to disable the marker walk OUTRIGHT. The containment test was
    // string arithmetic (`dir:sub(1, #boundary + 1) == boundary .. "/"`),
    // so a `/` boundary asked whether each ancestor began with `"//"`,
    // which no canonical path does. Every ancestor was judged out of
    // bounds, no marker was ever examined, and each chapter got its own
    // root — the lane's headline behaviour, silently off, with the
    // predicate's unit-level answers all still looking plausible.
    //
    // Pinned through ATTACH because that is where the symptom lives: two
    // texlab processes for one thesis, not a wrong string.
    //
    // Still hermetic despite the unclamped boundary: innermost marker
    // wins, and `thesis/` has one, so no `latexmkrc` above the tempdir
    // can change the answer.
    let fx = Fixture::new();
    let mut state = editor();
    exec(&state, "pmacs.project.set_search_boundary(\"/\")");
    let seen: String = eval(&state, "return pmacs.project.search_boundary() or \"\"");
    assert_eq!(
        seen, "/",
        "fixture precondition: the boundary must be the filesystem root"
    );
    point_command_at(&state, &fake_lsp_path());
    fx.write("thesis/latexmkrc", "");
    let one = fx.write("thesis/chapters/one.tex", "\\section{One}\n");
    let two = fx.write("thesis/appendix/two.tex", "\\section{Two}\n");
    open(&state, &one);
    settle(&mut state);
    open(&state, &two);
    settle(&mut state);

    let rows = rows(&state);
    assert_eq!(
        rows.len(),
        1,
        "a root boundary must behave like any other boundary: both \
         chapters resolve to the thesis root, so ONE server: {rows:?}"
    );
    assert_eq!(
        rows[0].split('|').nth(1).unwrap(),
        file_uri(&fx.dir("thesis")),
        "and that one server is rooted at the marker directory"
    );
}

#[test]
fn two_markerless_documents_in_different_directories_do_not_share_a_server() {
    // The complement of the pin above: the fallback is the file's own
    // directory, so unrelated loose documents keep separate scopes
    // rather than collapsing into one rootless server.
    let fx = Fixture::new();
    let mut state = editor();
    fx.bind(&state);
    point_command_at(&state, &fake_lsp_path());
    let one = fx.write("a/one.tex", DOC);
    let two = fx.write("b/two.tex", DOC);
    open(&state, &one);
    settle(&mut state);
    open(&state, &two);
    settle(&mut state);

    let rows = rows(&state);
    assert_eq!(rows.len(), 2, "one server per document directory: {rows:?}");
    let roots: Vec<&str> = rows.iter().map(|r| r.split('|').nth(1).unwrap()).collect();
    assert!(
        roots.contains(&file_uri(&fx.dir("a")).as_str()),
        "{roots:?}"
    );
    assert!(
        roots.contains(&file_uri(&fx.dir("b")).as_str()),
        "{roots:?}"
    );
}

// ---------------------------------------------------------------------------
// §6 — a missing `texlab` surfaces guidance through the existing
// spawn-failure path (#204). Asserted, not assumed.
// ---------------------------------------------------------------------------

#[test]
fn a_missing_texlab_surfaces_installation_guidance() {
    let fx = Fixture::new();
    let mut state = editor();
    fx.bind(&state);
    // A path that cannot exist, asserted — texlab IS installed on the
    // development machine, so relying on its absence would make this
    // vacuous here and meaningful only in CI.
    let absent = fx.dir("no-such-bin/texlab");
    assert!(
        !absent.exists(),
        "fixture precondition: {} must not exist",
        absent.display()
    );
    point_command_at(&state, &absent.display().to_string());
    let doc = fx.write("paper/paper.tex", DOC);
    open(&state, &doc);
    settle(&mut state);

    assert!(rows(&state).is_empty(), "nothing may have started");
    let status = status(&state);
    assert!(
        status.contains("did not start") && status.contains("latex"),
        "the spawn-failure path must name the language: {status:?}"
    );
    assert!(
        status.contains("pmacs.lsp.config.latex.command"),
        "the guidance must name the override seam: {status:?}"
    );

    // And the failure is recorded, not just flashed.
    let recorded: bool = eval(
        &state,
        "for _, f in ipairs(pmacs.lsp.spawn_failures()) do\n\
           if f.language == \"latex\" then return true end\n\
         end\n\
         return false",
    );
    assert!(recorded, "M-x lsp.status must carry the latex failure");
}

// ---------------------------------------------------------------------------
// The resolver declines only when there is no directory to vouch for.
// A decline is the one path that reaches `pmacs.project.detect`, so its
// preconditions are worth pinning.
// ---------------------------------------------------------------------------

#[test]
fn latex_root_declines_for_a_non_string_or_pathless_argument() {
    let state = editor();
    let nil_arg: Option<String> = eval(&state, "return pmacs.lsp.config.latex.root(nil)");
    assert_eq!(nil_arg, None, "a pathless buffer declines");
    let bare: Option<String> = eval(
        &state,
        "return pmacs.lsp.config.latex.root(\"noslash.tex\")",
    );
    assert_eq!(bare, None, "a name with no directory component declines");
}

#[test]
fn latex_root_walk_stops_at_the_search_boundary() {
    // R8's shape, pinned directly: a marker ABOVE the boundary must be
    // invisible, or every markerless assertion in this file is hostage
    // to the developer's filesystem.
    let fx = Fixture::new();
    let state = editor();
    // Marker at the tempdir root, boundary set BELOW it.
    fx.write("latexmkrc", "");
    let inner = fx.mkdir("inner");
    fx.write("inner/chapters/one.tex", "\\section{One}\n");
    exec(
        &state,
        &format!("pmacs.project.set_search_boundary(\"{}\")", lua_str(&inner)),
    );

    let doc = fx.dir("inner/chapters/one.tex");
    assert_eq!(
        resolve_root(&state, &doc).as_deref(),
        Some(fx.dir("inner/chapters").display().to_string().as_str()),
        "the walk must not climb past the search boundary to reach the \
         marker above it"
    );

    // The other direction, and it is not decoration: "stops at the
    // boundary" is also satisfied by a walk that never runs at all —
    // which is precisely what a `/` boundary used to produce. So assert
    // that within the boundary the walk still CLIMBS, and that the
    // boundary directory itself is a candidate (inclusive, matching
    // `set_search_boundary`'s documented contract).
    fx.write("inner/.texlabroot", "");
    assert_eq!(
        resolve_root(&state, &doc).as_deref(),
        Some(inner.display().to_string().as_str()),
        "a marker AT the boundary directory is found, and the walk \
         climbs out of `chapters/` to reach it"
    );
}

#[test]
fn latex_root_for_a_document_at_the_filesystem_root_is_the_root() {
    // The same root-is-special trap one level up: `/paper.tex` slices to
    // an EMPTY directory string, which canonicalizes to nothing, so the
    // resolver DECLINED — and a decline is the one path that falls
    // through to `pmacs.project.detect`, whose walk includes `.git`.
    // Hermetic: the boundary is this fixture's tempdir, so `/` is out of
    // bounds, no marker is examined, and the answer is the directory
    // itself regardless of what sits at the filesystem root.
    let fx = Fixture::new();
    let state = editor();
    fx.bind(&state);
    let doc = Path::new("/pmacs-lsp-latex-no-such-document.tex");
    assert!(
        !doc.exists(),
        "fixture precondition: {} must not exist",
        doc.display()
    );

    assert_eq!(
        resolve_root(&state, doc).as_deref(),
        Some("/"),
        "a document at the filesystem root roots at `/`; it must not \
         decline into the shared `.git`-aware detector"
    );
}

#[path = "common/iso.rs"]
mod iso;
