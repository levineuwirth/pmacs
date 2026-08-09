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
//! Two disciplines it keeps:
//!
//! * **Every "not applicable" cell in Q#DC-2's preflight matrix is
//!   asserted as NOT refusing**, not merely left untested. A check
//!   deliberately omitted and a check someone forgot look identical from
//!   the outside, and the next reader restores the second one.
//! * **A refusal is asserted on its reason**, never on the mere fact
//!   that something failed. `commit_to` has five distinct refusals and a
//!   raise; "it errored" would pass on any of the wrong ones.
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

/// Run `body` under `profile` and report `(ok, reason)`.
///
/// `profile` is spliced as a Lua expression, so a caller can pass
/// `"nil"`, `"'panel'"`, `"42"` — the argument-shape distinctions
/// Q#DC-5 turns on are exactly what this suite has to vary.
fn commit(s: &EditorState, profile: Option<&str>) {
    let call = match profile {
        Some(profile) => format!("pmacs.window.commit_to(dest, body, {profile})"),
        None => "pmacs.window.commit_to(dest, body)".to_string(),
    };
    exec(
        s,
        &format!(
            "ran = false
             local body = function() ran = true end
             raised = nil
             local caught, a, b = pcall(function() return {call} end)
             if caught then ok, reason = a, b
             else ok, reason, raised = false, nil, tostring(a) end"
        ),
    );
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
/// *Mutation:* retype the argument to `Option<String>`. The number and
/// table rows fail.
#[test]
fn a_bad_profile_is_refused_by_one_message_that_names_the_accepted_values() {
    let mut messages = Vec::new();
    for (label, profile) in [
        ("unknown string", "'documents'"),
        ("number", "42"),
        ("table", "{}"),
        ("boolean", "true"),
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
