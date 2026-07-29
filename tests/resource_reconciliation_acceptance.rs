//! dired Stage 2a acceptance — rename and delete reconciliation.
//!
//! `docs/dired-stage2-framing.md` §5, §6, §10; acceptance items 23–38
//! and 50–55.
//!
//! **This suite contains no dired content.** Stage 2a ships no dired
//! surface at all: it is the substrate transaction that Stage 2b's `R`,
//! `D` and `x` then stand on, and it closes three defects on `main`
//! that need no dired to be worth fixing — the workspace-edit phantom
//! buffer, the raw first-match registry lookup both `apply_resource_op`
//! arms used, and the incomplete removal lifecycle.
//!
//! Two disciplines the framing forces on every row here:
//!
//! * **Drive the real entry point.** A reconciliation with no
//!   production caller passes every direct-call test, so the rename
//!   rows go through `pmacs.fs.rename` (worker-dispatched, harvested in
//!   the drain) or through `pmacs.buffer.apply_resource_op` (synchronous,
//!   main-thread), never through `EditorCore::reconcile_rename`.
//! * **Pump to quiescence, never to a frame count**, because every
//!   mutation is worker-dispatched.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pmacs::editor::EditorState;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn exec(state: &EditorState, source: &str) {
    state
        .lua_host
        .lua()
        .load(source.to_owned())
        .exec()
        .unwrap_or_else(|e| panic!("lua exec failed: {e}\n--- source ---\n{source}"));
}

fn eval<T: mlua::FromLuaMulti>(state: &EditorState, source: &str) -> T {
    state
        .lua_host
        .lua()
        .load(source.to_owned())
        .eval()
        .unwrap_or_else(|e| panic!("lua eval failed: {e}\n--- source ---\n{source}"))
}

/// Escape a path for embedding in a Lua double-quoted string.
fn lua_str(path: &Path) -> String {
    path.display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

/// Pump the async runtime until `predicate` holds or the deadline
/// lapses. Quiescence, not a frame count: the whole point of items 24
/// and 25 is that the reconciliation happens in the drain, and the drain
/// runs whenever a reply arrives.
fn pump_until<F: Fn(&EditorState) -> bool>(state: &mut EditorState, what: &str, predicate: F) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !predicate(state) {
        assert!(Instant::now() < deadline, "pump deadline exceeded: {what}");
        state.tick_async();
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Pump a fixed number of times without any expectation. Used only to
/// give a dispatched job every chance to settle before asserting that
/// something did **not** happen.
fn pump_a_while(state: &mut EditorState) {
    for _ in 0..80 {
        state.tick_async();
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// A canonicalized temp directory. Canonicalized because the buffer
/// registry stores lexically-normalized absolute paths and macOS's
/// `/var` is a symlink to `/private/var`; without this the expected
/// paths below would differ from the stored ones by a symlink hop.
struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Fixture {
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

    fn dir(&self, rel: &str) -> PathBuf {
        let path = self.root.join(rel);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn at(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }
}

fn editor() -> EditorState {
    let state = EditorState::new();
    // No language server may spawn from these fixtures. The LSP rows
    // that DO want one configure it explicitly.
    exec(&state, "pmacs.lsp.config = {}");
    state
}

/// Open `path` into a buffer and return the Lua global name holding its
/// handle. Buffers are held on globals so a test can re-read their path
/// and name after the reconciliation moved them.
fn open_as(state: &EditorState, global: &str, path: &Path) {
    exec(
        state,
        &format!(
            "_G.{global} = pmacs.buffer.find_or_open(\"{}\")",
            lua_str(path)
        ),
    );
}

fn buffer_path(state: &EditorState, global: &str) -> Option<String> {
    eval(
        state,
        &format!("local b = _G.{global}; return b and b:path() or nil"),
    )
}

fn buffer_name(state: &EditorState, global: &str) -> Option<String> {
    eval(
        state,
        &format!("local b = _G.{global}; return b and b:name() or nil"),
    )
}

fn buffer_is_valid(state: &EditorState, global: &str) -> bool {
    eval(
        state,
        &format!("local b = _G.{global}; return (b ~= nil) and b:is_valid()"),
    )
}

/// Dispatch a rename **without awaiting** the handle, then pump.
/// Fire-and-forget is the shape item 25 pins: the reconciliation must
/// not live at result consumption.
fn rename_fire_and_forget(state: &mut EditorState, from: &Path, to: &Path) {
    exec(
        state,
        &format!(
            "pmacs.fs.rename(\"{}\", \"{}\")",
            lua_str(from),
            lua_str(to)
        ),
    );
    pump_until(state, "rename lands on disk", |_| to.exists());
    // The rename landing on disk and the reply reaching the main thread
    // are two events; pump past the first to reach the second.
    pump_a_while(state);
}

fn remove_fire_and_forget(state: &mut EditorState, path: &Path) {
    exec(state, &format!("pmacs.fs.remove(\"{}\")", lua_str(path)));
    pump_until(state, "remove lands on disk", |_| !path.exists());
    pump_a_while(state);
}

// ---------------------------------------------------------------------------
// 25 — no-await rename
// ---------------------------------------------------------------------------

/// Acceptance 25. Dispatch `pmacs.fs.rename`, **never take the
/// result**, pump: the open buffer's path has moved.
///
/// Bite: fails if the reconciliation lives at result consumption
/// (`_take_result`) rather than in the drain, because nothing here ever
/// consumes the handle.
#[test]
fn acc25_a_never_awaited_rename_still_moves_the_open_buffers_path() {
    let fx = Fixture::new();
    let old = fx.write("notes.txt", "hello\n");
    let new = fx.at("renamed.txt");
    let mut state = editor();
    open_as(&state, "B", &old);
    assert_eq!(
        buffer_path(&state, "B").as_deref(),
        Some(old.to_str().unwrap())
    );

    rename_fire_and_forget(&mut state, &old, &new);

    assert_eq!(
        buffer_path(&state, "B").as_deref(),
        Some(new.to_str().unwrap()),
        "the buffer must follow a fire-and-forget rename"
    );
}

// ---------------------------------------------------------------------------
// 26, 27, 28 — the walk
// ---------------------------------------------------------------------------

/// Acceptance 26. A buffer open on `dir/child.txt` follows
/// `dir` → `newdir`.
#[test]
fn acc26_a_buffer_under_a_renamed_directory_follows_it() {
    let fx = Fixture::new();
    fx.dir("tree");
    let child = fx.write("tree/child.txt", "x\n");
    let mut state = editor();
    open_as(&state, "B", &child);

    rename_fire_and_forget(&mut state, &fx.at("tree"), &fx.at("newtree"));

    assert_eq!(
        buffer_path(&state, "B").as_deref(),
        Some(fx.at("newtree/child.txt").to_str().unwrap()),
        "a descendant keeps its relative tail under the new root"
    );
}

/// Acceptance 27. **Every** match, not the first: two descendant
/// buffers under the renamed directory *and* two buffers visiting the
/// same exact path all move.
///
/// Bite: fails against `find_by_path`'s first match. One child buffer
/// would not defeat a first-match implementation; this does, twice over
/// — and the duplicate-path pair is the case `find_by_path` cannot even
/// see, because it returns on the first hit.
#[test]
fn acc27_every_affected_buffer_moves_not_only_the_first() {
    let fx = Fixture::new();
    fx.dir("tree");
    let one = fx.write("tree/one.txt", "1\n");
    let two = fx.write("tree/nested/two.txt", "2\n");
    let mut state = editor();
    open_as(&state, "ONE", &one);
    open_as(&state, "TWO", &two);
    // Two buffers on the SAME exact path. `pmacs.buffer.from_file`
    // creates a fresh buffer without deduping, which is how a duplicate
    // path binding is reachable from public Lua.
    exec(
        &state,
        &format!("_G.DUP = pmacs.buffer.from_file(\"{}\")", lua_str(&one)),
    );
    let dup_first: String = eval(&state, "return _G.ONE:path()");
    let dup_second: String = eval(&state, "return _G.DUP:path()");
    assert_eq!(
        dup_first, dup_second,
        "precondition: two distinct buffers bound to one path"
    );

    rename_fire_and_forget(&mut state, &fx.at("tree"), &fx.at("newtree"));

    assert_eq!(
        buffer_path(&state, "ONE").as_deref(),
        Some(fx.at("newtree/one.txt").to_str().unwrap()),
        "first descendant"
    );
    assert_eq!(
        buffer_path(&state, "TWO").as_deref(),
        Some(fx.at("newtree/nested/two.txt").to_str().unwrap()),
        "second, more deeply nested descendant"
    );
    assert_eq!(
        buffer_path(&state, "DUP").as_deref(),
        Some(fx.at("newtree/one.txt").to_str().unwrap()),
        "the second buffer on the same path — invisible to a first-match \
         lookup, and left pointing at nothing by one"
    );
}

/// Acceptance 28. Renaming `/…/foo` must not rebind a buffer on
/// `/…/foobar`.
///
/// Bite: fails against a string `starts_with` instead of a
/// path-component prefix.
#[test]
fn acc28_a_false_string_prefix_is_not_a_path_prefix() {
    let fx = Fixture::new();
    fx.dir("foo");
    let inside = fx.write("foo/a.txt", "in\n");
    let sibling = fx.write("foobar.txt", "out\n");
    let mut state = editor();
    open_as(&state, "IN", &inside);
    open_as(&state, "OUT", &sibling);

    rename_fire_and_forget(&mut state, &fx.at("foo"), &fx.at("renamed"));

    assert_eq!(
        buffer_path(&state, "IN").as_deref(),
        Some(fx.at("renamed/a.txt").to_str().unwrap()),
        "the real descendant moves"
    );
    assert_eq!(
        buffer_path(&state, "OUT").as_deref(),
        Some(sibling.to_str().unwrap()),
        "`foobar.txt` shares a string prefix with `foo` and is not under it"
    );
}

/// Acceptance 28, delete side — **and this is the row that bites.**
///
/// The rename row above cannot falsify a string-prefix walk on its own:
/// `reconcile_rename` calls `Path::strip_prefix` to rebuild the
/// descendant's tail, and that is component-aware too, so a false
/// prefix match is silently dropped a second time and the buffer stays
/// put. Deletion has no such second guard — the walk's verdict IS the
/// kill list — so the containment rule has to be pinned here.
///
/// Bite: a string `starts_with` instead of `Path::starts_with` kills a
/// buffer on `foobar.txt` when `foo/` is deleted.
#[test]
fn acc28_delete_a_false_string_prefix_does_not_widen_the_kill_list() {
    let fx = Fixture::new();
    fx.dir("foo");
    let inside = fx.write("foo/a.txt", "in\n");
    let sibling = fx.write("foobar.txt", "out\n");
    let mut state = editor();
    open_as(&state, "KEEP", &fx.write("keep.txt", "k\n"));
    open_as(&state, "IN", &inside);
    open_as(&state, "OUT", &sibling);

    exec(
        &state,
        &format!(
            "pmacs.buffer.apply_resource_op {{ kind = \"delete\", \
               path = \"{}\", recursive = true }}",
            lua_str(&fx.at("foo"))
        ),
    );

    assert!(!fx.at("foo").exists(), "the directory is gone");
    assert!(
        !buffer_is_valid(&state, "IN"),
        "the real descendant is reconciled away"
    );
    assert!(
        buffer_is_valid(&state, "OUT"),
        "`foobar.txt` shares a string prefix with `foo` and is not under \
         it — killing it destroys an unrelated buffer whose file still \
         exists"
    );
    assert!(
        sibling.exists(),
        "and that file is indeed still on disk, which is what makes the \
         kill wrong rather than merely early"
    );
}

// ---------------------------------------------------------------------------
// 29 — name provenance, both directions
// ---------------------------------------------------------------------------

/// Acceptance 29(a). A buffer opened by a **relative** path is named
/// `foo.rs` while its stored path is absolute, and its name follows the
/// rename because its load site recorded `PathDerived`.
///
/// Bite: rev 7's string-equality rule fails this — the name never
/// equalled the normalized path, so it would have been left stale while
/// insisting it was user-chosen.
#[test]
fn acc29a_a_relative_opens_name_follows_the_rename() {
    let fx = Fixture::new();
    let old = fx.write("relative.txt", "x\n");
    let new = fx.at("moved.txt");
    let mut state = editor();
    // Open by a path whose *spelling* is not the stored path: a `.`
    // component is folded by normalization but kept in the name, which
    // reproduces the relative-open shape without depending on the
    // process cwd.
    let as_given = fx.at("./relative.txt");
    open_as(&state, "B", &as_given);
    assert_eq!(
        buffer_name(&state, "B").as_deref(),
        Some(as_given.to_str().unwrap()),
        "precondition: the name is the path AS GIVEN, not the stored path"
    );
    assert_eq!(
        buffer_path(&state, "B").as_deref(),
        Some(old.to_str().unwrap()),
        "precondition: only the stored path is normalized"
    );

    rename_fire_and_forget(&mut state, &old, &new);

    assert_eq!(
        buffer_path(&state, "B").as_deref(),
        Some(new.to_str().unwrap())
    );
    assert_eq!(
        buffer_name(&state, "B").as_deref(),
        Some(new.to_str().unwrap()),
        "the name follows because the load site recorded path provenance, \
         not because the old name happened to match the old path"
    );
}

/// Acceptance 29(b). A name set explicitly through
/// `pmacs.buffer.set_name` survives the rename **even when that string
/// normalizes to the file's own stored path**.
///
/// Bite: rev 8's path-equivalence heuristic fails this — the chosen
/// name normalizes to the exact stored path, so the heuristic would
/// overwrite it.
#[test]
fn acc29b_an_explicitly_set_name_survives_even_when_it_denotes_the_file() {
    let fx = Fixture::new();
    let old = fx.write("notes", "x\n");
    let new = fx.at("notes-renamed");
    let mut state = editor();
    open_as(&state, "B", &old);
    // The chosen name IS the file's absolute path. Under a
    // path-equivalence rule this is indistinguishable from a
    // path-derived name; under recorded provenance it is not.
    exec(
        &state,
        &format!("pmacs.buffer.set_name(_G.B, \"{}\")", lua_str(&old)),
    );
    assert_eq!(
        buffer_name(&state, "B").as_deref(),
        Some(old.to_str().unwrap()),
        "precondition: the explicit name normalizes to the stored path"
    );

    rename_fire_and_forget(&mut state, &old, &new);

    assert_eq!(
        buffer_path(&state, "B").as_deref(),
        Some(new.to_str().unwrap()),
        "the PATH always follows"
    );
    assert_eq!(
        buffer_name(&state, "B").as_deref(),
        Some(old.to_str().unwrap()),
        "and the explicitly chosen name does not, however much it looks \
         like a path-derived one"
    );
}

// ---------------------------------------------------------------------------
// 36, 37 — the synchronous arm, and failure
// ---------------------------------------------------------------------------

/// Acceptance 36. `apply_resource_op`'s rename finds a buffer whose
/// stored path is normalized but whose op names it **un-normalized**.
///
/// Bite: fails against the raw `find_by_path(&from)` this arm used —
/// stored paths are normalized on write, so a raw lookup with a `.`
/// component in it misses the buffer entirely and the rename silently
/// reconciles nothing.
#[test]
fn acc36_the_synchronous_arm_matches_an_un_normalized_op_path() {
    let fx = Fixture::new();
    let old = fx.write("sync.txt", "x\n");
    let new = fx.at("sync-moved.txt");
    let state = editor();
    open_as(&state, "B", &old);

    let unnormalized = fx.at("./sync.txt");
    exec(
        &state,
        &format!(
            "pmacs.buffer.apply_resource_op {{ kind = \"rename\", \
               old_path = \"{}\", new_path = \"{}\" }}",
            lua_str(&unnormalized),
            lua_str(&new)
        ),
    );

    assert!(new.exists(), "the rename happened on disk");
    assert_eq!(
        buffer_path(&state, "B").as_deref(),
        Some(new.to_str().unwrap()),
        "the buffer must be found even though the op spelled the source \
         path differently from the stored one"
    );
}

/// Acceptance 37. A **failed** rename reconciles nothing — and fires no
/// hook.
#[test]
fn acc37_a_failed_rename_reconciles_nothing_and_fires_no_hook() {
    let fx = Fixture::new();
    let present = fx.write("present.txt", "x\n");
    let missing = fx.at("does-not-exist.txt");
    let mut state = editor();
    open_as(&state, "B", &present);
    exec(
        &state,
        "_G.FIRED = 0
         pmacs.hook.add('resource.renamed', function() _G.FIRED = _G.FIRED + 1 end)",
    );

    // Renaming a path that does not exist fails in the worker.
    exec(
        &state,
        &format!(
            "pmacs.fs.rename(\"{}\", \"{}\")",
            lua_str(&missing),
            lua_str(&fx.at("target.txt"))
        ),
    );
    pump_a_while(&mut state);

    let fired: i64 = eval(&state, "return _G.FIRED");
    assert_eq!(fired, 0, "a failed mutation reconciles nothing");
    assert_eq!(
        buffer_path(&state, "B").as_deref(),
        Some(present.to_str().unwrap()),
        "and no unrelated buffer moved"
    );
}

// ---------------------------------------------------------------------------
// 50, 55 — the hooks
// ---------------------------------------------------------------------------

/// Acceptance 50. `resource.renamed` fires **exactly once** per
/// successful rename, with `(old, new)` as normalized absolute paths,
/// and does not fire for a rename that failed. The symmetric assertion
/// for `resource.deleted` accompanies it.
///
/// Bite: fails if the hook fires for a failed rename, or fires with the
/// un-normalized path a caller happened to spell.
#[test]
fn acc50_the_hooks_fire_once_with_normalized_paths() {
    let fx = Fixture::new();
    let old = fx.write("hooked.txt", "x\n");
    let new = fx.at("hooked-moved.txt");
    let doomed = fx.write("doomed.txt", "y\n");
    let mut state = editor();
    exec(
        &state,
        "_G.RENAMES = {}
         _G.DELETES = {}
         pmacs.hook.add('resource.renamed', function(a, b)
           _G.RENAMES[#_G.RENAMES + 1] = tostring(a) .. ' -> ' .. tostring(b)
         end)
         pmacs.hook.add('resource.deleted', function(p)
           _G.DELETES[#_G.DELETES + 1] = tostring(p)
         end)",
    );

    // Spell BOTH paths un-normalized, so the hook's arguments can only
    // be canonical if the fire site normalizes them.
    exec(
        &state,
        &format!(
            "pmacs.fs.rename(\"{}\", \"{}\")",
            lua_str(&fx.at("./hooked.txt")),
            lua_str(&fx.at("./hooked-moved.txt"))
        ),
    );
    pump_until(&mut state, "rename hook", |s| {
        let n: i64 = eval(s, "return #_G.RENAMES");
        n > 0
    });
    pump_a_while(&mut state);

    let renames: String = eval(&state, "return table.concat(_G.RENAMES, '|')");
    assert_eq!(
        renames,
        format!("{} -> {}", old.display(), new.display()),
        "exactly one row, and both paths canonical — a path-keyed \
         subscriber needs the form the registry keys on"
    );

    exec(
        &state,
        &format!("pmacs.fs.remove(\"{}\")", lua_str(&fx.at("./doomed.txt"))),
    );
    pump_until(&mut state, "delete hook", |s| {
        let n: i64 = eval(s, "return #_G.DELETES");
        n > 0
    });
    pump_a_while(&mut state);
    let deletes: String = eval(&state, "return table.concat(_G.DELETES, '|')");
    assert_eq!(
        deletes,
        doomed.display().to_string(),
        "one row, canonical path"
    );
}

/// Acceptance 55. Both hooks are `all-must-succeed`, not
/// short-circuit: with two subscribers registered and the **first one
/// raising**, the second still runs and the error is reported rather
/// than swallowed.
///
/// Bite: fails against a `short-circuit` registration, where the first
/// subscriber's return would stop the fan-out and silently prevent every
/// later one from reconciling — which no test asserting only "the hook
/// fired" would catch.
#[test]
fn acc55_a_raising_subscriber_does_not_stop_the_fan_out() {
    let fx = Fixture::new();
    let old = fx.write("fanout.txt", "x\n");
    let new = fx.at("fanout-moved.txt");
    let doomed = fx.write("fanout-doomed.txt", "y\n");
    let mut state = editor();
    exec(
        &state,
        "_G.SECOND_RAN = 0
         _G.SECOND_DELETED = 0
         pmacs.hook.add('resource.renamed', function() error('first subscriber explodes') end)
         pmacs.hook.add('resource.renamed', function() _G.SECOND_RAN = _G.SECOND_RAN + 1 end)
         pmacs.hook.add('resource.deleted', function() error('first subscriber explodes') end)
         pmacs.hook.add('resource.deleted', function() _G.SECOND_DELETED = _G.SECOND_DELETED + 1 end)",
    );

    rename_fire_and_forget(&mut state, &old, &new);
    let ran: i64 = eval(&state, "return _G.SECOND_RAN");
    assert_eq!(
        ran, 1,
        "`all-must-succeed` collects the first callback's error and \
         continues; a short-circuit registration would have stopped here"
    );

    remove_fire_and_forget(&mut state, &doomed);
    let deleted: i64 = eval(&state, "return _G.SECOND_DELETED");
    assert_eq!(deleted, 1, "same for `resource.deleted`");

    // The error is reported, not swallowed: the hook error log is the
    // `*errors*` buffer.
    let errors: String = eval(
        &state,
        "for _, b in ipairs(pmacs.buffer.list()) do
           if b:name() == '*errors*' then return b:slice(0, b:len()) end
         end
         return ''",
    );
    assert!(
        errors.contains("first subscriber explodes"),
        "the raising subscriber's error must be reported; *errors* held: \
         {errors:?}"
    );
}

// ---------------------------------------------------------------------------
// 23, 24 — the delete lookups
// ---------------------------------------------------------------------------

/// Acceptance 23. `apply_resource_op`'s delete reaches **descendants**
/// and a **second buffer on the same path** — the raw first-match lookup
/// replaced by the shared prefix-aware, normalizing query.
///
/// (#190 owns the modified-buffer refusal; it refuses before disk, so by
/// the time this lane's reconciliation runs there is no modified buffer
/// on the synchronous path to spare. This row asserts the lookup fix.)
#[test]
fn acc23_the_synchronous_delete_reaches_descendants_and_duplicates() {
    let fx = Fixture::new();
    fx.dir("tree/nested");
    let one = fx.write("tree/one.txt", "1\n");
    fx.write("tree/nested/two.txt", "2\n");
    let state = editor();
    open_as(&state, "ONE", &one);
    open_as(&state, "TWO", &fx.at("tree/nested/two.txt"));
    exec(
        &state,
        &format!("_G.DUP = pmacs.buffer.from_file(\"{}\")", lua_str(&one)),
    );
    // Keep an unrelated buffer alive so the last-buffer refusal is not
    // what this row measures.
    open_as(&state, "KEEP", &fx.write("keep.txt", "k\n"));

    exec(
        &state,
        &format!(
            "pmacs.buffer.apply_resource_op {{ kind = \"delete\", \
               path = \"{}\", recursive = true }}",
            lua_str(&fx.at("./tree"))
        ),
    );

    assert!(!fx.at("tree").exists(), "the tree is gone from disk");
    assert!(
        !buffer_is_valid(&state, "ONE"),
        "a buffer directly under the deleted directory"
    );
    assert!(
        !buffer_is_valid(&state, "TWO"),
        "a more deeply nested descendant"
    );
    assert!(
        !buffer_is_valid(&state, "DUP"),
        "the second buffer on the same path — the one a first-match \
         lookup cannot see, which #190 deliberately left in place \
         because it had no two-phase kill to route it through"
    );
    assert!(buffer_is_valid(&state, "KEEP"), "an unrelated buffer");
}

/// Acceptance 24. A **fire-and-forget** `pmacs.fs.remove` reconciles
/// too: never taking the handle still kills the unmodified buffer, which
/// is what makes the drain harvest the right seam rather than dired
/// firing the hook itself.
#[test]
fn acc24_a_never_awaited_remove_still_kills_the_unmodified_buffer() {
    let fx = Fixture::new();
    let doomed = fx.write("gone.txt", "x\n");
    let mut state = editor();
    open_as(&state, "KEEP", &fx.write("keep.txt", "k\n"));
    open_as(&state, "B", &doomed);
    assert!(buffer_is_valid(&state, "B"));

    remove_fire_and_forget(&mut state, &doomed);

    assert!(
        !buffer_is_valid(&state, "B"),
        "the harvest must reconcile a delete no one awaited"
    );
    assert!(buffer_is_valid(&state, "KEEP"));
}

/// Acceptance 18's substrate half, and §6's modified case: a **modified**
/// buffer whose file is deleted out from under it keeps its contents. The
/// buffer half is the part of the promise that is robust, because it runs
/// at drain time against whatever state exists then.
#[test]
fn a_modified_buffer_survives_a_delete_with_its_contents() {
    let fx = Fixture::new();
    let doomed = fx.write("dirty.txt", "original\n");
    let mut state = editor();
    open_as(&state, "KEEP", &fx.write("keep.txt", "k\n"));
    open_as(&state, "B", &doomed);
    exec(&state, "_G.B:insert(0, 'edited ')");
    let modified: bool = eval(&state, "return _G.B:is_modified()");
    assert!(modified, "precondition");

    remove_fire_and_forget(&mut state, &doomed);

    assert!(
        buffer_is_valid(&state, "B"),
        "a modified buffer is kept alive deliberately, not killed"
    );
    let contents: String = eval(&state, "return _G.B:slice(0, _G.B:len())");
    assert_eq!(contents, "edited original\n", "with its contents intact");
    assert!(!doomed.exists(), "while the file is gone");
}

// ---------------------------------------------------------------------------
// 51, 52, 53 — the removal lifecycle
// ---------------------------------------------------------------------------

/// Acceptance 51. A killed buffer completes **both** removal phases:
/// after a delete reconciles, an `on_removed` callback registered for
/// that buffer has fired and its buffer-local keymap entries are gone.
///
/// Bite: fails against an implementation that calls only
/// `EditorCore::kill_buffer`, which does no phase-2 cleanup at all.
///
/// Note 51 and 52 are a matched pair and **neither alone is
/// sufficient** — each pre-existing removal path passes one and fails
/// the other, which is exactly why both phases had to be named.
#[test]
fn acc51_a_killed_buffer_completes_both_removal_phases() {
    let fx = Fixture::new();
    let doomed = fx.write("phase2.txt", "x\n");
    let mut state = editor();
    open_as(&state, "KEEP", &fx.write("keep.txt", "k\n"));
    open_as(&state, "B", &doomed);
    exec(
        &state,
        "_G.ON_REMOVED = 0
         pmacs.buffer.on_removed(_G.B, function() _G.ON_REMOVED = _G.ON_REMOVED + 1 end)
         pmacs.command.define { name = 'test.noop', description = 'x', fn = function() end }
         pmacs.keymap.bind { scope = 'buffer', buffer = _G.B,
                             sequence = 'C-c C-1', command = 'test.noop' }
         -- `pmacs.keymap.lookup` is deliberately raw-global, so the
         -- buffer-scoped row is only visible through `list()`.
         function _G.BOUND_ROWS()
           local n = 0
           for _, e in ipairs(pmacs.keymap.list()) do
             if e.command == 'test.noop' then n = n + 1 end
           end
           return n
         end",
    );
    let bound_before: i64 = eval(&state, "return _G.BOUND_ROWS()");
    assert_eq!(
        bound_before, 1,
        "precondition: the buffer-local binding exists"
    );

    remove_fire_and_forget(&mut state, &doomed);

    assert!(!buffer_is_valid(&state, "B"), "phase 1 removed the buffer");
    let fired: i64 = eval(&state, "return _G.ON_REMOVED");
    assert_eq!(
        fired, 1,
        "phase 2 must fire the registered on_removed callback; 0 means the \
         reconciliation called `EditorCore::kill_buffer` alone"
    );
    let bound_after: i64 = eval(&state, "return _G.BOUND_ROWS()");
    assert_eq!(
        bound_after, 0,
        "phase 2 must purge the buffer-scoped keymap, so a later buffer \
         cannot inherit a dead one's bindings"
    );
}

/// Acceptance 52. A window displaying the deleted buffer is
/// **redirected**, not left dangling: no window holds a removed id.
///
/// Bite: fails against `remove_buffer_and_fire`, which is what
/// `apply_resource_op` used — phase 2 without phase 1, so
/// `BufferRegistry::remove` runs while every window showing the buffer
/// keeps pointing at the id it just dropped.
#[test]
fn acc52_a_window_showing_the_deleted_buffer_is_redirected() {
    let fx = Fixture::new();
    let doomed = fx.write("shown.txt", "x\n");
    let state = editor();
    open_as(&state, "KEEP", &fx.write("keep.txt", "k\n"));
    open_as(&state, "B", &doomed);
    exec(&state, "pmacs.window.switch_buffer(_G.B)");
    let doomed_id = state.core.borrow().active_buffer_id();
    assert!(
        state
            .core
            .borrow()
            .windows
            .values()
            .any(|w| w.buffer_id == doomed_id),
        "precondition: a window shows the doomed buffer"
    );

    exec(
        &state,
        &format!(
            "pmacs.buffer.apply_resource_op {{ kind = \"delete\", path = \"{}\" }}",
            lua_str(&doomed)
        ),
    );

    let core = state.core.borrow();
    assert!(
        !core.registry.borrow().contains(doomed_id),
        "the buffer was removed"
    );
    let dangling: Vec<_> = core
        .windows
        .iter()
        .filter(|(_, w)| !core.registry.borrow().contains(w.buffer_id))
        .map(|(id, w)| (*id, w.buffer_id))
        .collect();
    assert!(
        dangling.is_empty(),
        "no window may hold a removed buffer id; dangling: {dangling:?}"
    );
    assert!(
        !core.windows.values().any(|w| w.buffer_id == doomed_id),
        "and specifically not the deleted one"
    );
}

/// Acceptance 53. The last-buffer and mid-edit refusals are
/// **reported, not silent**, and neither aborts the reconciliation of
/// other buffers.
#[test]
fn acc53_the_last_buffer_refusal_keeps_the_buffer_and_the_rest_proceeds() {
    // Half one: the file behind the only open buffer. `kill_buffer`
    // refuses to remove the last remaining buffer, so the file goes and
    // the buffer stays.
    let fx = Fixture::new();
    let only = fx.write("only.txt", "x\n");
    let mut state = editor();
    // Drop every other buffer so the target really is the last one.
    exec(
        &state,
        &format!(
            "_G.ONLY = pmacs.buffer.find_or_open(\"{}\")
             for _, b in ipairs(pmacs.buffer.list()) do
               if tostring(b) ~= tostring(_G.ONLY) then
                 pcall(pmacs.buffer.kill, b)
               end
             end
             return #pmacs.buffer.list()",
            lua_str(&only)
        ),
    );
    let count: i64 = eval(&state, "return #pmacs.buffer.list()");
    assert_eq!(count, 1, "precondition: exactly one buffer is open");

    remove_fire_and_forget(&mut state, &only);
    assert!(
        buffer_is_valid(&state, "ONLY"),
        "the last remaining buffer cannot be killed, so it survives the \
         deletion of its file"
    );

    // Half two: a directory of buffers where one refuses removal. The
    // rest must still reconcile.
    let fx2 = Fixture::new();
    fx2.dir("batch");
    let a = fx2.write("batch/a.txt", "a\n");
    let b = fx2.write("batch/b.txt", "b\n");
    let c = fx2.write("batch/c.txt", "c\n");
    let mut state2 = editor();
    open_as(&state2, "KEEP", &fx2.write("keep.txt", "k\n"));
    open_as(&state2, "A", &a);
    open_as(&state2, "B", &b);
    open_as(&state2, "C", &c);
    // B refuses: it is modified.
    exec(&state2, "_G.B:insert(0, 'dirty ')");

    remove_fire_and_forget(&mut state2, &fx2.at("batch/a.txt"));
    remove_fire_and_forget(&mut state2, &fx2.at("batch/b.txt"));
    remove_fire_and_forget(&mut state2, &fx2.at("batch/c.txt"));

    assert!(!buffer_is_valid(&state2, "A"), "A reconciled");
    assert!(
        buffer_is_valid(&state2, "B"),
        "B was kept because it is modified"
    );
    assert!(
        !buffer_is_valid(&state2, "C"),
        "and C still reconciled afterwards — one refusal must not abort \
         the rest"
    );
}

// ---------------------------------------------------------------------------
// 53b — a mid-edit refusal leaves editor state UNCHANGED
// ---------------------------------------------------------------------------

/// Acceptance 53b, all three assertions, **stated individually**.
///
/// With the buffer `editing_in_progress`, displayed in an ordinary
/// window, shown in a side window, and present in `round_trip_buffers`,
/// a delete reconciling it must leave each of the following provably
/// untouched. Each fails independently against the same one-line bite —
/// removing the `editing_in_progress` preflight — which is the point: a
/// single compound assertion can pass on two of the three and hide the
/// third.
///
/// | # | Assertion | What the missing preflight breaks |
/// |---|---|---|
/// | i | the ordinary window still shows the buffer, cursor/selection/`view_top` intact | `kill_buffer` redirects the window to the fallback before `BufferRegistry::remove` refuses |
/// | ii | the side window is still open and still shows the buffer | `remove_side_window` collapses it first |
/// | iii | the buffer is still in `round_trip_buffers` | `round_trip_buffers.remove` runs first — the **first** thing `kill_buffer` does, and the easiest to miss |
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "three independent assertions, each with its own precondition;               a compound check is exactly what this row exists to avoid"
)]
fn acc53b_a_mid_edit_refusal_leaves_window_side_and_round_trip_state_untouched() {
    let fx = Fixture::new();
    let doomed = fx.write("midedit.txt", "0123456789\nsecond line\n");
    let mut state = editor();
    // A grid frontend's real frame size is its declaration, and a side
    // window needs one before it can be placed.
    state.sync_frame_geometry(
        pmacs::protocol::FrontendId::LOCAL,
        pmacs::cell::CellSize::new(24, 80),
    );
    open_as(&state, "KEEP", &fx.write("keep.txt", "k\n"));
    open_as(&state, "B", &doomed);
    exec(&state, "pmacs.window.switch_buffer(_G.B)");
    exec(&state, "pmacs.buffer.set_round_trip_input(_G.B, true)");

    let doomed_id = state.core.borrow().active_buffer_id();
    let ordinary_window = state.core.borrow().active_window_id();
    // Seat a distinctive cursor + selection + scroll position on the
    // ORDINARY window, so a redirect is detectable as more than "the
    // window moved".
    {
        let mut core = state.core.borrow_mut();
        let win = core
            .windows
            .get_mut(&ordinary_window)
            .expect("ordinary window");
        win.cursor = 4;
        win.selection = Some(pmacs::window::Selection { anchor: 2 });
        win.view_top = 1;
    }

    // A SIDE window showing the same buffer, so the collapse the
    // preflight prevents has something to collapse.
    exec(
        &state,
        "pmacs.window.display(_G.B, { side = \"bottom\", height = 4 })",
    );
    let side_windows: Vec<_> = state
        .core
        .borrow()
        .side_window_for(pmacs::protocol::FrontendId::LOCAL)
        .into_iter()
        .collect();
    assert!(
        !side_windows.is_empty(),
        "precondition: a side window exists"
    );
    assert_eq!(
        state
            .core
            .borrow()
            .windows
            .get(&side_windows[0])
            .expect("side window")
            .buffer_id,
        doomed_id,
        "precondition: the side window shows the doomed buffer"
    );
    assert_ne!(
        side_windows[0], ordinary_window,
        "precondition: the side window is a second window"
    );
    assert!(
        state.core.borrow().buffer_round_trips(doomed_id),
        "precondition: the buffer round-trips input"
    );

    // Put the buffer mid-edit. `begin_edit` is the flag
    // `BufferRegistry::remove` refuses on, and the whole point of the
    // preflight is that the refusal arrives too late.
    {
        let core = state.core.borrow();
        let mut reg = core.registry.borrow_mut();
        reg.get_mut(doomed_id)
            .expect("doomed buffer")
            .begin_edit()
            .expect("begin edit");
    }

    remove_fire_and_forget(&mut state, &doomed);

    let core = state.core.borrow();
    // (i) the ordinary window, with its seated state.
    let win = core
        .windows
        .get(&ordinary_window)
        .expect("the ordinary window still exists");
    assert_eq!(
        win.buffer_id, doomed_id,
        "(i) the ordinary window must still show the buffer"
    );
    assert_eq!(win.cursor, 4, "(i) cursor");
    assert_eq!(
        win.selection,
        Some(pmacs::window::Selection { anchor: 2 }),
        "(i) selection"
    );
    assert_eq!(win.view_top, 1, "(i) view_top");

    // (ii) the side window.
    for side in &side_windows {
        let side_win = core.windows.get(side).unwrap_or_else(|| {
            panic!(
                "(ii) side window {side:?} was collapsed by a kill that should never have started"
            )
        });
        assert_eq!(
            side_win.buffer_id, doomed_id,
            "(ii) the side window must still show the buffer"
        );
    }

    // (iii) round-trip membership.
    assert!(
        core.buffer_round_trips(doomed_id),
        "(iii) the buffer must still round-trip input — this is the FIRST \
         thing `kill_buffer` drops and the easiest to miss"
    );

    assert!(
        core.registry.borrow().contains(doomed_id),
        "and the buffer itself is still in the registry"
    );
}

// ---------------------------------------------------------------------------
// 54 — independent mutations both reconcile, in either arrival order
// ---------------------------------------------------------------------------

/// Acceptance 54, integration layer. Dispatch a rename and a delete on
/// **disjoint** paths, wait for both, and assert both registry effects
/// occurred. It fails against dropping or deduplicating one resource
/// kind.
///
/// The disjoint end state is confidence coverage, **not** a claimed bite
/// against interdependent sequencing: disjoint paths necessarily
/// commute. The controlled-bus layer that does pin arrival order lives
/// in `src/async_runtime.rs`, and no test here pretends to pin an order
/// the mechanism does not establish.
#[test]
fn acc54_a_rename_and_a_delete_on_disjoint_paths_both_reconcile() {
    for reverse in [false, true] {
        let fx = Fixture::new();
        let renamed_from = fx.write("moves.txt", "m\n");
        let renamed_to = fx.at("moved.txt");
        let deleted = fx.write("goes.txt", "g\n");
        let mut state = editor();
        open_as(&state, "KEEP", &fx.write("keep.txt", "k\n"));
        open_as(&state, "MOVES", &renamed_from);
        open_as(&state, "GOES", &deleted);

        let rename = format!(
            "pmacs.fs.rename(\"{}\", \"{}\")",
            lua_str(&renamed_from),
            lua_str(&renamed_to)
        );
        let remove = format!("pmacs.fs.remove(\"{}\")", lua_str(&deleted));
        if reverse {
            exec(&state, &remove);
            exec(&state, &rename);
        } else {
            exec(&state, &rename);
            exec(&state, &remove);
        }

        pump_until(&mut state, "both mutations", |s| {
            renamed_to.exists() && !deleted.exists() && !buffer_is_valid(s, "GOES")
        });
        pump_a_while(&mut state);

        assert_eq!(
            buffer_path(&state, "MOVES").as_deref(),
            Some(renamed_to.to_str().unwrap()),
            "the rename reconciled (dispatch order reversed: {reverse})"
        );
        assert!(
            !buffer_is_valid(&state, "GOES"),
            "the delete reconciled (dispatch order reversed: {reverse})"
        );
    }
}

// ---------------------------------------------------------------------------
// The LSP-facing rows (30, 31c, 32, 34, 35)
// ---------------------------------------------------------------------------
//
// Driven against `pmacs_fake_lsp` so nothing here needs a real toolchain
// on PATH. The fake publishes two synthetic diagnostics (one Error at
// line 0, one Warning at line 2) on every `didOpen`, which is what makes
// "the new URI's diagnostics" observable at all.

fn fake_lsp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_lsp").to_owned()
}

/// Configure `language` to spawn the fake, and pin the project-marker
/// walk to `root` so a stray `.git` above the tempdir cannot silently
/// turn a markerless fixture into a detected one.
fn configure_fake(state: &EditorState, root: &Path, language: &str) {
    exec(
        state,
        &format!(
            "pmacs.project.set_search_boundary(\"{}\")
             pmacs.lsp.config.{language} = {{ command = \"{}\" }}",
            lua_str(root),
            fake_lsp_path()
        ),
    );
}

/// Pump the real frame order — processes, LSP, async — until `predicate`
/// holds. All three are needed: the fake's frames arrive through the
/// supervisor, the manager parses them, and the rename settles on the
/// async bus.
fn settle_until<F: Fn(&mut EditorState) -> bool>(
    state: &mut EditorState,
    what: &str,
    predicate: F,
) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !predicate(state) {
        assert!(
            Instant::now() < deadline,
            "settle deadline exceeded: {what}"
        );
        state.tick_processes();
        state.tick_lsp();
        state.tick_async();
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn settle_a_while(state: &mut EditorState) {
    for _ in 0..120 {
        state.tick_processes();
        state.tick_lsp();
        state.tick_async();
        std::thread::sleep(Duration::from_millis(3));
    }
}

fn server_count(state: &EditorState) -> i64 {
    eval(state, "return #pmacs.lsp.list()")
}

/// `language|root_uri|state` per live server, sorted, so assertions do
/// not depend on spawn order.
fn server_rows(state: &EditorState) -> Vec<String> {
    let joined: String = eval(
        state,
        r#"
        local out = {}
        for _, s in ipairs(pmacs.lsp.list()) do
          out[#out + 1] = table.concat({
            s.language_id or "", s.root_uri or "",
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

fn diag_count(state: &EditorState, uri: &str) -> i64 {
    eval(
        state,
        &format!(
            "local e, w, i, h = pmacs.diag.count(\"{uri}\")
             return (e or 0) + (w or 0) + (i or 0) + (h or 0)"
        ),
    )
}

/// Mirror of `file_uri_for` in `builtin/runtime/lsp.lua`. Reimplemented
/// rather than imported, so the test states the expected encoding
/// independently of the code under test.
fn file_uri(path: &Path) -> String {
    let mut out = String::from("file://");
    for byte in path.display().to_string().as_bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' | b':' => {
                out.push(*byte as char);
            }
            _ => {
                use std::fmt::Write as _;
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

/// Every window's overlay kinds, in composition order, keyed by window.
fn overlay_kinds_per_window(state: &EditorState) -> Vec<(u64, Vec<&'static str>)> {
    let core = state.core.borrow();
    let mut rows: Vec<(u64, Vec<&'static str>)> = core
        .windows
        .iter()
        .map(|(id, w)| (id.raw(), w.overlay_kinds()))
        .collect();
    rows.sort_by_key(|(id, _)| *id);
    rows
}

/// Count cells carrying a diagnostic **error** underline colour, per
/// window rect, by painting one real frame.
///
/// This is the only per-window, view-level observation available:
/// `DiagnosticView.uri` is private and `View` has no downcast, so
/// asserting on the store would prove nothing about whether the overlay
/// was re-rooted. A view still pointing at the old URI renders nothing,
/// because `forget_uri` emptied that key.
fn error_underlines_per_window(state: &EditorState) -> Vec<(u64, usize)> {
    use pmacs::cell::{Cell, CellGrid, CellSize, Color};
    use pmacs::protocol::FrontendId;
    use pmacs::window::Rect;

    let size = CellSize::new(24, 80);
    let mut cells = vec![Cell::default(); (size.rows * size.cols) as usize];
    let mut grid = CellGrid {
        cells: &mut cells,
        stride: size.cols,
        size,
    };
    pmacs::editor::paint_frame(
        state,
        FrontendId::LOCAL,
        &std::collections::HashMap::new(),
        &mut grid,
        size,
    );
    let placements = {
        let core = state.core.borrow();
        let view = core.views.get(&FrontendId::LOCAL).expect("LOCAL view");
        let area = Rect::new(0, 0, size.rows - 1, size.cols);
        let fixed = core.panel_fixed_rows(FrontendId::LOCAL, area.size.rows);
        view.layout.compute(area, &fixed)
    };
    let error = Color::Indexed(1);
    let mut rows: Vec<(u64, usize)> = placements
        .into_iter()
        .map(|(win, rect)| {
            let mut n = 0;
            for row in rect.origin.row..rect.origin.row + rect.size.rows {
                for col in rect.origin.col..rect.origin.col + rect.size.cols {
                    let idx = (row * size.cols + col) as usize;
                    if cells.get(idx).map(|c| c.style.underline_color) == Some(error) {
                        n += 1;
                    }
                }
            }
            (win.raw(), n)
        })
        .collect();
    rows.sort_by_key(|(id, _)| *id);
    rows
}

/// Acceptance 30. An attached LSP buffer with **diagnostics present
/// before** the rename, shown in **at least two windows**: afterwards
/// both windows render the **new** URI's diagnostics, the old URI's
/// store is empty, and each window's overlay keeps its **position in
/// the composition order**.
///
/// Bite, two mutations: `rec.uri` updated without re-rooting the
/// diagnostic view (both windows then render nothing, because the old
/// key is empty); and a remove-and-re-push, which would pass a
/// one-window render test while moving the diagnostic overlay to the end
/// of the stack — caught by the composition-order assertion.
#[test]
fn acc30_diagnostics_re_root_in_every_window_and_keep_their_stack_position() {
    let fx = Fixture::new();
    fx.write("proj/Cargo.toml", "[package]\nname = \"p\"\n");
    let old = fx.write(
        "proj/src/main.rs",
        "fn main() {}\n// second\n// third line here\n",
    );
    let new = fx.at("proj/src/renamed.rs");
    let mut state = editor();
    state.sync_frame_geometry(
        pmacs::protocol::FrontendId::LOCAL,
        pmacs::cell::CellSize::new(24, 80),
    );
    configure_fake(&state, &fx.root, "rust");
    open_as(&state, "B", &old);

    let old_uri = file_uri(&old);
    let new_uri = file_uri(&new);
    settle_until(&mut state, "diagnostics for the old URI", |s| {
        diag_count(s, &old_uri) > 0
    });

    // A second window showing the same buffer, with its own
    // `DiagnosticView`. A split alone does not carry one — the view
    // does not implement `clone_for_split` — so the switch hook is what
    // attaches it, and that path only ever touches the ACTIVE window.
    exec(&state, "pmacs.window.split_horizontal()");
    // `try_split_active` leaves focus where it was, so the switch hook —
    // which can only reach the ACTIVE window — has to be given the new
    // one explicitly.
    exec(&state, "pmacs.window.focus_next()");
    exec(&state, "pmacs.window.switch_buffer(_G.B)");
    settle_a_while(&mut state);

    // Push one more overlay AFTER the diagnostic in each window.
    // Without this the composition-order assertion below cannot bite:
    // the LSP attach leaves `diagnostic` LAST in the stack, and moving
    // the last element to the end is a no-op, so a remove-and-re-push
    // would be indistinguishable from an in-place mutation.
    // `_attach_highlight` uses `push_overlay` (no dedup), so a second
    // call appends.
    exec(
        &state,
        "pmacs.parse._attach_highlight(_G.B, pmacs.parse.buffer_language(_G.B))",
    );
    exec(&state, "pmacs.window.focus_next()");
    exec(
        &state,
        "pmacs.parse._attach_highlight(_G.B, pmacs.parse.buffer_language(_G.B))",
    );

    let before_kinds = overlay_kinds_per_window(&state);
    assert!(
        before_kinds
            .iter()
            .all(|(_, kinds)| kinds.iter().position(|k| *k == "diagnostic")
                < Some(kinds.len() - 1)),
        "precondition: the diagnostic overlay must NOT be last, or \
         \"keeps its stack position\" is unfalsifiable; got {before_kinds:?}"
    );
    let before_paint = error_underlines_per_window(&state);
    assert_eq!(
        before_paint.len(),
        2,
        "precondition: two windows are placed; got {before_paint:?}"
    );
    for (win, n) in &before_paint {
        assert!(
            *n > 0,
            "precondition: window {win} must already paint diagnostic \
             underlines; got {before_paint:?} with overlays \
             {before_kinds:?}"
        );
    }
    let diag_positions_before: Vec<(u64, Option<usize>)> = before_kinds
        .iter()
        .map(|(w, kinds)| (*w, kinds.iter().position(|k| *k == "diagnostic")))
        .collect();
    assert!(
        diag_positions_before.iter().all(|(_, p)| p.is_some()),
        "precondition: every window carries a diagnostic overlay; got \
         {before_kinds:?}"
    );

    rename_fire_and_forget(&mut state, &old, &new);
    settle_until(&mut state, "diagnostics for the new URI", |s| {
        diag_count(s, &new_uri) > 0
    });
    settle_a_while(&mut state);

    assert_eq!(
        diag_count(&state, &old_uri),
        0,
        "the old URI's store must be empty"
    );
    assert!(
        diag_count(&state, &new_uri) > 0,
        "and the new URI's must be populated"
    );

    let after_kinds = overlay_kinds_per_window(&state);
    let diag_positions_after: Vec<(u64, Option<usize>)> = after_kinds
        .iter()
        .map(|(w, kinds)| (*w, kinds.iter().position(|k| *k == "diagnostic")))
        .collect();
    assert_eq!(
        diag_positions_after, diag_positions_before,
        "each window's diagnostic overlay must keep its position in the \
         composition order; a remove-and-re-push would move it to the end \
         ({before_kinds:?} -> {after_kinds:?})"
    );

    let after_paint = error_underlines_per_window(&state);
    assert_eq!(after_paint.len(), 2, "still two windows: {after_paint:?}");
    for (win, n) in &after_paint {
        assert!(
            *n > 0,
            "window {win} must render the NEW URI's diagnostics; 0 means \
             its overlay is still keyed under the old URI, whose store the \
             forget emptied ({after_paint:?})"
        );
    }
}

/// Acceptance 31c, at the Lua binding. Raises for an unknown server id;
/// **succeeds** for a URI with no state under a known server.
///
/// The second arm is the one that matters: the `resource.renamed`
/// subscriber calls this per attachment, and an attachment need not have
/// any pending route or populated result store. An over-strict binding
/// would turn that ordinary idempotent case into an error inside a hook.
#[test]
fn acc31c_the_forget_uri_binding_raises_for_an_unknown_server_and_not_for_an_unknown_uri() {
    let fx = Fixture::new();
    fx.write("proj/Cargo.toml", "[package]\nname = \"p\"\n");
    let file = fx.write("proj/src/main.rs", "fn main() {}\n");
    let mut state = editor();
    configure_fake(&state, &fx.root, "rust");
    open_as(&state, "B", &file);
    settle_until(&mut state, "one live server", |s| server_count(s) == 1);

    // An unknown id has to be a real handle to a server the manager no
    // longer holds: `LspServerIdLua` is opaque and cannot be forged from
    // an integer, which is itself the binding's first line of defence.
    let raised: String = eval(
        &state,
        "local sid
         for _, row in ipairs(pmacs.lsp.list()) do sid = row.id end
         assert(sid, 'no server to stale out')
         _G.STALE = sid
         pmacs.lsp.stop(sid)
         return 'stopped'",
    );
    assert_eq!(raised, "stopped");
    settle_until(&mut state, "the server is forgotten", |s| {
        let gone: bool = eval(
            s,
            "local ok = pcall(pmacs.lsp.forget, _G.STALE)
             return #pmacs.lsp.list() == 0",
        );
        gone
    });
    let raised: String = eval(
        &state,
        "local ok, err = pcall(pmacs.lsp.forget_uri, _G.STALE, 'file:///nope.rs')
         if ok then return 'DID NOT RAISE' end
         return tostring(err)",
    );
    assert!(
        raised.contains("unknown server"),
        "an unknown server id must raise, matching `pmacs.lsp.forget`; got \
         {raised:?}"
    );

    // And the success arm, against a live server.
    open_as(
        &state,
        "C",
        &fx.write("proj/src/second.rs", "fn second() {}\n"),
    );
    settle_until(&mut state, "a live server again", |s| server_count(s) == 1);
    let ok: bool = eval(
        &state,
        "local sid
         for _, row in ipairs(pmacs.lsp.list()) do sid = row.id end
         local a = pcall(pmacs.lsp.forget_uri, sid, 'file:///never-opened.rs')
         local b = pcall(pmacs.lsp.forget_uri, sid, 'file:///never-opened.rs')
         return a and b",
    );
    assert!(
        ok,
        "a URI with no state under a known server is an idempotent \
         success, and repeating it stays safe"
    );
}

/// Acceptance 32. A rename **across project roots** re-runs
/// `ensure_server` and the buffer ends up attached to a **different**
/// server; a same-root rename reuses the existing one (#161's affinity
/// key is the detected project root).
#[test]
fn acc32_a_cross_root_rename_re_runs_ensure_server_and_a_same_root_one_reuses() {
    // Same root first: renaming within one package must not spawn a
    // second server.
    let fx = Fixture::new();
    fx.write("a/Cargo.toml", "[package]\nname = \"a\"\n");
    let inside = fx.write("a/src/main.rs", "fn main() {}\n");
    let mut state = editor();
    configure_fake(&state, &fx.root, "rust");
    open_as(&state, "B", &inside);
    settle_until(&mut state, "one initialized server", |s| {
        server_rows(s) == vec![format!("rust|{}|initialized", file_uri(&fx.at("a")))]
    });
    let root_a = file_uri(&fx.at("a"));
    assert_eq!(
        server_rows(&state),
        vec![format!("rust|{root_a}|initialized")],
        "precondition: one server, rooted at package a"
    );

    rename_fire_and_forget(&mut state, &inside, &fx.at("a/src/moved.rs"));
    settle_a_while(&mut state);
    assert_eq!(
        server_count(&state),
        1,
        "a same-root rename reuses the existing server: {:?}",
        server_rows(&state)
    );

    // Now across roots: `b` is its own package, so its file needs its
    // own server.
    fx.write("b/Cargo.toml", "[package]\nname = \"b\"\n");
    std::fs::create_dir_all(fx.at("b/src")).unwrap();
    rename_fire_and_forget(
        &mut state,
        &fx.at("a/src/moved.rs"),
        &fx.at("b/src/moved.rs"),
    );
    settle_until(&mut state, "a second server for package b", |s| {
        server_count(s) == 2
    });
    settle_a_while(&mut state);

    let root_b = file_uri(&fx.at("b"));
    let rows = server_rows(&state);
    assert!(
        rows.iter().any(|r| r.contains(&root_b)),
        "the cross-root rename must spawn a server rooted at package b; \
         rows were {rows:?}"
    );
    assert_eq!(
        rows.len(),
        2,
        "exactly one new server, not one per reconciliation pass: {rows:?}"
    );
}

/// Point the `rust` server at the `applyeditplan` fake carrying `plan`,
/// and hand back the sink the client's response to the server-initiated
/// `workspace/applyEdit` lands in. Must run before the first `.rs` file
/// is opened — that open is what launches the server.
fn plan_server(state: &EditorState, dir: &Path, plan: &serde_json::Value) -> PathBuf {
    let plan_path = dir.join("plan.json");
    std::fs::write(&plan_path, serde_json::to_vec(plan).unwrap()).unwrap();
    let sink = dir.join("applyedit-response.json");
    exec(
        state,
        &format!(
            "pmacs.lsp.config.rust = {{
               command = \"{}\",
               env = {{
                 PMACS_FAKE_LSP_MODE = 'applyeditplan',
                 PMACS_FAKE_LSP_EDIT_PLAN = '{}',
                 PMACS_FAKE_LSP_APPLYEDIT_SINK = '{}',
               }},
             }}",
            fake_lsp_path(),
            plan_path.display(),
            sink.display()
        ),
    );
    sink
}

/// Ask the fake to deliver its planned `workspace/applyEdit`. Driven by
/// an `executeCommand` rather than fired at `initialized`, so the test
/// controls *when* the batch arrives — these fixtures depend on a
/// specific buffer being active first, and a server-timed request would
/// race that setup.
fn trigger_apply_edit(state: &EditorState) {
    exec(
        state,
        "local sid
         for _, row in ipairs(pmacs.lsp.list()) do
           if row.state and row.state.kind == 'initialized' then sid = row.id end
         end
         assert(sid, 'no initialized server')
         pmacs.lsp.request_execute_command(sid, 'pmacs.fake.applyEdit', {})",
    );
}

fn wait_for_apply_response(state: &mut EditorState, sink: &Path) -> serde_json::Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        state.tick_processes();
        state.tick_lsp();
        state.tick_async();
        if let Ok(raw) = std::fs::read(sink)
            && let Ok(v) = serde_json::from_slice::<serde_json::Value>(&raw)
        {
            assert!(
                v.get("fakeError").is_none(),
                "the fixture itself failed: {v:?}"
            );
            return v;
        }
        assert!(
            Instant::now() < deadline,
            "the client never answered the server's workspace/applyEdit"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Acceptance 34. Renaming the **active** file through the full
/// `apply_workspace_edit` path returns the user to the **same buffer**
/// (now under its new path), and leaves no buffer bound to the obsolete
/// path.
///
/// The plan deliberately edits *another* file first. Without that the
/// row cannot bite at all: the applier only has to restore the origin if
/// something moved the active buffer away, and a lone rename op does not.
///
/// Bite: the applier restoring by path instead of by buffer handle. A
/// captured path no longer resolves after its own batch renamed it, so
/// `find_or_open` raises, the `pcall` swallows it, and the user is
/// stranded in whatever buffer the last applied op left active. No
/// reconciliation can reach a string a Lua local already captured.
///
/// **One framing claim corrected here.** §5's G1 says the stale path
/// "materializes a phantom": that `find_or_open(origin)` reaches
/// `resolve_target_buffer`'s `NotFound` arm, which creates an empty
/// path-backed buffer and selects it. It does not.
/// `pmacs.buffer.find_or_open` (`src/lua_bindings/mod.rs`) calls
/// `crate::file_io::load_file` directly and maps the error, so a missing
/// path **raises**; the `NotFound` arm belongs to
/// `EditorCore::resolve_target_buffer`, which serves
/// `pmacs.window.display_file` and the startup/daemon target, not this
/// binding. The defect is real but smaller than G1 states — a silently
/// swallowed restore, not a fabricated file — and this row asserts the
/// half that is true. The no-buffer-at-the-old-path assertion is kept as
/// a cheap guard against a future fallback that *would* create one, and
/// is not the biting half.
#[test]
fn acc34_renaming_the_active_file_through_the_applier_returns_the_same_buffer() {
    let fx = Fixture::new();
    fx.write("proj/Cargo.toml", "[package]\nname = \"p\"\n");
    let old = fx.write("proj/src/main.rs", "fn main() {}\n");
    let other = fx.write("proj/src/other.rs", "fn other() {}\n");
    let new = fx.at("proj/src/renamed.rs");
    let mut state = editor();
    configure_fake(&state, &fx.root, "rust");
    let sink = plan_server(
        &state,
        &fx.root,
        &serde_json::json!({
            "documentChanges": [
                {
                    // Moves the active buffer away, so the restore has
                    // something to undo.
                    "textDocument": { "uri": file_uri(&other), "version": 1 },
                    "edits": [{
                        "range": {
                            "start": { "line": 0, "character": 0 },
                            "end":   { "line": 0, "character": 0 },
                        },
                        "newText": "// touched\n",
                    }],
                },
                {
                    "kind": "rename",
                    "oldUri": file_uri(&old),
                    "newUri": file_uri(&new),
                },
            ],
        }),
    );
    open_as(&state, "B", &old);
    settle_until(&mut state, "server initialized", |s| {
        server_rows(s).iter().any(|r| r.ends_with("|initialized"))
    });
    // The applier restores the buffer that was active when the batch
    // began, so make that the file being renamed.
    exec(&state, "pmacs.window.switch_buffer(_G.B)");
    let active_before = state.core.borrow().active_buffer_id();

    trigger_apply_edit(&state);
    let response = wait_for_apply_response(&mut state, &sink);
    assert_eq!(
        response["result"]["applied"], true,
        "the batch must apply: {response:?}"
    );
    settle_a_while(&mut state);

    assert!(new.exists(), "the rename landed on disk");
    assert!(!old.exists());
    assert_eq!(
        state.core.borrow().active_buffer_id(),
        active_before,
        "the user must be returned to the SAME buffer, now under its new \
         path — a path-based restore raises on the renamed-away path, the \
         pcall swallows it, and the user is left wherever the last applied \
         op put them"
    );
    assert_eq!(
        buffer_path(&state, "B").as_deref(),
        Some(new.to_str().unwrap()),
        "and that buffer's path followed the rename"
    );

    let stale: bool = eval(
        &state,
        &format!(
            "for _, b in ipairs(pmacs.buffer.list()) do
               if b:path() == \"{}\" then return true end
             end
             return false",
            lua_str(&old)
        ),
    );
    assert!(!stale, "no buffer may remain bound to the obsolete path");
}

/// Acceptance 35. When the origin buffer is **gone** after the edit, the
/// applier restores **nothing** rather than falling back to the old
/// path.
#[test]
fn acc35_when_the_origin_buffer_is_gone_the_applier_restores_nothing() {
    let fx = Fixture::new();
    fx.write("proj/Cargo.toml", "[package]\nname = \"p\"\n");
    let doomed = fx.write("proj/src/main.rs", "fn main() {}\n");
    let other = fx.write("proj/src/other.rs", "fn other() {}\n");
    let mut state = editor();
    configure_fake(&state, &fx.root, "rust");
    let sink = plan_server(
        &state,
        &fx.root,
        &serde_json::json!({
            "documentChanges": [{
                "kind": "delete",
                "uri": file_uri(&doomed),
            }],
        }),
    );
    // `other` keeps the registry non-empty so the delete's kill is not
    // refused for being the last buffer.
    open_as(&state, "OTHER", &other);
    open_as(&state, "B", &doomed);
    settle_until(&mut state, "server initialized", |s| {
        server_rows(s).iter().any(|r| r.ends_with("|initialized"))
    });
    exec(&state, "pmacs.window.switch_buffer(_G.B)");

    trigger_apply_edit(&state);
    let response = wait_for_apply_response(&mut state, &sink);
    assert_eq!(
        response["result"]["applied"], true,
        "the batch must apply: {response:?}"
    );
    settle_a_while(&mut state);

    assert!(!doomed.exists(), "the file is gone");
    assert!(
        !buffer_is_valid(&state, "B"),
        "and its clean buffer was reconciled away"
    );
    let phantom: bool = eval(
        &state,
        &format!(
            "for _, b in ipairs(pmacs.buffer.list()) do
               if b:path() == \"{}\" then return true end
             end
             return false",
            lua_str(&doomed)
        ),
    );
    assert!(
        !phantom,
        "the applier must restore NOTHING rather than re-opening the path \
         it just deleted — a path fallback would recreate it as an empty \
         buffer, and the next C-x C-s would resurrect the file"
    );
    let active_valid = {
        let core = state.core.borrow();
        let id = core.active_buffer_id();
        core.registry.borrow().contains(id)
    };
    assert!(
        active_valid,
        "and the window it left behind must sit on a live buffer"
    );
}
