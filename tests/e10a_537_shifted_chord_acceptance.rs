// tests/e10a_537_shifted_chord_acceptance.rs --- #537: a chord on a
// character the layout shifts reads as the character, in both frontends.

//! On a US layout `M-?` is Alt+Shift+/, `C-_` Ctrl+Shift+- and `C-x ^`
//! `C-x` then Shift+6. The GPU sent each character the layout made with
//! the Shift that made it still set, and `Chord::new` strips Shift from
//! letters alone, so the daemon read `M-S-?`, `C-S-_` (redo, not undo)
//! and `C-x S-^`. The GPU now drops Shift from a character key, and
//! reads what a legacy terminal and Emacs read.
//!
//! The GPU's row presses the physical chords through its production
//! `App` against a real daemon, the keymap reached named on the band;
//! the unit rows beside `translate_key` read every documented chord off
//! the wire.

mod common;
#[path = "support/mod.rs"]
mod support;

/// A command per chord, each saying which chord reached it, bound over
/// the defaults (`C-_` and `C-?` are undo and redo there; a bound
/// sequence is unbound first, since `bind` refuses one).
#[cfg(feature = "crdt")]
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
