// tests/e10a_537_shifted_chord_acceptance.rs --- #537: a chord on a
// character the layout shifts reads as the character, in both frontends.

//! On a US layout `M-?` is Alt+Shift+/, `C-_` Ctrl+Shift+- and `C-x ^`
//! `C-x` then Shift+6. The GPU sent each character the layout made with
//! the Shift that made it still set, and `Chord::new` strips Shift from
//! letters alone, so the daemon read `M-S-?`, `C-S-_` (redo, not undo)
//! and `C-x S-^`. The TUI on a kitty-protocol terminal read the unshifted
//! key with Shift (`M-S-/`), since it asked only for disambiguated
//! escape codes. Both now read what a legacy terminal and Emacs read:
//! the GPU drops Shift from a character key, and the TUI asks the
//! terminal for the shifted key as well (flags 1|4).
//!
//! The GPU's row presses the physical chords through its production
//! `App` against a real daemon, the keymap reached named on the band;
//! the unit rows beside `translate_key` read every documented chord off
//! the wire. The TUI's rows read the flags it pushes, and replay foot's
//! own bytes for each chord, under those flags, through a real TUI's
//! input path to the echo line.

mod common;
#[path = "support/mod.rs"]
mod support;

/// A command per chord, each saying which chord reached it, bound over
/// the defaults (`C-_` and `C-?` are undo and redo there; a bound
/// sequence is unbound first, since `bind` refuses one).
const SAW_INIT: &str = r#"
pmacs.lsp.config = {}
local function saw(sequence, name)
  pmacs.command.define {
    name = name,
    description = "Say that " .. sequence .. " reached the keymap.",
    fn = function() pmacs.editor.set_status("saw " .. sequence) end,
  }
  pcall(pmacs.keymap.unbind, { scope = "global", sequence = sequence })
  pmacs.keymap.bind { scope = "global", sequence = sequence, command = name }
end
saw("M-?", "e537.saw-meta-question")
saw("C-_", "e537.saw-control-underscore")
saw("C-?", "e537.saw-control-question")
saw("C-x ^", "e537.saw-c-x-caret")
"#;

/// The GPU, Alt+Shift+/, Ctrl+Shift+-, Ctrl+Shift+/ and `C-x` then
/// Shift+6 as winit reports them on a US layout, through the headless
/// probe's production `App`: each reaches the chord a terminal and Emacs
/// read. At `176a143` the band said `M-S-?: not bound`, Ctrl+Shift+-
/// ran redo (`C-S-_`), Ctrl+Shift+/ was `C-S-?` and `C-x S-^` unbound.
#[cfg(feature = "crdt")]
#[test]
fn e537_the_gpu_reads_a_shifted_character_chord_as_the_character() {
    use common::daemon::TestDaemon;

    let dir = std::env::temp_dir().join(format!("pmacs-e537-gpu-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let notes = dir.join("notes.txt");
    std::fs::write(&notes, "one\ntwo\nthree\n").unwrap();
    let init = format!(
        "{SAW_INIT}pmacs.buffer.find_or_open({:?})\n",
        notes.display().to_string()
    );
    let daemon = TestDaemon::spawn_with_env_and_init(&[], &init);
    let steps = "shows:notes.txt \
                 key:M-S-? quiet:700 place:meta \
                 key:C-S-_ quiet:700 place:undo \
                 key:C-S-? quiet:700 place:redo \
                 key:C-x quiet:500 key:S-^ quiet:700 place:caret";
    let Some(facts) = common::gpu_probe::run_script_probe(daemon.socket_path(), "e537", steps)
    else {
        support::record_skip("the GPU probe did not run");
        return;
    };
    let fact = |key: &str| common::gpu_probe::fact(&facts, key).to_owned();
    assert_eq!(
        fact("meta.name"),
        "notes.txt",
        "premise: the window shows notes.txt; facts {facts:?}"
    );
    let read: Vec<(&str, String)> = [
        ("meta", "M-?"),
        ("undo", "C-_"),
        ("redo", "C-?"),
        ("caret", "C-x ^"),
    ]
    .into_iter()
    .filter(|(label, chord)| !fact(&format!("{label}.status")).contains(&format!("saw {chord}")))
    .map(|(label, _)| (label, fact(&format!("{label}.status"))))
    .collect();
    assert!(
        read.is_empty(),
        "each physical chord reaches the chord of its character; the band read {read:?}; \
         facts {facts:?}"
    );
}

// ---------------------------------------------------------------------------
// The TUI on a kitty-protocol terminal
// ---------------------------------------------------------------------------

use common::pty::{ScreenTui, spawn_pmacs_in_pty_tapped};
use std::time::Duration;

const LIMIT: Duration = Duration::from_secs(30);

/// What foot 1.28 sent for each physical chord on a standard US
/// (evdev, pc105) keymap, recorded in fix round 2 by a virtual keyboard
/// uploading that keymap: under the kitty flags 1 the TUI pushed until
/// #537, and under flags 5 (1|4, `REPORT_ALTERNATE_KEYS`). Columns: the
/// keys, the chord the keymap should see, the bytes under 1, under 5.
const FOOT: [(&str, &str, &[u8], &[u8]); 10] = [
    ("Alt+Shift+/", "M-?", b"\x1b[47;4u", b"\x1b[47:63;4u"),
    ("Alt+Shift+,", "M-<", b"\x1b[44;4u", b"\x1b[44:60;4u"),
    ("Alt+Shift+.", "M->", b"\x1b[46;4u", b"\x1b[46:62;4u"),
    ("Alt+Shift+5", "M-%", b"\x1b[53;4u", b"\x1b[53:37;4u"),
    ("Alt+Shift+1", "M-!", b"\x1b[49;4u", b"\x1b[49:33;4u"),
    ("Alt+Shift+6", "M-^", b"\x1b[54;4u", b"\x1b[54:94;4u"),
    ("Alt+Shift+[", "M-{", b"\x1b[91;4u", b"\x1b[91:123;4u"),
    ("Alt+Shift+]", "M-}", b"\x1b[93;4u", b"\x1b[93:125;4u"),
    ("Ctrl+Shift+-", "C-_", b"\x1b[45;6u", b"\x1b[45:95;6u"),
    ("Ctrl+Shift+/", "C-?", b"\x1b[47;6u", b"\x1b[47:63;6u"),
];

/// The same recording's guards, whose meaning the flags do not change:
/// Ctrl+Shift+a (`C-A` either way: `Chord::new` makes `a` with Shift
/// `A`), Shift+Left and Shift+Tab (the same bytes under both), with
/// what the echo line reads after each.
const FOOT_GUARDS: [(&str, &str, &[u8], &[u8]); 3] = [
    ("Ctrl+Shift+a", "saw C-A", b"\x1b[97;6u", b"\x1b[97:65;6u"),
    ("Shift+Left", "saw S-<left>", b"\x1b[1;2D", b"\x1b[1;2D"),
    (
        "Shift+Tab",
        "S-BackTab: not bound",
        b"\x1b[9;2u",
        b"\x1b[9;2u",
    ),
];

/// A TUI on `notes.txt` whose keymap says which chord reached it, for
/// every chord of [`FOOT`] and the two bound guards.
fn saw_tui(tag: &str) -> ScreenTui {
    use std::fmt::Write as _;
    let dir = std::env::temp_dir().join(format!("pmacs-e537-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let config = dir.join("config");
    std::fs::create_dir_all(config.join("pmacs")).unwrap();
    let mut init = SAW_INIT.to_owned();
    for (i, (_, chord, _, _)) in FOOT.iter().enumerate() {
        let _ = writeln!(init, "saw({chord:?}, \"e537.saw-{i}\")");
    }
    init.push_str("saw(\"C-A\", \"e537.saw-c-a\")\nsaw(\"S-<left>\", \"e537.saw-s-left\")\n");
    std::fs::write(config.join("pmacs/init.lua"), init).unwrap();
    let file = dir.join("notes.txt");
    std::fs::write(&file, "one\ntwo\nthree\n").unwrap();
    let file = file.display().to_string();
    let (pty, tap) = spawn_pmacs_in_pty_tapped(
        &[&file],
        &[
            ("HOME", dir.as_path()),
            ("XDG_CONFIG_HOME", config.as_path()),
        ],
        24,
        100,
    );
    let mut tui = ScreenTui::new(pty, tap, 24, 100);
    tui.wait_for(LIMIT, "the document", &|t| t.screen_contains("three"));
    tui
}

/// The kitty keyboard flags the TUI last pushed (`CSI > flags u`), read
/// from what it wrote to its terminal.
fn pushed_flags(tui: &ScreenTui) -> Option<u32> {
    let out = String::from_utf8_lossy(&tui.pty.output()).into_owned();
    out.rmatch_indices("\x1b[>").find_map(|(at, _)| {
        let rest = &out[at + 3..];
        let end = rest.find('u')?;
        rest[..end].parse().ok()
    })
}

/// The TUI asks a kitty-protocol terminal for the shifted key as well as
/// the disambiguated one: `CSI > 5 u`. At `176a143` it pushed `CSI > 1 u`,
/// so a terminal sent Alt+Shift+/ as `/` with Shift, `M-S-/`.
#[test]
fn e537_the_tui_asks_for_the_shifted_key() {
    let tui = saw_tui("flags");
    assert_eq!(
        pushed_flags(&tui),
        Some(5),
        "the TUI pushes DISAMBIGUATE_ESCAPE_CODES | REPORT_ALTERNATE_KEYS; {}",
        tui.describe()
    );
}

/// Foot's own bytes for each physical chord, as it sends them under the
/// flags this TUI pushed, through the TUI's input path (crossterm, then
/// `Chord::new`) to the keymap: each reaches the chord of its character,
/// `M-?` for Alt+Shift+/, `C-_` (undo) for Ctrl+Shift+-, `C-?` (redo)
/// for Ctrl+Shift+/. At `176a143` the TUI pushed flags 1, foot sent the
/// unshifted key with Shift, and the echo line read `M-S-/: not bound`.
#[test]
fn e537_the_tui_reads_foots_shifted_chords_as_the_character() {
    let mut tui = saw_tui("chords");
    let flags = pushed_flags(&tui).expect("premise: the TUI pushed kitty flags");
    let mut wrong = Vec::new();
    for (keys, chord, under_1, under_5) in FOOT {
        let bytes = if flags & 4 != 0 { under_5 } else { under_1 };
        tui.write(bytes);
        let want = format!("saw {chord}");
        if tui
            .watch_for(Duration::from_secs(3), &|t| t.bottom_row().contains(&want))
            .is_none()
        {
            wrong.push(format!("{keys}: {:?}", tui.bottom_row().trim_end()));
        }
        // A prompt a wrong chord opened (`M-%` at the base's `M-S-5` is
        // unbound, but a default could) must not take the next one.
        tui.write(b"\x07");
    }
    assert!(
        wrong.is_empty(),
        "under flags {flags} the echo line did not name the chord's character: {wrong:#?}"
    );
}

/// The guards: a Ctrl+Shift letter, Shift+Left and Shift+Tab read as
/// before under either flag set.
#[test]
fn e537_the_tui_reads_a_shifted_letter_and_named_keys_as_before() {
    let mut tui = saw_tui("guards");
    let flags = pushed_flags(&tui).expect("premise: the TUI pushed kitty flags");
    let mut wrong = Vec::new();
    for (keys, said, under_1, under_5) in FOOT_GUARDS {
        let bytes = if flags & 4 != 0 { under_5 } else { under_1 };
        tui.write(bytes);
        if tui
            .watch_for(Duration::from_secs(3), &|t| t.bottom_row().contains(said))
            .is_none()
        {
            wrong.push(format!("{keys}: {:?}", tui.bottom_row().trim_end()));
        }
        tui.write(b"\x07");
    }
    assert!(wrong.is_empty(), "under flags {flags}: {wrong:#?}");
}
