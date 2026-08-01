// tests/discovery_acceptance.rs --- P4 Stage 1, the discovery family.

//! `COHERENCE.md` §5 graded discoverability "substrate without surface".
//! These pins cover the surface:
//! `docs/discovery-stage1-command-family-framing.md` §4.
//!
//! **Every command is driven through the real M-x path**, stated once in
//! `run_from_palette` below. `pmacs.command.invoke_interactive` is *not*
//! M-x — it rotates the interactive-command boundary and calls the body;
//! it opens no palette. Journey Stage 1b-2 established this and Stage
//! 1b-3 re-established it, so it is encoded in a helper here rather than
//! left to each pin to remember.
//!
//! Six of the eleven canonical commands open a **second** prompt. A pin
//! that stops after the first RET has tested the palette, not the
//! command.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::protocol::FrontendId;

#[path = "common/iso.rs"]
mod iso;

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_owned()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_owned()).eval().unwrap()
}

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: mods,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn press(s: &mut EditorState, code: KeyCode) {
    s.dispatch_key(FrontendId::LOCAL, key(code, KeyModifiers::NONE));
}

fn type_str(s: &mut EditorState, text: &str) {
    for ch in text.chars() {
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char(ch), KeyModifiers::NONE),
        );
    }
}

fn minibuffer_active(s: &EditorState) -> bool {
    eval(s, "return pmacs.minibuffer.is_active()")
}

fn named_text(s: &EditorState, name: &str) -> String {
    eval(
        s,
        &format!(
            r#"
            for _, id in ipairs(pmacs.buffer.list()) do
                if pmacs.describe.buffer(id).name == {name:?} then
                    return id:slice(0, id:len())
                end
            end
            return ""
            "#
        ),
    )
}

fn help_text(s: &EditorState) -> String {
    named_text(s, "*help*")
}

/// Drive the **real** M-x path: dispatch `M-x`, type the command name,
/// assert the palette selected exactly that command *before* RET — the
/// only moment it is observable, since `accept()` does `session.take()`
/// and a selected candidate shadows typed text — then accept.
///
/// `second` is the argument for the six commands that open another
/// prompt; `None` for the five that do not.
fn run_from_palette(s: &mut EditorState, command: &str, second: Option<&str>) {
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char('x'), KeyModifiers::ALT),
    );
    assert!(minibuffer_active(s), "M-x must open the palette");
    type_str(s, command);
    assert_eq!(
        eval::<Option<String>>(s, "return pmacs.minibuffer.selected()").as_deref(),
        Some(command),
        "the palette must have {command} selected; a different candidate \
         would run a different command"
    );
    press(s, KeyCode::Enter);

    if let Some(arg) = second {
        assert!(
            minibuffer_active(s),
            "{command} takes an argument and must open a second prompt"
        );
        exec(s, &format!("pmacs.minibuffer.set_contents({arg:?})"));
        press(s, KeyCode::Enter);
    } else {
        assert!(
            !minibuffer_active(s),
            "{command} takes no argument; a second prompt means the census is wrong"
        );
    }
}

/// Constructed with **isolated bootstrap roots**, never ambiently.
///
/// An integration test is compiled without `cfg(test)`, so a raw
/// `EditorState::new()` reads the developer's real `init.lua` and writes
/// bundled packages into their real data root. The adoption ratchet in
/// `ambient_isolation_acceptance` caught this suite the moment the
/// isolation lane merged — which is the ratchet doing its job against
/// brand-new code, so this file is migrated rather than allowlisted.
fn editor() -> EditorState {
    EditorState::new_with_roots(&iso::roots())
}

/// The eleven canonical commands and the argument each needs, if any.
/// **Six take one** — `describe-command` is easy to forget, because it
/// joined the family by rename rather than by being new.
fn family() -> Vec<(&'static str, Option<&'static str>)> {
    vec![
        ("help.describe-command", Some("help.list-commands")),
        ("help.describe-setting", None), // supplied per-test: needs a real setting
        ("help.describe-key", Some("C-x C-f")),
        ("help.describe-mode", None),
        ("help.describe-buffer", None),
        ("help.describe-hook", Some("buffer.after-load")),
        ("help.where-is", Some("help.list-commands")),
        ("help.list-commands", None),
        ("help.list-keybindings", None),
        ("help.list-settings", None),
        ("help.apropos", Some("compile")),
    ]
}

// ---------------------------------------------------------------------------
// The family runs
// ---------------------------------------------------------------------------

/// **N (acceptance 1)** — every canonical command runs from M-x and
/// renders content, including the second prompt where it takes one.
#[test]
fn d1_every_command_runs_from_the_palette_and_renders() {
    for (name, arg) in family() {
        let mut s = editor();
        // `describe-setting` needs a setting that exists; take the first.
        let arg = if name == "help.describe-setting" {
            Some(eval::<String>(&s, "return pmacs.config.list()[1].name"))
        } else {
            arg.map(str::to_owned)
        };
        run_from_palette(&mut s, name, arg.as_deref());
        let text = help_text(&s);
        assert!(
            !text.is_empty(),
            "{name} must render content into *help*; got empty"
        );
    }
}

/// **N (acceptance 2)** — `where-is` agrees with the keymap.
///
/// Falsified by rendering a static string.
#[test]
fn d2_where_is_reports_the_real_binding() {
    let mut s = editor();
    exec(
        &s,
        "pmacs.command.define { name = 'test.whereis-probe',
           description = 'probe', fn = function() end }
         pmacs.keymap.bind { scope = 'global', sequence = 'C-c Q',
           command = 'test.whereis-probe' }",
    );
    run_from_palette(&mut s, "help.where-is", Some("test.whereis-probe"));
    let text = help_text(&s);
    assert!(
        text.contains("C-c Q"),
        "where-is must report the chord actually bound; got:\n{text}"
    );
}

/// **N (acceptance 3)** — `list-keybindings` covers every binding
/// `keymap.list()` reports. A property over the data, not a fixed list.
#[test]
fn d3_list_keybindings_covers_every_binding() {
    let mut s = editor();
    let sequences: Vec<String> = eval(
        &s,
        "local out = {}
         for _, r in ipairs(pmacs.keymap.list()) do out[#out+1] = r.sequence end
         return out",
    );
    assert!(
        !sequences.is_empty(),
        "precondition: the keymap must be non-empty or this loop is vacuous"
    );
    run_from_palette(&mut s, "help.list-keybindings", None);
    let text = help_text(&s);
    for seq in sequences {
        assert!(
            text.contains(&seq),
            "list-keybindings omits {seq:?}; got:\n{text}"
        );
    }
}

// ---------------------------------------------------------------------------
// apropos — substring, not fuzzy
// ---------------------------------------------------------------------------

/// **N (acceptance 4)** — apropos matches descriptions, not only names,
/// **and does so by substring**.
///
/// The negative half is the one that pins Q#D3. A bare "a subsequence
/// finds nothing" assertion would pass as an ordinary no-match; this
/// registers a fixture whose description contains `qzjx` **only** as the
/// non-contiguous sequence `q z j x`, and first proves no registered
/// command contains `qzjx` as a substring. A fuzzy implementation finds
/// the fixture, so the pin fails under fuzzy rather than passing.
#[test]
fn d4_apropos_matches_descriptions_by_substring_not_subsequence() {
    let mut s = editor();
    exec(
        &s,
        "pmacs.command.define { name = 'test.apropos-description-probe',
           description = 'zzyzx marker for the description-search pin',
           fn = function() end }
         pmacs.command.define { name = 'test.apropos-subsequence-fixture',
           description = 'q z j x letters spaced apart on purpose',
           fn = function() end }",
    );

    // Positive: a word in exactly one DESCRIPTION and no NAME.
    let name_hits: i64 = eval(
        &s,
        "local n = 0
         for _, c in ipairs(pmacs.command.list()) do
           if c:lower():find('zzyzx', 1, true) then n = n + 1 end
         end
         return n",
    );
    assert_eq!(name_hits, 0, "precondition: 'zzyzx' is in no command NAME");
    run_from_palette(&mut s, "help.apropos", Some("zzyzx"));
    assert!(
        help_text(&s).contains("test.apropos-description-probe"),
        "apropos must search descriptions, not only names; got:\n{}",
        help_text(&s)
    );

    // Negative, discriminating: `qzjx` is a subsequence of the fixture's
    // description but a substring of nothing.
    let substring_hits: i64 = eval(
        &s,
        "local n = 0
         for _, c in ipairs(pmacs.command.list()) do
           local d = pmacs.describe.command(c)
           local desc = (d and d.description) or ''
           if c:lower():find('qzjx', 1, true) or desc:lower():find('qzjx', 1, true) then
             n = n + 1
           end
         end
         return n",
    );
    assert_eq!(
        substring_hits, 0,
        "precondition: 'qzjx' must be a substring of nothing, or the \
         negative below proves nothing"
    );

    let mut s2 = editor();
    exec(
        &s2,
        "pmacs.command.define { name = 'test.apropos-subsequence-fixture',
           description = 'q z j x letters spaced apart on purpose',
           fn = function() end }",
    );
    run_from_palette(&mut s2, "help.apropos", Some("qzjx"));
    assert!(
        !help_text(&s2).contains("test.apropos-subsequence-fixture"),
        "substring matching must NOT find a subsequence — a fuzzy \
         implementation finds the fixture here; got:\n{}",
        help_text(&s2)
    );
}

// ---------------------------------------------------------------------------
// describe-setting completion
// ---------------------------------------------------------------------------

/// **N (acceptance 5)** — completion assists, and a non-matching typo
/// still reaches the existing error path.
///
/// Both halves, because §3.2 has two outcomes and rev 1 asserted only a
/// third that does not exist ("a typo cannot reach `on_accept`").
#[test]
fn d5_describe_setting_completes_and_a_typo_still_errors() {
    let mut s = editor();
    let first: String = eval(&s, "return pmacs.config.list()[1].name");

    // (a) typing a real setting's full name selects it.
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char('x'), KeyModifiers::ALT),
    );
    type_str(&mut s, "help.describe-setting");
    press(&mut s, KeyCode::Enter);
    assert!(minibuffer_active(&s), "the setting prompt must open");
    exec(&s, &format!("pmacs.minibuffer.set_contents({first:?})"));
    assert_eq!(
        eval::<Option<String>>(&s, "return pmacs.minibuffer.selected()").as_deref(),
        Some(first.as_str()),
        "a real setting name must be the selected candidate"
    );
    press(&mut s, KeyCode::Enter);
    assert!(
        help_text(&s).contains(&first),
        "accepting a completed setting describes it"
    );

    // (b) a name matching nothing still reaches `on_accept` and errors —
    // completion is assistance, not validation.
    let mut s2 = editor();
    run_from_palette(
        &mut s2,
        "help.describe-setting",
        Some("qqzz-no-such-setting"),
    );
    assert!(
        s2.core.borrow().status.contains("no such setting"),
        "a non-matching typo reaches the existing error path; status: {:?}",
        s2.core.borrow().status
    );
}

// ---------------------------------------------------------------------------
// The index
// ---------------------------------------------------------------------------

/// **N (acceptance 6)** — `M-x help` lists the family, as a property.
///
/// Targeted mutation: adding a twelfth canonical command without
/// indexing it.
#[test]
fn d6_the_help_index_lists_every_family_command() {
    let mut s = editor();
    let family: Vec<String> = eval(&s, "return pmacs.help.family");
    assert_eq!(
        family.len(),
        11,
        "the canonical family is eleven commands; update the index and \
         this pin together"
    );
    run_from_palette(&mut s, "help", None);
    let text = help_text(&s);
    for name in family {
        assert!(text.contains(&name), "the index omits {name}; got:\n{text}");
    }
}

// ---------------------------------------------------------------------------
// Preservation
// ---------------------------------------------------------------------------

/// **P (acceptance 7)** — every command's `*help*` write goes through
/// `_show_help`.
///
/// Pins §3.4's actual claim: one owner for `*help*` writes. Not a
/// one-site migration claim — `src/help.rs` has no renderer for
/// settings, lists or apropos.
#[test]
fn d7_preservation_every_render_goes_through_the_one_seam() {
    let mut s = editor();
    exec(
        &s,
        "_seam_calls = 0
         local real = pmacs.editor._show_help
         pmacs.editor._show_help = function(text)
           _seam_calls = _seam_calls + 1
           return real(text)
         end",
    );
    for (name, arg) in family() {
        let arg = if name == "help.describe-setting" {
            Some(eval::<String>(&s, "return pmacs.config.list()[1].name"))
        } else {
            arg.map(str::to_owned)
        };
        run_from_palette(&mut s, name, arg.as_deref());
    }
    assert_eq!(
        eval::<i64>(&s, "return _seam_calls"),
        11,
        "all eleven commands must render through _show_help; a command \
         writing its own buffer would not be counted"
    );
}

/// **P (acceptance 8)** — the old names still work, as forwarders.
///
/// Targeted mutation: dropping the forwarders after the rename, which is
/// the failure a user with muscle memory hits first.
#[test]
fn d8_preservation_the_old_names_forward() {
    let mut s = editor();
    run_from_palette(
        &mut s,
        "editor.describe-command",
        Some("help.list-commands"),
    );
    let forwarded = help_text(&s);
    assert!(
        forwarded.contains("help.list-commands"),
        "editor.describe-command must still describe the command it is \
         given; got:\n{forwarded}"
    );

    let mut s2 = editor();
    run_from_palette(&mut s2, "help.describe-command", Some("help.list-commands"));
    assert_eq!(
        forwarded,
        help_text(&s2),
        "the forwarder must render the same subject as its target"
    );
}

/// **P (acceptance 8, cont.)** — the forwarders also work when invoked
/// **programmatically**, not only from M-x.
///
/// This pin exists because its absence shipped a bug: the forwarder body
/// used `invoke_interactive`, which raises when the alias is reached
/// through `pmacs.command.invoke` — a real caller in
/// `config_registry_acceptance`. The M-x pin above passed throughout,
/// because M-x is not the only way in.
#[test]
fn d8c_preservation_the_forwarders_work_programmatically() {
    let s = editor();
    for old in ["editor.describe-command", "editor.describe-setting"] {
        let ok: bool = eval(
            &s,
            &format!("return pcall(pmacs.command.invoke, {old:?}) and true or false"),
        );
        assert!(
            ok,
            "{old} must be invocable programmatically, not only from the palette"
        );
    }
}

/// **P (acceptance 8, cont.)** — the untouched list commands still work.
#[test]
fn d8b_preservation_list_buffers_and_workers_are_untouched() {
    let s = editor();
    for name in ["editor.list-buffers", "editor.list-workers"] {
        assert!(
            eval::<bool>(&s, &format!("return pmacs.command.exists({name:?})")),
            "{name} must still be registered"
        );
    }
}

/// **P (acceptance 9)** — no command's predicate is evaluated.
///
/// Driven through the real palette, not `invoke_interactive` directly:
/// otherwise it would pass even if M-x itself grew predicate filtering.
/// A stage that starts evaluating predicates must change this pin
/// knowingly.
#[test]
fn d9_preservation_a_raising_predicate_does_not_block_a_command() {
    let mut s = editor();
    exec(
        &s,
        "_ran = false
         pmacs.command.define {
           name = 'test.predicate-probe',
           description = 'probe whose predicate raises',
           predicate = function() error('predicate evaluated') end,
           fn = function() _ran = true end,
         }",
    );
    run_from_palette(&mut s, "test.predicate-probe", None);
    assert!(
        eval::<bool>(&s, "return _ran"),
        "the command must run: predicates are stored and exposed but \
         never evaluated (framing §2.4)"
    );
}
