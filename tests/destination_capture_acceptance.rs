// tests/destination_capture_acceptance.rs --- the Lua-reachable capture.

//! Acceptance for `docs/destination-capture-framing.md` §7: a
//! destination any asynchronous continuation can capture, and the
//! profile that says which of `commit_to`'s preconditions it depends on
//! (Q#DC-1 … Q#DC-5).
//!
//! **What this suite does NOT prove**, deliberately: that git — or any
//! other adopter — surfaces in the right frontend. This lane ships the
//! mechanism and the tests for the mechanism; adoption is #227's, after
//! it lands (§8). Every test here therefore drives the Lua surface
//! directly rather than through a consumer.
//!
//! Three disciplines it keeps:
//!
//! * **Every "not applicable" cell in Q#DC-2's preflight matrix is
//!   asserted as NOT refusing**, not merely left untested. A check
//!   deliberately omitted and a check someone forgot look identical from
//!   the outside, and the next reader restores the second one.
//! * **A refusal is asserted on its reason**, never on the mere fact
//!   that something failed. `commit_to` has five distinct refusals and a
//!   raise; "it errored" would pass on any of the wrong ones.
//! * **The panel profile's relaxation is pinned as a preflight PLUS the
//!   refusal that keeps it true** (revision 8). The preflight measures
//!   whether this frontend places side requests in the panel; the body is
//!   arbitrary *synchronous* Lua, so refusing `await` — which only stops
//!   another coroutine interleaving — does not stop it invalidating that
//!   measurement. The answer is neither to predict the body nor to catch
//!   it late at placement (by then it has created buffers, handles and
//!   paint, which is "four mutations too late" all over again) but to
//!   **refuse the mutation at the attempt**, exactly as `await` is
//!   refused. Three tests carry it and none subsumes another:
//!   `a_panel_commit_that_falls_back_runs_the_document_preflight` (the
//!   body must not run at all when the fallback already holds),
//!   `a_body_that_tries_to_create_the_fallback_is_refused_at_the_attempt`
//!   (the mutation is refused, and nothing partial is left behind), and
//!   `a_panel_commit_that_falls_back_with_a_valid_destination_still_lands`
//!   (falling back is still graceful degradation, not an error).
//!
//! `tests/journey_acceptance.rs` and `tests/dired_acceptance.rs` are the
//! preservation half of the same §7 and are run alongside this suite:
//! they hold the Stage 1a contract this lane generalizes, and if either
//! needed editing the generalization changed Journey semantics rather
//! than extending them.

use pmacs::buffer::BufferId;
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;
use pmacs::window::{FrontendView, Layout, Window, WindowId};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

/// A fresh editor with LSP disabled: no test here asserts anything about
/// a language server, so the wipe cannot make an assertion vacuous.
fn editor() -> EditorState {
    let s = EditorState::new_with_roots(&crate::iso::roots());
    exec(&s, "pmacs.lsp.config = {}");
    s
}

/// A directory with a file worth displaying.
fn project() -> TempDir {
    let td = tempfile::tempdir().expect("tempdir");
    std::fs::write(td.path().join("alpha.txt"), b"alpha\n").expect("write alpha");
    td
}

fn active_name(s: &EditorState) -> String {
    eval(s, "return pmacs.window.buffer():name()")
}

fn buffer_in(s: &EditorState, window: WindowId) -> Option<BufferId> {
    s.core.borrow().windows.get(&window).map(|w| w.buffer_id)
}

/// A window's buffer **by name**, so a placement assertion reads as
/// "`*result*` went to the panel" rather than as two opaque ids.
fn name_in(s: &EditorState, window: WindowId) -> String {
    let buffer = buffer_in(s, window).expect("window is live");
    let core = s.core.borrow();
    let registry = core.registry.borrow();
    registry.get(buffer).expect("buffer").name().to_string()
}

/// Whether `window` is pinned to its buffer (Q#BP2c `dedicated`).
fn dedicated(s: &EditorState, window: WindowId) -> bool {
    s.core
        .borrow()
        .windows
        .get(&window)
        .is_some_and(|w| w.params.dedicated)
}

/// Whether a buffer by this name exists at all.
///
/// The "nothing partial was installed" assertion needs to see a side
/// effect the body would have left *before* reaching any display, and a
/// created-but-never-shown buffer is exactly that.
fn buffer_exists(s: &EditorState, name: &str) -> bool {
    eval(
        s,
        &format!(
            "for _, id in ipairs(pmacs.buffer.list()) do
               if pmacs.describe.buffer(id).name == {name:?} then return true end
             end
             return false"
        ),
    )
}

fn local_window(s: &EditorState) -> WindowId {
    s.core
        .borrow()
        .views
        .get(&FrontendId::LOCAL)
        .expect("LOCAL view")
        .active
}

/// The frontend that competes for ambient authority.
const COMPETITOR: FrontendId = FrontendId(7);

/// A frontend that has a layout but no live document window (Q#DC-4).
const DOCUMENTLESS: FrontendId = FrontendId(9);

/// Register a second frontend with its own single-window layout,
/// mirroring `build_fresh_frontend_view` — the same helper shape
/// `journey_acceptance` and `bottom_panel_stage1_acceptance` use.
fn attach_frontend(s: &EditorState, fid: FrontendId) -> WindowId {
    let win = WindowId::next();
    let mut core = s.core.borrow_mut();
    let buffer_id = core.active_buffer_id();
    let text_view = {
        let reg = core.registry.borrow();
        pmacs::text_view::TextView::new(reg.get(buffer_id).expect("buffer"))
    };
    core.windows
        .insert(win, Window::new(win, buffer_id, text_view));
    core.register_frontend_view(fid, view_over(win));
    win
}

/// Register a frontend whose layout names a window that is **not live**,
/// so `primary_document_window` finds nothing to hand back.
///
/// **Why this shape and not a side-window-only layout.** The obvious
/// reading of "a frontend with no document window" is a frontend showing
/// only a bottom panel — but that state is asserted impossible: Q#BP6
/// says a layout always retains at least one non-side window, and
/// `EditorCore::non_side_target` carries a `debug_assert!` that fires
/// under `cargo test` if one ever does. So the reachable spelling of the
/// same condition is a layout whose document window has gone while the
/// view remains, which is what this builds.
///
/// **Recorded honestly, because the framing implies more than the tree
/// does** (`docs/destination-capture-framing.md` Q#DC-4): with Q#BP6
/// held, a *registered* frontend in a healthy editor always has a live
/// document window, so the absent document pair is a **defensive**
/// branch rather than a routine one. It is still the right decision —
/// capture stays total, and an adopter with nowhere to land gets a
/// refusal naming that rather than permission to fall back to ambient
/// state — and it is still worth pinning, because the alternative to
/// pinning it is a branch nothing ever executes.
fn attach_documentless_frontend(s: &EditorState, fid: FrontendId) {
    let mut core = s.core.borrow_mut();
    core.register_frontend_view(fid, view_over(WindowId::next()));
}

fn view_over(win: WindowId) -> FrontendView {
    FrontendView {
        layout: Layout::single(win),
        active: win,
        fold_projection: true,
        panel_capable: true,
        frame_geometry: None,
        panel_hidden: false,
    }
}

/// Capture through the **production** Lua entry point and leave the
/// userdata in the global `dest`.
///
/// Nothing in this suite can construct one by any other route — that is
/// what `a_forged_destination_is_still_refused` is about — so every test
/// below runs against a destination the editor minted.
fn capture(s: &EditorState) {
    exec(s, "dest = pmacs.window.capture_destination()");
    assert!(
        eval::<bool>(s, "return dest ~= nil"),
        "the capture must always yield a destination while a frontend exists"
    );
}

/// Run a body that also executes `also` under `profile`, reporting
/// `(ok, reason)`.
///
/// `profile` is spliced as a Lua expression, so a caller can pass
/// `"nil"`, `"'panel'"`, `"42"` — the argument-shape distinctions
/// Q#DC-5 turns on are exactly what this suite has to vary. `also` is
/// spliced as Lua statements, for the rows that must observe *where* an
/// accepted commit put its result and not merely that it was accepted.
fn commit_body(s: &EditorState, profile: Option<&str>, also: &str) {
    let call = match profile {
        Some(profile) => format!("pmacs.window.commit_to(dest, body, {profile})"),
        None => "pmacs.window.commit_to(dest, body)".to_string(),
    };
    exec(
        s,
        &format!(
            "ran = false
             local body = function() ran = true; {also} end
             raised = nil
             local caught, a, b = pcall(function() return {call} end)
             if caught then ok, reason = a, b
             else ok, reason, raised = false, nil, tostring(a) end"
        ),
    );
}

/// Run an inert body under `profile` and report `(ok, reason)`.
fn commit(s: &EditorState, profile: Option<&str>) {
    commit_body(s, profile, "");
}

fn ok(s: &EditorState) -> bool {
    eval(s, "return ok == true")
}

fn ran(s: &EditorState) -> bool {
    eval(s, "return ran")
}

fn reason(s: &EditorState) -> String {
    eval(s, "return tostring(reason)")
}

/// The message a raise (as opposed to a `(false, reason)` refusal)
/// carried, or `None` if nothing was raised.
fn raised(s: &EditorState) -> Option<String> {
    eval::<Option<String>>(s, "return raised")
}

// ---------------------------------------------------------------------------
// §7 — a captured destination survives a frontend switch
// ---------------------------------------------------------------------------

/// **N** — the failure the lane exists for: the result lands in the
/// frontend that *asked*, not in whichever one is ambient when the work
/// settles.
///
/// Asserted for **both** profiles. The panel profile drops three of the
/// four preflight checks, and a plausible way to implement that is to
/// drop the scope with them — which would leave a panel continuation
/// resolving its target from ambient state, the exact P1a defect. So the
/// scope is pinned per profile rather than once.
///
/// Falsified by making the commit display ambiently: the file then
/// appears in the competitor's window. Asserting merely that
/// `capture_destination()` returns userdata would pass on a capture that
/// does nothing.
#[test]
fn a_captured_destination_survives_a_frontend_switch() {
    for profile in [None, Some("'panel'")] {
        let td = project();
        let s = editor();
        capture(&s);

        let local_win = local_window(&s);
        let other_win = attach_frontend(&s, COMPETITOR);
        let other_before = buffer_in(&s, other_win);

        // The competitor becomes the dispatching frontend while the work
        // is "in flight" — the state a worker completion returns to.
        s.core.borrow_mut().active_frontend = COMPETITOR;

        let alpha = td.path().join("alpha.txt").display().to_string();
        exec(
            &s,
            &format!(
                "committed = pmacs.window.commit_to(dest, function()
                   pmacs.window.display_file({alpha:?})
                 end{})",
                profile.map_or(String::new(), |p| format!(", {p}"))
            ),
        );

        assert!(
            eval::<bool>(&s, "return committed"),
            "{profile:?}: the commit must be accepted"
        );
        assert_eq!(
            buffer_in(&s, other_win),
            other_before,
            "{profile:?}: the competing frontend's window must be untouched"
        );
        s.core.borrow_mut().active_frontend = FrontendId::LOCAL;
        assert_eq!(
            active_name(&s),
            alpha,
            "{profile:?}: the commit must land in the capturing frontend's window"
        );
        assert_eq!(
            local_window(&s),
            local_win,
            "{profile:?}: and in that window, not a new one"
        );
    }
}

// ---------------------------------------------------------------------------
// §7 — the forged destination stays refused
// ---------------------------------------------------------------------------

/// **P (Q#JR14d)** — generalizing the capture does not widen what
/// extension code can fabricate.
///
/// A plausible `{frontend, window, buffer}` table is what any Lua could
/// build, and the capture now hands out the *same* userdata type through
/// a public entry point — so the type check is re-asserted after the
/// rename rather than assumed to have survived it.
///
/// *Mutation:* accept `mlua::Value::Table` in the borrow arm. This
/// fails; nothing in `journey_acceptance` covers the new entry point.
#[test]
fn a_forged_destination_is_still_refused() {
    let s = editor();
    capture(&s);
    let win = eval::<i64>(&s, "return dest:window()");

    exec(
        &s,
        &format!(
            "ran = false
             local caught, err = pcall(pmacs.window.commit_to,
               {{ frontend = 0, window = {win}, buffer = 0 }},
               function() ran = true end)
             rejected = (not caught) and tostring(err) or '<accepted>'"
        ),
    );

    let rejected: String = eval(&s, "return rejected");
    assert!(
        rejected.contains("cannot be constructed from Lua"),
        "a forged table must be rejected by type, not merely fail later; got {rejected:?}"
    );
    assert!(
        !ran(&s),
        "a rejected destination must not reach the callback"
    );
}

// ---------------------------------------------------------------------------
// §7 — the preflight matrix, in BOTH profiles (Q#DC-2)
// ---------------------------------------------------------------------------

/// **N** — each of the four preconditions refuses under the document
/// profile, and each of the three the panel profile omits does **not**
/// refuse under it.
///
/// This is the substance of Q#DC-2. The matrix:
///
/// | # | precondition | document | panel |
/// |---|--------------|----------|-------|
/// | 1 | frontend has a layout | required | **required** |
/// | 2 | window still live | required | not applicable |
/// | 3 | window still shows the captured buffer | required | not applicable |
/// | 4 | window is not dedicated | required | not applicable |
///
/// The panel column is the half that could not be written before this
/// lane, and the half most at risk of being "fixed" later by someone who
/// reads an omission as an oversight — a panel result does not occupy
/// the captured document window, does not replace its buffer, and does
/// not need it to exist, so each of checks 2–4 would refuse `git.status`
/// for a document-window change unrelated to where the panel goes.
///
/// Table-driven so the failure message names *which* cell regressed,
/// which eight near-identical tests would give up in exchange for
/// nothing.
///
/// *Mutation:* apply all four checks in both profiles — the three panel
/// rows fail. *Second mutation:* apply only check 1 in both profiles —
/// the three document rows fail.
#[test]
fn the_preflight_matrix_holds_in_both_profiles() {
    // (label, Lua that breaks the precondition, reason fragment,
    //  whether the PANEL profile refuses too)
    let cases: [(&str, &str, &str, bool); 4] = [
        (
            "frontend gone",
            // Handled in Rust below: unregistering a view has no Lua surface.
            "",
            "requesting frontend is gone",
            true,
        ),
        (
            "window gone",
            "local doomed = dest:window()
             pmacs.window.split_horizontal()
             while pmacs.window.current() == doomed do pmacs.window.focus_next() end
             pmacs.window.close_others()",
            "is gone",
            false,
        ),
        (
            "stale buffer",
            "pmacs.window.switch_buffer(pmacs.buffer.create('*usurper*'))",
            "now shows another buffer",
            false,
        ),
        (
            "dedicated",
            "pmacs.window.set_params(dest:window(), { dedicated = true })",
            "is dedicated",
            false,
        ),
    ];

    for (label, break_it, expected, panel_refuses) in cases {
        for profile in [None, Some("'panel'")] {
            let s = editor();
            capture(&s);

            if label == "frontend gone" {
                s.core
                    .borrow_mut()
                    .unregister_frontend_view(FrontendId::LOCAL);
            } else {
                exec(&s, break_it);
            }

            commit(&s, profile);
            assert_eq!(
                raised(&s),
                None,
                "{label}/{profile:?}: a precondition is a refusal, not a raise"
            );

            let refuses = profile.is_none() || panel_refuses;
            if refuses {
                assert!(!ok(&s), "{label}/{profile:?}: commit_to must refuse");
                assert!(
                    reason(&s).contains(expected),
                    "{label}/{profile:?}: reason must say why; wanted {expected:?}, got {:?}",
                    reason(&s)
                );
                assert!(
                    !ran(&s),
                    "{label}/{profile:?}: the callback must not run at all -- validating \
                     after it is four mutations too late"
                );
            } else {
                assert!(
                    ok(&s),
                    "{label}/panel: this check is DELIBERATELY omitted for a panel \
                     result, which touches no document window; got refusal {:?}",
                    reason(&s)
                );
                assert!(ran(&s), "{label}/panel: the callback must run");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// §7 — the panel profile's relaxation is CONDITIONAL (Q#DC-2, rev 6–9)
// ---------------------------------------------------------------------------

/// The Lua a `"panel"` continuation runs: put a result buffer in the
/// bottom panel. It is the shape `listview.open` resolves to by default
/// (`builtin/runtime/listview.lua`), and the shape git's `*git-status*`
/// adoption will take.
const PANEL_BODY: &str = "pmacs.window.display(pmacs.buffer.create('*result*'), \
                          { side = 'bottom' })";

/// A reusable panel: present and **undedicated**, so the preflight
/// measures "this frontend places side requests in the panel" and the
/// relaxation applies. Every mutation row starts from here except the
/// one whose whole point is that no panel exists yet.
const PANEL_ARRANGED: &str = "pmacs.window.display(pmacs.buffer.create('*pinned*'), \
                              { side = 'bottom', dedicated = false, select = false })";

/// Every route by which a `commit_to` body can reach a write to a **side**
/// window's `Window::params.dedicated`, as `(label, arrangement before the
/// capture, the attempted mutation)`.
///
/// **One row per WRITE SITE, not per call spelling** (§3's enumeration). A
/// single row is exactly what would let a second route keep the defect —
/// which is not hypothetical: review found the `display{side, dedicated}`
/// route *after* `set_params` was specified, and one spelling of it
/// reaches three different writes.
///
/// | row | reaches |
/// |---|---|
/// | `set_params` | the direct write in the binding (Q#BP2c) |
/// | `display{side, dedicated}` replacing | `apply_placement`'s **replacing** arm |
/// | `display{side, dedicated}` same buffer | its **non-replacing** arm |
/// | `display{side, dedicated}` with no panel | its **created** arm |
///
/// The three `display` rows converge on one guard, in `display_buffer` —
/// `apply_placement` has exactly one caller, so every request-driven
/// dedication passes through it. They are still separate rows because that
/// convergence is a property of today's call graph, and a row per arm
/// fails loudly if it stops holding.
///
/// Shared by the two tests that drive them, at commit depth 1 and through
/// a nested commit: a route guarded at one depth and not the other is the
/// defect revision 9 fixes, and a table each would let the two drift.
const DEDICATION_ROUTES: [(&str, &str, &str); 4] = [
    (
        "set_params",
        PANEL_ARRANGED,
        "pmacs.window.set_params(pmacs.window.panel(), { dedicated = true })",
    ),
    (
        "display{side, dedicated} replacing",
        PANEL_ARRANGED,
        "pmacs.window.display(pmacs.buffer.create('*usurp*'),
           { side = 'bottom', dedicated = true, select = false })",
    ),
    (
        // The same buffer the panel already shows: `replacing` is false,
        // so this lands in a DIFFERENT arm of the same function, which a
        // row against the replacing arm alone would not exercise.
        "display{side, dedicated} same buffer",
        PANEL_ARRANGED,
        "pmacs.window.display(pmacs.window.buffer(pmacs.window.panel()),
           { side = 'bottom', dedicated = true, select = false })",
    ),
    (
        // NO panel at capture time: the preflight relaxes because
        // `side_window_for` is None (a side request would CREATE a panel,
        // never fall back). The body then creates one dedicated, which
        // makes the next side request fall back.
        "display{side, dedicated} creating the panel",
        "",
        "pmacs.window.display(pmacs.buffer.create('*usurp*'),
           { side = 'bottom', dedicated = true, select = false })",
    ),
];

/// Arrange one of the two reasons a side request falls back into a
/// document window, and assert the arrangement took.
///
/// The two arms are independent branches of
/// `EditorCore::resolve_placement`, so a fix that handled only one would
/// leave the other live. Every fallback test below drives both.
fn arrange_fallback(s: &EditorState, cause: &str) {
    if cause == "not panel-capable" {
        // Q#BP13's capability gate: `side` is honoured only on a
        // panel-capable frontend.
        s.core
            .borrow_mut()
            .views
            .get_mut(&FrontendId::LOCAL)
            .expect("LOCAL view")
            .panel_capable = false;
    } else {
        // Q#BP3 2.iii: the one side slot is dedicated to another buffer,
        // and a second panel is never created.
        exec(
            s,
            "pmacs.window.display(pmacs.buffer.create('*pinned*'),
               { side = 'bottom', dedicated = true, select = false })",
        );
        assert!(
            s.core.borrow().side_window_for(FrontendId::LOCAL).is_some(),
            "{cause}: the arrangement must actually create the side slot"
        );
    }
}

/// **N** — a `"panel"` commit whose placement *already* falls back is
/// refused **before its body runs**, on the stale-intent reason.
///
/// The defect: the panel column dropped checks 2–4 on the claim that a
/// panel result never touches a document window — but panel placement
/// falls back to an ordinary document window and then *installs the
/// result there* (`EditorCore::apply_placement` says so in its own
/// comment). The relaxation therefore handed a `"panel"` commit
/// permission to overwrite a document view with no stale-intent guard:
/// capture A, the user opens B, the continuation lands and B is gone.
///
/// **This is the EARLY half, not the guarantee.** It is served by
/// `EditorCore::commit_destination_refusal` consulting
/// `panel_placement_can_fall_back`, which can only read the state that
/// holds *now*. The reason that is worth having anyway is the same reason
/// `commit_to` preflights at all: a body allocates a buffer, registers a
/// handle and paints long before it reaches any call that could refuse,
/// so refusing here leaves no debris. A frontend that cannot render a
/// panel will not acquire the capability mid-body, which is exactly the
/// case this catches.
///
/// The guarantee — for the case a snapshot **cannot** catch, where the
/// body creates the fallback itself — is
/// `a_panel_commit_whose_body_creates_the_fallback_is_refused_at_placement`.
/// Neither test subsumes the other: this one pins that nothing runs, that
/// one pins that nothing lands.
///
/// Each row asserts four things: the commit **refuses**, it refuses for
/// the stale-intent reason (not incidentally), the body never ran, and
/// the newer buffer is still there.
///
/// *Mutation:* delete the `panel_placement_can_fall_back` arm from
/// `commit_destination_refusal`. Both rows fail — the body runs, and the
/// placement backstop then refuses as a *raise*, so `ok`/`ran`/`reason`
/// all move.
#[test]
fn a_panel_commit_that_falls_back_runs_the_document_preflight() {
    for cause in ["not panel-capable", "side slot dedicated elsewhere"] {
        let s = editor();

        // Arrange the fallback cause BEFORE capturing, so the preflight
        // can see it — which is exactly what distinguishes this test from
        // the body-induced one below.
        arrange_fallback(&s, cause);

        capture(&s);
        let doc = local_window(&s);
        assert_eq!(
            eval::<Option<u64>>(&s, "return dest:window()"),
            Some(doc.raw()),
            "{cause}: the capture must name the document window, not the panel"
        );

        // The user replaces the captured buffer while the work is in
        // flight: `*newer*` is newer information than the request.
        exec(
            &s,
            "pmacs.window.switch_buffer(pmacs.buffer.create('*newer*'))",
        );
        assert_eq!(
            name_in(&s, doc),
            "*newer*",
            "{cause}: the arrangement must make the captured window stale"
        );

        commit_body(&s, Some("'panel'"), PANEL_BODY);

        assert_eq!(
            raised(&s),
            None,
            "{cause}: a precondition is a refusal, not a raise"
        );
        assert!(
            !ok(&s),
            "{cause}: a \"panel\" commit that lands in a DOCUMENT window must run the \
             document preflight -- the relaxation is conditional on the placement really \
             being a panel"
        );
        assert!(
            reason(&s).contains("now shows another buffer"),
            "{cause}: and refuse on stale intent; got {:?}",
            reason(&s)
        );
        assert!(!ran(&s), "{cause}: the callback must not run");
        assert_eq!(
            name_in(&s, doc),
            "*newer*",
            "{cause}: the user's newer buffer must survive -- this is the assertion that \
             fails loudest when the guard is removed"
        );
    }
}

/// **N** — a body that tries to **create** the fallback is refused **at
/// the attempt**, and the refusal lands on the mutation rather than on
/// the outcome.
///
/// This is the case no preflight snapshot can catch, and the two rows
/// above cannot reach it: both establish their fallback state *before*
/// `commit_to` is entered. The body is arbitrary **synchronous** Lua, so
/// refusing `await` — which stops another coroutine interleaving —
/// places no restriction on it:
///
/// ```lua
/// pmacs.window.set_params(pmacs.window.panel(), { dedicated = true })
/// pmacs.window.display(result, { side = "bottom" })
/// ```
///
/// Two statements: the first invalidates the preflight, the second cashes
/// it in. The arrangement is deliberately the **inverse** of the rows
/// above — the preflight says "this lands in the panel", the relaxation
/// applies, and the body runs.
///
/// **Asserting only "document B was not replaced" is insufficient**, and
/// an earlier version of this test made exactly that mistake: it passes
/// on a design that lets the body mutate freely and merely declines the
/// final installation, leaving every other side effect behind. So the
/// three assertions that matter are that the **dedication call itself is
/// refused**, the slot is **still undedicated afterwards**, and **nothing
/// partial was installed**.
///
/// # One row per WRITE SITE, not per call spelling
///
/// The rows are `DEDICATION_ROUTES`, which documents why it is a write-site
/// enumeration rather than a list of call spellings.
///
/// The rest of the enumeration is **unreachable rather than refused**
/// and is recorded in `EditorCore::panel_commit_dedication_refusal`,
/// because a test cannot express it: `panel_capable` has no Lua binding;
/// **losing** the side window is not a fallback route at all
/// (`resolve_placement` creates a fresh panel instead); and `quit`
/// restoring a `dedicated: true` presentation cannot be constructed,
/// since `QuitAction::Restore` only captures that flag on a *replacing*
/// side placement and a dedicated slot can never be the target of one.
///
/// *Mutation:* delete the `panel_commit_dedication_refusal` call from
/// either guarded site — `set_params` drops row 1, `display_buffer`
/// drops rows 2–4 — and every other test in this file still passes.
#[test]
fn a_body_that_tries_to_create_the_fallback_is_refused_at_the_attempt() {
    for (label, arrange, attempt) in DEDICATION_ROUTES {
        let s = editor();
        exec(&s, arrange);

        let panel_before = s.core.borrow().side_window_for(FrontendId::LOCAL);
        if let Some(panel) = panel_before {
            assert!(
                !dedicated(&s, panel),
                "{label}: the slot must start UNDEDICATED, or the preflight would have \
                 refused and this row would be re-proving the preflight"
            );
        }
        let panel_buffer_before = panel_before.map(|panel| name_in(&s, panel));

        capture(&s);
        let doc = local_window(&s);
        exec(
            &s,
            "pmacs.window.switch_buffer(pmacs.buffer.create('*newer*'))",
        );

        commit_body(&s, Some("'panel'"), &format!("{attempt}\n{PANEL_BODY}"));

        assert!(
            ran(&s),
            "{label}: the body must have run -- the preflight could not have known"
        );

        // 1. THE MUTATION ITSELF IS REFUSED, on content.
        let raised = raised(&s).unwrap_or_else(|| {
            panic!("{label}: the attempted mutation must be refused, not merely declined later")
        });
        assert!(
            raised.contains("cannot dedicate the side window"),
            "{label}: the refusal must name the operation it is refusing; got {raised:?}"
        );
        assert!(
            raised.contains("\"panel\" commit_to"),
            "{label}: and why it is refused here specifically; got {raised:?}"
        );

        // 2. THE SLOT IS STILL UNDEDICATED -- including the row where
        //    the slot would have been created dedicated, which must
        //    leave no slot at all rather than an undedicated one.
        let panel_after = s.core.borrow().side_window_for(FrontendId::LOCAL);
        assert_eq!(
            panel_after, panel_before,
            "{label}: a refused mutation must not have created or removed the side slot"
        );
        if let Some(panel) = panel_after {
            assert!(
                !dedicated(&s, panel),
                "{label}: a refused mutation must not have happened -- the whole design \
                 rests on the preflight's measurement still being true afterwards"
            );
        }

        // 3. NOTHING PARTIAL WAS INSTALLED.
        if let (Some(panel), Some(before)) = (panel_after, panel_buffer_before.as_ref()) {
            assert_eq!(
                &name_in(&s, panel),
                before,
                "{label}: the panel must still show what it showed"
            );
        }
        assert_eq!(
            name_in(&s, doc),
            "*newer*",
            "{label}: and the user's newer buffer must survive"
        );
        assert!(
            !buffer_exists(&s, "*result*"),
            "{label}: the refusal must land BEFORE the body's own display -- a `*result*` \
             buffer means the commit got partway and then stopped"
        );
        assert!(
            !buffer_exists(&s, "*usurp*") || panel_after == panel_before,
            "{label}: no usurping presentation may have been installed"
        );
    }
}

/// **N** — a **nested** `commit_to` cannot mask the restriction an
/// enclosing `"panel"` commit is relying on (revision 9).
///
/// # The defect
///
/// Revision 8 held **one** contract on the core, and entering a commit
/// *replaced* it for the inner body's extent, restoring it afterwards
/// (`ScopedFrontend::enter`). So the guarantee above had a hole exactly
/// one call wide:
///
/// ```lua
/// pmacs.window.commit_to(outer, function()          -- "panel": relaxed preflight
///   pmacs.window.commit_to(inner, function()        -- "document": MASKS the outer contract
///     pmacs.window.set_params(pmacs.window.panel(), { dedicated = true })
///   end)                                            -- ...and succeeds
///   pmacs.window.display(result, { side = "bottom" })
/// end, "panel")                                     -- ...which now FALLS BACK
/// ```
///
/// Every step is legal on its own. The outer commit's relaxed preflight
/// was granted because this frontend places side requests in the panel;
/// the nested commit put the refusal that keeps that true out of force;
/// and the outer commit then resumed and overwrote the user's newer
/// document buffer — the original P1a failure, reached through one extra
/// call rather than through a route the write-site enumeration missed.
///
/// **What this invalidated, precisely.** Not §3's enumeration of
/// dedication write sites: all four rows below are the same writes, and
/// each is still guarded. What was wrong was the claim that the guard was
/// **in force for the whole outer body**. So the fix composes contracts
/// instead of replacing them — the strictest active restriction wins —
/// and the enumeration is inherited unchanged.
///
/// **A late refusal would not have been a fix**, and revision 7 was
/// already rejected for being one: by the time the outer commit resumes,
/// the nested callback has already dedicated the slot. The dedication has
/// to be *prevented*, which is why this asserts on the nested attempt and
/// on the slot's state, not merely on where the outer result landed.
///
/// # Why the rows are the same four
///
/// A fix that reinstated the outer contract for only one write site would
/// pass a single-row version of this. `DEDICATION_ROUTES` therefore drives
/// both depths, so a route guarded at depth 1 and not through a nested
/// scope fails loudly.
///
/// *Mutation:* restore `push_commit_contract`/`exit_commit_contract` to a
/// single swapped slot (revision 8's `enter_commit_contract`) and only
/// this test fails.
#[test]
fn a_nested_commit_cannot_mask_an_outer_panel_restriction() {
    for (label, arrange, attempt) in DEDICATION_ROUTES {
        let s = editor();
        exec(&s, arrange);

        let panel_before = s.core.borrow().side_window_for(FrontendId::LOCAL);
        if let Some(panel) = panel_before {
            assert!(
                !dedicated(&s, panel),
                "{label}: the slot must start UNDEDICATED, or the outer preflight would \
                 have refused and this row would be re-proving the preflight"
            );
        }

        capture(&s);
        let doc = local_window(&s);
        // The user's newer buffer: what the outer commit overwrites if its
        // side request is made to fall back.
        exec(
            &s,
            "pmacs.window.switch_buffer(pmacs.buffer.create('*newer*'))",
        );

        // The nested commit is a plain, valid, DOCUMENT-profile commit —
        // its destination is captured fresh inside the outer body, so it
        // passes all four checks on its own account and its callback
        // really runs. Nothing about it is malformed; that is the point.
        commit_body(
            &s,
            Some("'panel'"),
            &format!(
                "local inner = pmacs.window.capture_destination()
                 nested_ran = false
                 local caught, a = pcall(pmacs.window.commit_to, inner, function()
                   nested_ran = true
                   {attempt}
                 end)
                 nested_raised = (not caught) and tostring(a) or nil
                 {PANEL_BODY}"
            ),
        );

        assert!(
            ran(&s),
            "{label}: the outer body must have run -- its preflight could not have known"
        );
        assert!(
            eval::<bool>(&s, "return nested_ran"),
            "{label}: the nested callback must have run -- a nested commit refused at its \
             own preflight would prove nothing about masking"
        );

        // 1. THE MUTATION IS STILL REFUSED, inside the nested scope.
        let nested_raised: Option<String> = eval(&s, "return nested_raised");
        let nested_raised = nested_raised.unwrap_or_else(|| {
            panic!(
                "{label}: the enclosing \"panel\" restriction must survive the nested \
                 commit -- masking it is revision 9's defect"
            )
        });
        assert!(
            nested_raised.contains("cannot dedicate the side window"),
            "{label}: the refusal must name the operation it is refusing; got \
             {nested_raised:?}"
        );
        assert!(
            nested_raised.contains("\"panel\" commit_to"),
            "{label}: and why it is refused here specifically; got {nested_raised:?}"
        );

        // 2. THE SLOT IS STILL UNDEDICATED. Prevention, not detection:
        //    the outer commit resumes after the nested one returns, so a
        //    refusal that arrived then would already be too late.
        let panel_after = s.core.borrow().side_window_for(FrontendId::LOCAL);
        let panel_after = panel_after.unwrap_or_else(|| {
            panic!("{label}: the outer body's own side display must have found a panel")
        });
        assert!(
            !dedicated(&s, panel_after),
            "{label}: a refused mutation must not have happened -- the outer commit's \
             relaxed preflight rests on the slot still being free"
        );
        if let Some(before) = panel_before {
            assert_eq!(
                panel_after, before,
                "{label}: the refusal must not have replaced the side slot"
            );
        }

        // 3. THE OUTER COMMIT'S DESTINATION IS INTACT: its result went to
        //    the PANEL, and the user's newer document buffer survived.
        //    This is the assertion that fails loudest on the unfixed
        //    tree — the outer side request falls back and `*result*`
        //    lands on top of `*newer*`.
        assert!(
            ok(&s),
            "{label}: the outer commit must still be accepted; got {:?}",
            reason(&s)
        );
        assert_eq!(raised(&s), None, "{label}: the outer commit must not raise");
        assert_eq!(
            name_in(&s, panel_after),
            "*result*",
            "{label}: the outer \"panel\" commit's result belongs in the panel"
        );
        assert_eq!(
            name_in(&s, doc),
            "*newer*",
            "{label}: and the user's newer buffer must survive"
        );
    }
}

/// **P** — nesting itself is **not** forbidden: a nested `commit_to` that
/// touches no dedication runs, returns its value, and leaves the enclosing
/// restriction exactly as it found it.
///
/// The other acceptable shape for revision 9's fix was to refuse a nested
/// `commit_to` outright. That closes the hole by forbidding a construction
/// no rule objects to — `commit_to` is public Lua API whose whole purpose
/// is to let a continuation say where its result belongs, and a body that
/// commits to a *second* destination (a diff beside a status panel, say)
/// is the shape #227's adoption is heading for. Only the **restriction**
/// needed preserving, so only the mutation is refused.
///
/// Three things are pinned, and the third is the one a `Vec::pop`-shaped
/// fix would get wrong:
///
/// 1. the nested commit is accepted, its body runs, and its result value
///    comes back through both frames;
/// 2. the enclosing restriction is back in force **after** the nested
///    commit returns — not cleared with it;
/// 3. **outside** every commit, dedication is ordinary and allowed —
///    otherwise the fix would have leaked a permanent restriction onto the
///    editor.
///
/// *Mutation:* refuse nested `commit_to` at the attempt, and this fails
/// while the masking test above still passes — which is what makes the two
/// a pair rather than one test written twice.
#[test]
fn an_ordinary_nested_commit_still_runs_and_restores_the_outer_restriction() {
    let s = editor();
    exec(&s, PANEL_ARRANGED);
    let panel = s
        .core
        .borrow()
        .side_window_for(FrontendId::LOCAL)
        .expect("the arrangement creates the panel");
    capture(&s);

    commit_body(
        &s,
        Some("'panel'"),
        "local inner = pmacs.window.capture_destination()
         -- A nested commit doing ordinary work: no dedication anywhere.
         nested_ok, nested_value = pmacs.window.commit_to(inner, function()
           pmacs.window.display(pmacs.buffer.create('*nested*'), { select = false })
           return 'inner-result'
         end)
         -- And the enclosing restriction is back afterwards.
         local caught, a = pcall(pmacs.window.set_params,
                                 pmacs.window.panel(), { dedicated = true })
         after_nested_raised = (not caught) and tostring(a) or nil",
    );

    assert_eq!(raised(&s), None, "the outer commit must not raise");
    assert!(ok(&s), "the outer commit must be accepted: {}", reason(&s));

    // 1. The nested commit ran and its value came back through both frames.
    assert!(
        eval::<bool>(&s, "return nested_ok"),
        "a nested commit that touches no dedication must be accepted -- forbidding all \
         nesting when only the restriction needed preserving is a behaviour regression"
    );
    assert_eq!(
        eval::<String>(&s, "return tostring(nested_value)"),
        "inner-result",
        "the nested body's return value must come back through both commit frames"
    );
    assert!(
        buffer_exists(&s, "*nested*"),
        "the nested body's own work must have happened"
    );

    // 2. The enclosing restriction is back in force after the nested
    //    commit returned -- popped, not cleared.
    let after: Option<String> = eval(&s, "return after_nested_raised");
    let after = after.expect(
        "the enclosing \"panel\" restriction must be back in force once the nested commit \
         returns -- a fix that cleared the stack on the inner exit would leave the rest of \
         the outer body unguarded",
    );
    assert!(
        after.contains("cannot dedicate the side window"),
        "and it must be the same refusal; got {after:?}"
    );
    assert!(!dedicated(&s, panel), "the slot must still be undedicated");

    // 3. OUTSIDE every commit, dedication is ordinary again: the guard
    //    must not have leaked a permanent restriction onto the editor.
    exec(
        &s,
        "pmacs.window.set_params(pmacs.window.panel(), { dedicated = true })",
    );
    assert!(
        dedicated(&s, panel),
        "outside a commit the field is writable as it always was (Q#BP2c)"
    );
}

/// **P** — a `"panel"` commit that falls back with a **still-valid**
/// destination lands in the document window, exactly as it does today.
///
/// The guard refuses on *staleness*, not on *falling back*. Falling back
/// is deliberate graceful degradation for a frontend that cannot render a
/// panel (`EditorCore::apply_placement`), and turning it into an error
/// would regress every consumer that works today on such a frontend — a
/// much bigger behaviour change than the defect being fixed.
///
/// Both causes, and asserted on **where the result landed** rather than
/// on the commit merely being accepted: a design that accepted the commit
/// and then dropped the display on the floor would pass a weaker version
/// of this.
///
/// *Mutation:* make `commit_destination_refusal` refuse outright whenever
/// a `"panel"` commit could fall back, instead of holding it to the
/// document preconditions. Both rows fail here; every refusal test still
/// passes, which is what makes this the pin that stops the fix
/// over-reaching.
#[test]
fn a_panel_commit_that_falls_back_with_a_valid_destination_still_lands() {
    for cause in ["not panel-capable", "side slot dedicated elsewhere"] {
        let s = editor();
        arrange_fallback(&s, cause);
        capture(&s);
        let doc = local_window(&s);

        // No staleness: the captured window still holds what it held.
        commit_body(&s, Some("'panel'"), PANEL_BODY);

        assert_eq!(raised(&s), None, "{cause}: the commit must not raise");
        assert!(
            ok(&s),
            "{cause}: a fallback with an intact destination is graceful degradation, not \
             an error; got refusal {:?}",
            reason(&s)
        );
        assert!(ran(&s), "{cause}: the callback must run");
        assert_eq!(
            name_in(&s, doc),
            "*result*",
            "{cause}: and the result really must land in the document window it fell \
             back to"
        );
    }
}

/// **P** — a `"panel"` commit that really lands in the panel still skips
/// checks 2–4.
///
/// The other half of the correction, and it is not optional coverage.
/// The cheapest way to close the fallback hole is to make the panel
/// profile run the document preflight unconditionally — which passes
/// every fallback row above while quietly collapsing the two profiles
/// into one, leaving the whole parameterization buying nothing and
/// `git.status` refused for a document-window change unrelated to where
/// its panel goes.
///
/// Deliberately arranged in the **same stale-intent state** the fallback
/// rows refuse on, so the only difference between this test and those is
/// whether the placement is really a panel. And it asserts *where* the
/// result went, not merely that the commit was accepted: an accepted
/// commit that still overwrote the document window would be the same
/// defect wearing a `true`.
///
/// *Mutation:* widen the relaxation's condition back — i.e. make
/// `panel_placement_can_fall_back` return `true` unconditionally, or run
/// the document preflight for every `"panel"` commit. This fails on the
/// refusal; the fallback rows above still pass. **This is the pin that
/// makes "collapse the two profiles into one" a visible design change
/// rather than a quiet implementation choice.**
#[test]
fn a_panel_commit_that_really_lands_in_the_panel_keeps_its_relaxation() {
    let s = editor();
    capture(&s);
    let doc = local_window(&s);

    // Exactly the state the fallback rows refuse on.
    exec(
        &s,
        "pmacs.window.switch_buffer(pmacs.buffer.create('*newer*'))",
    );

    commit_body(&s, Some("'panel'"), PANEL_BODY);

    assert!(
        ok(&s),
        "a panel-capable frontend with no dedicated side slot really places in the \
         panel, so checks 2-4 stay omitted; got refusal {:?}",
        reason(&s)
    );
    assert!(ran(&s), "and the callback must run");

    let panel = s
        .core
        .borrow()
        .side_window_for(FrontendId::LOCAL)
        .expect("the commit must have created the side window");
    assert_eq!(
        name_in(&s, panel),
        "*result*",
        "the result must land in the PANEL -- an accepted commit that fell back would \
         be the same defect with a `true` in front of it"
    );
    assert_eq!(
        name_in(&s, doc),
        "*newer*",
        "and the captured document window must be untouched"
    );
}

// ---------------------------------------------------------------------------
// §7 — the profile argument (Q#DC-5)
// ---------------------------------------------------------------------------

/// **P** — a two-argument `commit_to(dest, body)` takes the **document**
/// profile, so every caller written before the profile existed keeps all
/// four checks.
///
/// Witnessed by a check the panel profile omits — a stale buffer.
/// Asserting merely that the call does not error would pass on a legacy
/// call silently downgraded to the panel profile, which is the
/// regression that would quietly void Journey Stage 1a's guarantees for
/// dired and every future two-argument caller.
///
/// *Mutation:* default the profile to `Panel`. This fails;
/// `journey_acceptance` also fails, which is the point — the default is
/// what makes that suite's untouched pass a consequence of the signature
/// rather than of care.
#[test]
fn a_two_argument_commit_takes_the_document_profile() {
    let s = editor();
    capture(&s);
    exec(
        &s,
        "pmacs.window.switch_buffer(pmacs.buffer.create('*usurper*'))",
    );

    commit(&s, None);

    assert!(
        !ok(&s),
        "a two-argument commit must keep the stale-intent check"
    );
    assert!(
        reason(&s).contains("now shows another buffer"),
        "and refuse for that reason; got {:?}",
        reason(&s)
    );
    assert!(!ran(&s), "the callback must not run");
}

/// **N** — an explicit `nil` profile is the document profile, exactly as
/// omitting it is.
///
/// Witnessed separately from the two-argument case rather than assumed
/// equivalent: a Lua caller threading an optional variable produces
/// `commit_to(dest, body, nil)`, and a third behaviour there would stay
/// invisible until someone hit it in production.
///
/// *Mutation:* treat `Value::Nil` as an unrecognized profile. This
/// fails; the two-argument test above does not, because mlua supplies
/// `Nil` for a missing argument either way only if the binding asks for
/// a `Value` — which is the type this suite also pins below.
#[test]
fn an_explicit_nil_profile_is_the_document_profile() {
    let s = editor();
    capture(&s);
    exec(
        &s,
        "pmacs.window.switch_buffer(pmacs.buffer.create('*usurper*'))",
    );

    commit(&s, Some("nil"));

    assert_eq!(
        raised(&s),
        None,
        "an explicit nil must not be treated as a bad profile"
    );
    assert!(!ok(&s), "an explicit nil must keep the stale-intent check");
    assert!(
        reason(&s).contains("now shows another buffer"),
        "and refuse for that reason; got {:?}",
        reason(&s)
    );
}

/// **N** — an unrecognized profile is an ERROR naming the accepted
/// values, and a non-string profile is refused by the **same** message.
///
/// Two claims, one test, because their whole content is that they agree:
///
/// * a fallback to `"document"` would hand a caller different checks
///   than it asked for — the failure the parameterization exists to
///   prevent — so an unknown string raises;
/// * **this is the guard on the argument's type.** With
///   `profile: Option<String>` mlua rejects `42` and `{}` during
///   argument *conversion*, before the closure body runs, and the
///   message below becomes unreachable — the caller gets a generic
///   conversion error naming neither the rule nor the vocabulary. So the
///   number and table cases are asserted on the message's *content* and
///   against the string case's message, not merely on "an error
///   occurred".
///
/// **The `invalid utf-8` row is the same reachability class one layer
/// down.** A Lua string is a *byte* string, so `string.char(255)` is a
/// perfectly ordinary `Value::String` that a `to_str()` inside the body
/// still fails to convert — surfacing mlua's generic UTF-8 error before
/// the documented message is ever constructed. Accepting `Value` is not
/// enough on its own; the comparison has to be on bytes.
///
/// *Mutation:* retype the argument to `Option<String>`. The number and
/// table rows fail. *Second mutation:* compare via `name.to_str()?`. The
/// `invalid utf-8` row fails.
#[test]
fn a_bad_profile_is_refused_by_one_message_that_names_the_accepted_values() {
    let mut messages = Vec::new();
    for (label, profile) in [
        ("unknown string", "'documents'"),
        ("number", "42"),
        ("table", "{}"),
        ("boolean", "true"),
        ("invalid utf-8", "string.char(255)"),
    ] {
        let s = editor();
        capture(&s);
        commit(&s, Some(profile));

        let raised = raised(&s).unwrap_or_else(|| panic!("{label}: a bad profile must raise"));
        assert!(
            raised.contains("\"document\"") && raised.contains("\"panel\""),
            "{label}: the message must name both accepted values; got {raised:?}"
        );
        assert!(
            raised.contains("must be the string"),
            "{label}: and say a string was expected; got {raised:?}"
        );
        assert!(
            !ran(&s),
            "{label}: a bad profile must not reach the callback"
        );
        messages.push((label, raised));
    }

    let (_, first) = &messages[0];
    for (label, message) in &messages[1..] {
        assert_eq!(
            message, first,
            "{label}: a non-string profile must be refused by the SAME message as an \
             unrecognized one -- a different message means mlua rejected the value \
             during argument conversion, which is what `Option<String>` would do"
        );
    }
}

// ---------------------------------------------------------------------------
// §7 — no document window (Q#DC-4)
// ---------------------------------------------------------------------------

/// **N** — a frontend with no live document window still captures, and
/// the destination reports the absence.
///
/// Asserted as a *successful* capture rather than as `nil`: returning
/// `nil` here would push the adopter back onto ambient behaviour, which
/// is the P1a bug this lane removes. An adopter with nowhere to land
/// gets a refusal it can report; it does not get permission to guess.
///
/// The `window()` accessor reporting **nil** is the other half: the pair
/// is set or cleared together, so no consumer ever sees a window id
/// without the buffer that was captured with it.
///
/// *Mutation:* return `None` from `capture_view_destination` when
/// `primary_document_window` finds nothing. This fails on the capture
/// assertion inside the helper. *Second mutation:* keep `window` while
/// clearing `buffer`. This fails here.
#[test]
fn capture_succeeds_with_no_document_window() {
    let s = editor();
    attach_documentless_frontend(&s, DOCUMENTLESS);
    s.core.borrow_mut().active_frontend = DOCUMENTLESS;

    capture(&s);

    assert!(
        eval::<bool>(&s, "return dest:window() == nil"),
        "the document pair must be reported as ABSENT, not invented"
    );
}

/// **N** — on that destination a panel commit **succeeds** and a
/// document commit is **refused**, naming the missing window.
///
/// Both halves, because asserting only the refusal would pass on a
/// capture that refuses everything, and asserting only the success would
/// pass on one that checks nothing. Together they are Q#DC-4's decision:
/// the document pair is optional, and the profile is what decides
/// whether its absence matters.
///
/// The refusal is a `(false, reason)` like the other four rather than a
/// raise, so an adopter handles all five the same way.
///
/// *Mutation:* drop the `dest.window == None` arm. The document half
/// then commits against no window at all.
#[test]
fn a_panel_commit_succeeds_where_a_document_commit_is_refused() {
    let s = editor();
    attach_documentless_frontend(&s, DOCUMENTLESS);
    s.core.borrow_mut().active_frontend = DOCUMENTLESS;
    capture(&s);

    commit(&s, Some("'panel'"));
    assert!(
        ok(&s),
        "a panel result needs only a live frontend; got refusal {:?}",
        reason(&s)
    );
    assert!(ran(&s), "and its callback must run");

    commit(&s, None);
    assert_eq!(
        raised(&s),
        None,
        "the missing document window joins the preflight refusals rather than raising"
    );
    assert!(!ok(&s), "a document commit has nowhere to land");
    assert!(
        reason(&s).contains("no document window"),
        "and must say so; got {:?}",
        reason(&s)
    );
    assert!(!ran(&s), "and must not reach the callback");
}

// Isolated bootstrap storage roots (see the module docs): an
// integration test is compiled without `cfg(test)`, so a raw
// `EditorState::new()` would read the developer's real `init.lua` and
// write into their real data root.
#[path = "common/iso.rs"]
mod iso;
