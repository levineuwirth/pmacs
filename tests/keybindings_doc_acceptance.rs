// tests/keybindings_doc_acceptance.rs --- `docs/keybindings.md` is
// generated, not remembered (E1.8).

//! The global and mode keymap tables in `docs/keybindings.md` are
//! rendered from `KeymapStack::iter_all` --- the same source
//! `pmacs.keymap.list()` and `help.list-keybindings` read --- and this
//! suite holds the file to that rendering byte for byte.
//!
//! Why generate at all: the file used to carry a hand-typed "last
//! verified against `main` @ `<sha>`" stamp and a rule asking every PR
//! to update it by hand. It drifted anyway (its own §2 records a `TAB`
//! binding it had missed), and a keymap reference that disagrees with
//! the keymap is worse than none: a reader cannot tell which half is
//! wrong. What cannot be derived --- the Rust-hardcoded modal shadows,
//! the panel keymaps that exist only while a panel does, the terminal
//! caveats --- stays hand-written, outside the markers.
//!
//! To regenerate after changing a binding:
//!
//! ```text
//! PMACS_WRITE_KEYBINDINGS=1 cargo test --test keybindings_doc_acceptance
//! ```

use std::path::PathBuf;

use pmacs::editor::EditorState;

const BEGIN: &str = "<!-- keymap:begin -->";
const END: &str = "<!-- keymap:end -->";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn doc_path() -> PathBuf {
    repo_root().join("docs/keybindings.md")
}

#[path = "common/editor.rs"]
mod editor;
use editor::eval;

/// Every binding the live keymap holds, as `scope\u{1}sequence\u{1}command`.
fn bindings() -> Vec<(String, String, String)> {
    let s = EditorState::new_with_roots(&crate::iso::roots());
    s.lua_host.reopen_init_phase_for_testing();
    let rows: Vec<String> = eval(
        &s,
        "local out = {}
         for _, b in ipairs(pmacs.keymap.list()) do
           out[#out + 1] = b.scope .. '\\1' .. b.sequence .. '\\1' .. b.command
         end
         return out",
    );
    let mut parsed: Vec<(String, String, String)> = rows
        .into_iter()
        .map(|row| {
            let mut parts = row.split('\u{1}');
            let scope = parts.next().unwrap_or_default().to_owned();
            let sequence = parts.next().unwrap_or_default().to_owned();
            let command = parts.next().unwrap_or_default().to_owned();
            (scope, sequence, command)
        })
        .collect();
    parsed.sort();
    parsed
}

/// The markdown between the markers, markers included.
fn render(rows: &[(String, String, String)]) -> String {
    use std::fmt::Write as _;
    let mut out = String::from(BEGIN);
    out.push('\n');
    let mut scope: Option<&str> = None;
    for (row_scope, sequence, command) in rows {
        if scope != Some(row_scope.as_str()) {
            let _ = write!(
                out,
                "\n### Scope: {row_scope}\n\n| Key | Command |\n|---|---|\n"
            );
            scope = Some(row_scope.as_str());
        }
        let _ = writeln!(out, "| `{sequence}` | `{command}` |");
    }
    out.push('\n');
    out.push_str(END);
    out
}

fn block_of(text: &str) -> String {
    let begin = text
        .find(BEGIN)
        .expect("docs/keybindings.md has a keymap:begin marker");
    let end = text
        .find(END)
        .expect("docs/keybindings.md has a keymap:end marker");
    assert!(begin < end, "the markers are out of order");
    text[begin..end + END.len()].to_owned()
}

#[test]
fn the_generated_tables_match_the_live_keymap() {
    let rendered = render(&bindings());
    let current = std::fs::read_to_string(doc_path()).expect("read docs/keybindings.md");

    if std::env::var_os("PMACS_WRITE_KEYBINDINGS").is_some() {
        let existing = block_of(&current);
        let updated = current.replace(&existing, &rendered);
        std::fs::write(doc_path(), updated).expect("write docs/keybindings.md");
        return;
    }

    assert_eq!(
        block_of(&current),
        rendered,
        "docs/keybindings.md is out of date; regenerate with \
         PMACS_WRITE_KEYBINDINGS=1 cargo test --test keybindings_doc_acceptance"
    );
}

/// The generated block is not empty and names a chord this phase bound,
/// so a generator that silently produced nothing could not pass.
#[test]
fn the_generated_tables_are_not_vacuous() {
    let rows = bindings();
    assert!(
        rows.len() > 50,
        "the live keymap has {} bindings",
        rows.len()
    );
    assert!(
        rows.iter()
            .any(|(_, sequence, command)| sequence == "C-x w" && command == "editor.list-workers"),
        "the rendering must carry the bindings the keymap holds"
    );
}

/// README's "first ten minutes" table is hand-written prose --- it says
/// what each key is FOR, which no generator knows --- so the chords it
/// names are held to the live keymap here. A key that stops being bound,
/// or is rebound to something else, fails by name rather than greeting
/// the next reader of the README.
///
/// Only the chords are checked, not the descriptions: the point is that
/// the table cannot advertise a key that does nothing.
#[test]
fn every_chord_the_readme_advertises_is_bound() {
    let readme = std::fs::read_to_string(repo_root().join("README.md")).expect("read README.md");
    let table = readme
        .split_once("### The first ten minutes")
        .expect("README has the first-ten-minutes section")
        .1
        .split_once("\n## ")
        .expect("the section ends at the next heading")
        .0;

    let rows = bindings();
    let bound = |chord: &str| rows.iter().any(|(_, sequence, _)| sequence == chord);

    let mut checked = 0usize;
    for line in table.lines().filter(|line| line.starts_with("| `")) {
        let keys = line
            .trim_start_matches("| ")
            .split_once(" |")
            .expect("a table row has a second column")
            .0;
        // A cell may offer alternatives (`M-<` / `M->`), each in its own
        // backticks. Every one of them must be real.
        for chord in keys.split('`').skip(1).step_by(2) {
            assert!(
                bound(chord),
                "README's first-ten-minutes table advertises {chord:?}, which nothing is bound to"
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 20,
        "the table must actually have been read; {checked} chords checked"
    );
}

/// The words a hand-written passage uses to say a command has no key.
const DENIALS: [&str; 6] = [
    "no global chord",
    "no chord",
    "no binding",
    "no key",
    "not bound",
    "unbound",
];

/// The hand-written passages of `text` that deny a chord to a command
/// the global keymap binds: `(the passage's first line, command,
/// chord)`. A passage is a paragraph outside the generated block; it
/// denies when it holds one of [`DENIALS`], emphasis marks and line
/// breaks aside, and it names a command by putting it in backticks.
fn denied_but_bound(
    text: &str,
    rows: &[(String, String, String)],
) -> Vec<(String, String, String)> {
    let generated = block_of(text);
    let prose = text.replace(&generated, "");
    let mut found = Vec::new();
    for passage in prose.split("\n\n") {
        let plain = passage
            .replace('*', "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        if !DENIALS.iter().any(|denial| plain.contains(denial)) {
            continue;
        }
        for named in passage.split('`').skip(1).step_by(2) {
            for (scope, sequence, command) in rows {
                if scope == "global" && command == named {
                    let first = passage.trim().lines().next().unwrap_or_default();
                    found.push((first.to_owned(), command.clone(), sequence.clone()));
                }
            }
        }
    }
    found
}

/// E10a.9: the hand-written prose may not deny a chord the keymap
/// binds. The generated tables cannot catch this: until E10a.9 the
/// document's own table listed `C-x g` for `git.status` while a
/// sentence below it said the command "gets no global chord", and
/// every row of this suite passed.
#[test]
fn the_prose_denies_no_chord_the_keymap_binds() {
    let rows = bindings();
    let current = std::fs::read_to_string(doc_path()).expect("read docs/keybindings.md");
    let found = denied_but_bound(&current, &rows);
    assert!(
        found.is_empty(),
        "docs/keybindings.md says a command has no chord while the global keymap binds it \
         (passage, command, chord): {found:?}"
    );
}

/// The reader above is not vacuous: given the sentence the document
/// carried, in the document's own frame, it names the command and the
/// chord; and a passage that denies nothing is not reported.
#[test]
fn the_prose_reader_reports_the_sentence_the_document_carried() {
    let rows = bindings();
    let carried = format!(
        "# Keys\n\n{BEGIN}\n| `C-x g` | `git.status` |\n{END}\n\n\
         `git.status` gets **no global chord**: an opening key is a\n\
         command-surface decision the Stage 1 framing did not make.\n\n\
         `git.status` opens the panel; `dired` is on `C-x d`.\n"
    );
    assert_eq!(
        denied_but_bound(&carried, &rows),
        [(
            "`git.status` gets **no global chord**: an opening key is a".to_owned(),
            "git.status".to_owned(),
            "C-x g".to_owned()
        )]
    );
}

// Isolated bootstrap storage roots: an integration test is compiled
// without `cfg(test)`, so a raw `EditorState::new()` would read the
// developer's real `init.lua`.
#[path = "common/iso.rs"]
mod iso;
