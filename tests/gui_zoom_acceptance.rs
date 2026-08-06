//! GUI zoom acceptance (`QoL` Stage 2, `docs/gui-zoom-framing.md`).
//!
//! Zoom drives the font preference that already existed: `set_font`
//! writes it, `semantic_render` relays it as `FontFacts` at v17, and the
//! GPU frontend owns every pixel consequence. Nothing here knows a
//! metric — the no-pixels invariant holds through the whole feature.
//!
//! Each test gets its **own** bootstrap roots. `iso::roots()` is a pure
//! function of the build environment by design, so every suite sharing
//! it shares one state directory — fine when nobody writes, wrong here,
//! where the state file *is* the subject and parallel tests would
//! overwrite each other's fixture.

use std::path::{Path, PathBuf};

use pmacs::bootstrap::BootstrapRoots;
use pmacs::editor::EditorState;

/// Per-test roots plus the path zoom's state file will occupy
/// (`<state>/pmacs/gpu-zoom`, per `src/state.rs`).
fn roots_for(name: &str) -> (BootstrapRoots, PathBuf) {
    let base = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("gui-zoom")
        .join(name);
    let _ = std::fs::remove_dir_all(&base);
    let roots = BootstrapRoots::isolated_under(&base);
    for (_, dir) in roots.child_env() {
        std::fs::create_dir_all(&dir).expect("create controlled root");
    }
    let dir = roots.state_dir().expect("isolated roots have a state dir");
    std::fs::create_dir_all(&dir).expect("create state dir");
    (roots, dir.join("gpu-zoom"))
}

/// A session whose state dirs are installed — which is also what runs
/// `pmacs.zoom.restore`, so anything planted at `path` beforehand is
/// visible to it.
fn session(roots: &BootstrapRoots) -> EditorState {
    let state = EditorState::new_with_roots(roots);
    state.install_state_dirs();
    state
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

/// Current preference size in logical px, or `None` when unset — the
/// real "frontend's own default" state, never inferred from silence.
fn size(s: &EditorState) -> Option<f64> {
    eval(s, "return pmacs.gpu.font().size")
}

fn family(s: &EditorState) -> Option<String> {
    eval(s, "return pmacs.gpu.font().family")
}

// ---------------------------------------------------------------------------
// The starting point (Q#Z1 = c)
// ---------------------------------------------------------------------------

/// The first step starts from the CONFIGURED base, not a hardcoded
/// 16.0 — which is the whole reason (c) was chosen over (a). The daemon
/// never learns what the frontend resolved; it reads a user-facing
/// preference about zooming.
#[test]
fn the_first_step_starts_from_the_configured_base_not_a_constant() {
    let (roots, _) = roots_for("first_step_base");
    let s = session(&roots);
    assert_eq!(size(&s), None, "premise: untouched means unset");

    exec(&s, r#"pmacs.config.set("ui.gpu-font-size-base", 20.0)"#);
    exec(&s, "pmacs.zoom.increase()");
    assert_eq!(
        size(&s),
        Some(21.0),
        "20.0 base + 1.0 step. A hardcoded origin would give 17.0"
    );
}

/// Until a zoom happens the preference stays unset, so the frontend's
/// own default query still runs. This is what option (b) would have
/// destroyed for every user who never zooms.
#[test]
fn defining_the_settings_does_not_itself_set_a_size() {
    let (roots, _) = roots_for("no_implicit_size");
    let s = session(&roots);
    assert_eq!(size(&s), None);
    assert_eq!(family(&s), None);
}

// ---------------------------------------------------------------------------
// Step arithmetic (Q#Z2 = additive)
// ---------------------------------------------------------------------------

/// n steps in, then n steps out, returns to EXACTLY the starting value.
/// True because the step is centi-pixel representable and addition is
/// exact in that domain — the property multiplicative stepping loses.
#[test]
fn n_steps_in_then_n_out_returns_exactly() {
    let (roots, _) = roots_for("round_trip");
    let s = session(&roots);
    exec(&s, r#"pmacs.config.set("ui.gpu-zoom-step", 0.37)"#);
    exec(&s, "pmacs.zoom.increase()");
    let start = size(&s).expect("a size exists after the first step");

    for _ in 0..7 {
        exec(&s, "pmacs.zoom.increase()");
    }
    for _ in 0..7 {
        exec(&s, "pmacs.zoom.decrease()");
    }
    assert_eq!(
        size(&s),
        Some(start),
        "exact return, not approximately — 0.37 is centi-pixel \
         representable and the domain is quantized"
    );
}

/// The round trip holds for a step that is NOT centi-pixel
/// representable, which is the case the 0.37 test above cannot reach.
///
/// The registry accepts any finite number in range — `ConfigKind::Number`
/// validates finiteness and bounds and nothing else, and `on_change`
/// cannot veto — so 0.015 is a settable step. Used raw it breaks the
/// contract: each operation rounds independently, 16.015 rounds up and
/// 16.005 rounds down, giving 16.00 -> 16.02 -> 16.01.
///
/// Quantizing the step at the point of use restores exactness for every
/// accepted step, not just the representable ones.
#[test]
fn the_round_trip_survives_a_step_that_is_not_representable() {
    let (roots, _) = roots_for("round_trip_unrepresentable");
    let s = session(&roots);
    exec(&s, "pmacs.gpu.set_font { size = 16.0 }");
    exec(&s, r#"pmacs.config.set("ui.gpu-zoom-step", 0.015)"#);

    exec(&s, "pmacs.zoom.increase()");
    assert_eq!(
        size(&s),
        Some(16.02),
        "the effective step is the quantized one — 0.015 rounds to 0.02,          the same operation `validate_font_size` already applies to sizes"
    );

    exec(&s, "pmacs.zoom.decrease()");
    assert_eq!(
        size(&s),
        Some(16.0),
        "and back exactly. Used raw, 0.015 would land on 16.01 here,          because each operation rounds independently"
    );
}

/// A base that is not representable still yields a predictable origin,
/// and every step after the first is exact.
#[test]
fn an_unrepresentable_base_is_quantized_too() {
    let (roots, _) = roots_for("base_quantized");
    let s = session(&roots);
    exec(&s, r#"pmacs.config.set("ui.gpu-font-size-base", 20.004)"#);
    exec(&s, "pmacs.zoom.increase()");
    assert_eq!(
        size(&s),
        Some(21.0),
        "20.004 quantizes to 20.00, then + 1.0"
    );
}

/// An out-of-range step leaves the preference **unmutated** rather than
/// pinning it to the boundary. Pinning would silently break the round
/// trip precisely at the edges, where a user steps back and forth most.
#[test]
fn a_step_past_the_boundary_changes_nothing_and_says_so() {
    let (roots, _) = roots_for("boundary");
    let s = session(&roots);
    exec(&s, "pmacs.gpu.set_font { size = 71.5 }");
    exec(&s, r#"pmacs.config.set("ui.gpu-zoom-step", 2.0)"#);

    let why: Option<String> = eval(&s, "local _, why = pmacs.zoom.increase() return why");
    assert_eq!(
        size(&s),
        Some(71.5),
        "the preference is untouched, not pinned to 72.0"
    );
    assert!(
        why.unwrap_or_default().contains("unchanged"),
        "and the caller is told why"
    );
}

// ---------------------------------------------------------------------------
// Config bounds (§3.1)
// ---------------------------------------------------------------------------

/// A zero, negative, or sub-centi-pixel step is refused by the REGISTRY,
/// not discovered later as a zoom that does nothing or runs backwards.
///
/// The negative case is the sharp one: it would invert the commands, and
/// `gpu.zoom-in` shrinking is not a malfunction a user can diagnose from
/// the outside — the command still does something coherent.
#[test]
fn an_unusable_step_is_refused_at_the_registry() {
    let (roots, _) = roots_for("step_bounds");
    let s = session(&roots);
    for bad in ["0.0", "-1.0", "0.001"] {
        let ok: bool = eval(
            &s,
            &format!(r#"return pcall(pmacs.config.set, "ui.gpu-zoom-step", {bad})"#),
        );
        assert!(!ok, "a step of {bad} must be refused");
    }
    // …and the smallest representable step is allowed.
    let ok: bool = eval(
        &s,
        r#"return pcall(pmacs.config.set, "ui.gpu-zoom-step", 0.01)"#,
    );
    assert!(ok, "0.01 is one centi-pixel — the smallest real step");
}

/// The base is bounded to the wire range. A base outside it could never
/// be sent, so the first step would fail from a value the registry had
/// allowed the user to set.
#[test]
fn the_base_is_bounded_to_the_wire_range() {
    let (roots, _) = roots_for("base_bounds");
    let s = session(&roots);
    for bad in ["5.99", "72.01"] {
        let ok: bool = eval(
            &s,
            &format!(r#"return pcall(pmacs.config.set, "ui.gpu-font-size-base", {bad})"#),
        );
        assert!(!ok, "a base of {bad} is outside 6.00-72.00");
    }
}

// ---------------------------------------------------------------------------
// The family clobber (§5a)
// ---------------------------------------------------------------------------

/// `set_font` replaces BOTH fields unconditionally, so a size-only write
/// would clear a family the user configured — and they would get it back
/// only by restarting. Every zoom write must carry the family through.
#[test]
fn a_zoom_preserves_a_configured_family() {
    let (roots, _) = roots_for("family_kept");
    let s = session(&roots);
    exec(
        &s,
        r#"pmacs.gpu.set_font { family = "Iosevka", size = 18.0 }"#,
    );

    exec(&s, "pmacs.zoom.increase()");
    assert_eq!(size(&s), Some(19.0));
    assert_eq!(
        family(&s).as_deref(),
        Some("Iosevka"),
        "the family survives a zoom — set_font replaces both fields, so \
         a size-only write would silently drop it"
    );

    exec(&s, "pmacs.zoom.decrease()");
    assert_eq!(
        family(&s).as_deref(),
        Some("Iosevka"),
        "and every step after"
    );
}

// ---------------------------------------------------------------------------
// Reset (§5b)
// ---------------------------------------------------------------------------

/// Reset returns the size to UNSET — the frontend's own default — not to
/// the configured base. Resetting to the base would ship an explicit
/// size that merely happens to equal the default, making the untouched
/// state unreachable once a user has ever zoomed.
#[test]
fn reset_returns_to_unset_not_to_the_base() {
    let (roots, path) = roots_for("reset_unsets");
    let s = session(&roots);
    exec(&s, r#"pmacs.gpu.set_font { family = "Iosevka" }"#);
    exec(&s, "pmacs.zoom.increase()");
    assert!(size(&s).is_some(), "premise: a size is set");
    assert!(path.exists(), "premise: the zoom was saved");

    exec(&s, "pmacs.zoom.reset()");
    assert_eq!(size(&s), None, "unset, not the base value");
    assert_eq!(
        family(&s).as_deref(),
        Some("Iosevka"),
        "reset is about size; a configured family is not collateral"
    );

    // Cleared, so it does not resurrect on the next launch — "reset
    // until restart" is not what the word says.
    let saved = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        saved.trim().is_empty(),
        "reset clears the saved zoom, found {saved:?}"
    );
}

// ---------------------------------------------------------------------------
// Persistence and its parser (§5a1)
// ---------------------------------------------------------------------------

/// The exact bytes the writer produces round-trip. This is the case the
/// framing's first parser (`^(%d+)$`) would have REJECTED — it anchors
/// to end-of-subject, so it fails on the trailing newline — which is why
/// it is asserted rather than assumed.
#[test]
fn a_valid_newline_terminated_file_restores() {
    let (roots, path) = roots_for("restore_valid");
    std::fs::write(&path, "1800\n").expect("plant state");
    let s = session(&roots);
    assert_eq!(
        size(&s),
        Some(18.0),
        "restored at the seam, from the bytes the writer emits"
    );
}

/// What zoom writes is what zoom reads. Pins the two halves against each
/// other so a format change cannot land in one alone.
#[test]
fn the_written_format_is_the_format_that_parses() {
    let (roots, path) = roots_for("format_round_trip");
    {
        let s = session(&roots);
        exec(&s, "pmacs.gpu.set_font { size = 23.5 }");
        exec(&s, "pmacs.zoom.increase()");
        assert_eq!(size(&s), Some(24.5));
    }
    let raw = std::fs::read_to_string(&path).expect("state written");
    assert_eq!(raw, "2450\n", "centi-pixels, newline-terminated");

    let s2 = session(&roots);
    assert_eq!(size(&s2), Some(24.5), "and a fresh session reads it back");
}

/// Malformed, out-of-range, and shape-wrong state all behave as absent,
/// and the file is left intact for inspection rather than truncated.
///
/// The multi-line case is the one that matters most: `saveplace`'s
/// `gmatch("([^\n]+)")` line iterator would happily accept its FIRST
/// line. That is right for `recentf`, where a line is one independent
/// entry, and wrong here, where the file IS the value.
#[test]
fn unparseable_or_out_of_range_state_behaves_as_absent() {
    for (label, contents) in [
        ("multi-line", "1800\n1900\n"),
        ("no trailing newline", "1800"),
        ("decimal", "18.0\n"),
        ("leading space", " 1800\n"),
        ("empty", ""),
        ("bare newline", "\n"),
        ("non-numeric", "big\n"),
        ("above the wire range", "9999\n"),
        ("below the wire range", "100\n"),
    ] {
        let (roots, path) = roots_for(&format!("bad_{}", label.replace(' ', "_")));
        std::fs::write(&path, contents).expect("plant state");
        let s = session(&roots);
        assert_eq!(
            size(&s),
            None,
            "{label}: {contents:?} must behave exactly as no saved state"
        );
        assert_eq!(
            std::fs::read_to_string(&path).expect("still there"),
            contents,
            "{label}: the file is left intact — truncating it on read \
             would destroy the only evidence of whatever wrote it"
        );
    }
}

/// Precedence: a valid saved zoom beats a size set during startup, and
/// never touches a configured family.
///
/// The saved value is a later, deliberate user action; the init value is
/// the standing default it was chosen against — the same precedence
/// `saveplace` already applies to a remembered position.
#[test]
fn saved_state_beats_a_startup_size_but_never_the_family() {
    let (roots, path) = roots_for("precedence");
    std::fs::write(&path, "2600\n").expect("plant state");
    let s = EditorState::new_with_roots(&roots);
    // Stand in for init.lua, which runs before state is installed.
    exec(
        &s,
        r#"pmacs.gpu.set_font { family = "Iosevka", size = 12.0 }"#,
    );
    s.install_state_dirs();

    assert_eq!(size(&s), Some(26.0), "the remembered zoom wins on SIZE");
    assert_eq!(
        family(&s).as_deref(),
        Some("Iosevka"),
        "and never on family — restored state does not carry one"
    );
}

// ---------------------------------------------------------------------------
// No keybindings (Q#Z3 = C)
// ---------------------------------------------------------------------------

/// This stage ships commands and NO default bindings.
///
/// The keymap has no way to say "GPU frontends only" — `Scope` is
/// `Buffer | Mode | Global` and carries no frontend identity — so a
/// global binding would capture the chord in the TUI and take away the
/// terminal's own zoom, which is the very thing the user is pressing it
/// for. Better an unbound key than one that answers with an apology.
#[test]
fn the_commands_exist_and_nothing_is_bound() {
    let (roots, _) = roots_for("no_bindings");
    let s = session(&roots);

    // `pmacs.command.list()` returns plain names, as `help.lua` reads it.
    let names: Vec<String> = eval(&s, "return pmacs.command.list()");
    for want in ["gpu.zoom-in", "gpu.zoom-out", "gpu.zoom-reset"] {
        assert!(names.iter().any(|n| n == want), "{want} is discoverable");
    }

    let bound: Vec<String> = eval(
        &s,
        r#"local out = {}
           for _, b in ipairs(pmacs.keymap.list()) do
             if b.command and b.command:match("^gpu%.zoom") then
               out[#out + 1] = string.format("%s -> %s (%s)", b.sequence, b.command, b.scope)
             end
           end
           return out"#,
    );
    assert!(
        bound.is_empty(),
        "no zoom keybinding may be installed by default, found {bound:?}"
    );
}
