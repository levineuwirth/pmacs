//! E10k.3 --- a replica answers a key from bounded reads (#414).
//!
//! Before E10k.3 the attached TUI's `BufferMirror` answered each of its
//! four per-key questions (`prev_char_len`,
//! `cursor_at_end_of_line_safe_for_delete_back`, `cursor_at_end_of_line`,
//! `next_char_len`) by materializing the whole CRDT document, three times
//! a typed key, and the GPU copied the whole text out of its replica when
//! a key's or a remote op's deltas did not patch its projection, or when
//! an import applied ops with no text delta. Now the mirror reads one
//! character of the replica (`CrdtState::char_at_byte`,
//! `char_before_byte`), and the GPU patches from deltas, its one
//! whole-text rebuild a named recovery its failure arms call.
//!
//! What this file holds, read from the sources as they stand with the
//! scanner E10d's guard reads with (`tests/common/source_scan.rs`), each
//! list by function and count so that a list shrinks with the code:
//!
//! - [`MIRROR`]: `materialize_string` in `src/buffer_mirror.rs`, only in
//!   `materialize`, the cold path `SemanticClient` reconstructs from;
//! - [`ATTACH`]: no whole-text read in the attach client's key path
//!   (`src/attach.rs`, `src/optimistic.rs`);
//! - [`GPU`]: a whole-text copy of the GPU's replica only where a whole
//!   document arrives or is rebuilt;
//! - [`RESYNC`]: the rebuild called only from the two per-key functions'
//!   failure arms;
//! - [`EDITS`]: no `insert_utf8` or `delete_utf8` in the replicas' code
//!   or `CrdtState`'s: loro checks a byte position by copying the whole
//!   text before it, which the profile found under every replica's
//!   typed key (the attached client's optimistic insert, the GPU's, the
//!   daemon's edit), so an edit goes in at a character index found by
//!   lookups;
//! - [`CONVERT`]: `convert_pos` called only where it is that lookup,
//!   bytes to characters; the other direction copies the text before
//!   the offset, and the daemon's import converted that way once a key
//!   until E10k.4 (`CrdtState::unicode_to_utf8_pos` now bisects);
//! - [`FORK`]: no `fork()` in `CrdtState`: the daemon's check of a
//!   remote op's peers imported it into a fork of the whole document,
//!   which loro builds by encoding and decoding a snapshot, once a key
//!   (E10k.4, the profile's largest share of the daemon's time);
//! - [`CRDT_PER_KEY`]: no `materialize_string` and no `.to_string()` in
//!   the `CrdtState` functions a key reaches, where the mirror's helpers,
//!   the replicas' edits and the daemon's conversion do their reading
//!   (E10k review 1's Low 2: until fix round 1 no list read them);
//! - [`GPU_PER_KEY`]: no `.to_string()` at all in the GPU's per-key
//!   functions, so a copy through a binding (`text.to_string()`) or one
//!   rustfmt splits over two lines is seen, not only [`GPU`]'s spelling;
//! - and every function named above defined where it is looked for, so
//!   a rename cannot leave the lists holding nothing.

#[path = "common/source_scan.rs"]
mod source_scan;

use source_scan::{defined_fn, found, source};

/// One file's list: the needle, and the functions allowed it with how
/// many lines each holds and why.
struct List {
    file: &'static str,
    needle: &'static str,
    what: &'static str,
    allowed: &'static [(&'static str, usize, &'static str)],
}

const MIRROR: List = List {
    file: "src/buffer_mirror.rs",
    needle: "materialize_string",
    what: "copies the whole replica",
    allowed: &[(
        "materialize",
        1,
        "the cold path: SemanticClient's reconstruction, never a key's",
    )],
};

const ATTACH: [List; 4] = [
    List {
        file: "src/attach.rs",
        needle: "materialize",
        what: "copies the whole replica",
        allowed: &[],
    },
    List {
        file: "src/attach.rs",
        needle: "materialize_string",
        what: "copies the whole replica",
        allowed: &[],
    },
    List {
        file: "src/optimistic.rs",
        needle: "materialize",
        what: "copies the whole replica",
        allowed: &[],
    },
    List {
        file: "src/optimistic.rs",
        needle: "materialize_string",
        what: "copies the whole replica",
        allowed: &[],
    },
];

const GPU: List = List {
    file: "pmacs-gpu/src/main.rs",
    needle: "get_text(LORO_TEXT_CONTAINER).to_string()",
    what: "copies the whole replica",
    allowed: &[
        (
            "observe",
            1,
            "the managed probe's reading of a snapshot it was sent",
        ),
        (
            "apply_attach_message",
            1,
            "a BufferSnapshot's arrival: the whole document, once",
        ),
        (
            "resync_text_from_replica",
            1,
            "the rebuild after an edit's deltas failed to patch the projection",
        ),
    ],
};

const RESYNC: List = List {
    file: "pmacs-gpu/src/main.rs",
    needle: "resync_text_from_replica(",
    what: "rebuilds the projection from the whole replica",
    allowed: &[
        ("resync_text_from_replica", 1, "its own definition"),
        (
            "finish_optimistic_edit",
            2,
            "a key whose edit produced no delta, or whose delta failed to patch",
        ),
        (
            "apply_remote_crdt_op",
            1,
            "a remote op whose delta failed to patch",
        ),
    ],
};

/// The byte-addressed edits, nowhere in the replicas' code.
const EDITS: [List; 6] = [
    List {
        file: "src/crdt.rs",
        needle: "insert_utf8(",
        what: "edits by byte, which loro checks by copying the text before it",
        allowed: &[],
    },
    List {
        file: "src/crdt.rs",
        needle: "delete_utf8(",
        what: "edits by byte, which loro checks by copying the text before it",
        allowed: &[],
    },
    List {
        file: "src/buffer_mirror.rs",
        needle: "insert_utf8(",
        what: "edits by byte, which loro checks by copying the text before it",
        allowed: &[],
    },
    List {
        file: "src/buffer_mirror.rs",
        needle: "delete_utf8(",
        what: "edits by byte, which loro checks by copying the text before it",
        allowed: &[],
    },
    List {
        file: "pmacs-gpu/src/main.rs",
        needle: "insert_utf8(",
        what: "edits by byte, which loro checks by copying the text before it",
        allowed: &[],
    },
    List {
        file: "pmacs-gpu/src/main.rs",
        needle: "delete_utf8(",
        what: "edits by byte, which loro checks by copying the text before it",
        allowed: &[],
    },
];

/// loro's position conversion, only where it is a lookup.
const CONVERT: [List; 2] = [
    List {
        file: "src/crdt.rs",
        needle: "convert_pos(",
        what: "converts a position with loro",
        allowed: &[(
            "unicode_at_byte",
            1,
            "bytes to characters: a walk of loro's tree and a scan of one piece",
        )],
    },
    List {
        file: "pmacs-gpu/src/main.rs",
        needle: "convert_pos(",
        what: "converts a position with loro",
        allowed: &[(
            "loro_char_index_on_boundary",
            1,
            "bytes to characters: a walk of loro's tree and a scan of one piece",
        )],
    },
];

/// A fork of the document, nowhere in `CrdtState`.
const FORK: List = List {
    file: "src/crdt.rs",
    needle: ".fork()",
    what: "copies the whole document into a fork",
    allowed: &[],
};

/// Functions in one file that may hold no line with any of `needles`.
struct Forbidden {
    file: &'static str,
    functions: &'static [&'static str],
    needles: &'static [&'static str],
    what: &'static str,
}

/// `CrdtState`'s per-key functions: what the mirror's four helpers call
/// (`char_at_byte`, `char_before_byte`), the replicas' and the daemon's
/// edits (`insert`, `delete`), the daemon's conversion of an imported
/// key (`unicode_to_utf8_pos`), and the lookups under them.
const CRDT_PER_KEY: Forbidden = Forbidden {
    file: "src/crdt.rs",
    functions: &[
        "char_at_byte",
        "char_before_byte",
        "unicode_at_byte",
        "byte_to_unicode_on_boundary",
        "checked_unicode",
        "insert",
        "delete",
        "unicode_to_utf8_pos",
    ],
    needles: &["materialize_string", ".to_string()"],
    what: "copies the whole text",
};

/// The GPU's per-key functions, held to no `.to_string()` at all.
const GPU_PER_KEY: Forbidden = Forbidden {
    file: "pmacs-gpu/src/main.rs",
    functions: &[
        "optimistic_crdt_insert",
        "optimistic_crdt_delete",
        "finish_optimistic_edit",
        "apply_loro_text_delta_batches",
        "apply_remote_crdt_op",
    ],
    needles: &[".to_string()"],
    what: "copies a string, the whole replica's text through a binding as like as not",
};

/// What is wrong with `text`, the source of `list.file`, against `list`.
fn forbidden_in(list: &Forbidden, text: &str) -> Vec<String> {
    let mut problems = Vec::new();
    for needle in list.needles {
        for (function, line) in found(text, needle) {
            if list.functions.contains(&function.as_str()) {
                problems.push(format!(
                    "{}:{line} {} (`{needle}`) in `{function}`, a function a key reaches: \
                     a key answers from bounded reads (E10k.3, review 1's Low 2)",
                    list.file, list.what
                ));
            }
        }
    }
    for name in list.functions {
        if !text.lines().any(|line| defined_fn(line) == Some(name)) {
            problems.push(format!(
                "{} defines no `{name}`: this guard reads nothing for it",
                list.file
            ));
        }
    }
    problems
}

/// The per-key functions the roadmap names, each in its file.
const PER_KEY: [(&str, &str); 9] = [
    ("src/buffer_mirror.rs", "prev_char_len"),
    (
        "src/buffer_mirror.rs",
        "cursor_at_end_of_line_safe_for_delete_back",
    ),
    ("src/buffer_mirror.rs", "cursor_at_end_of_line"),
    ("src/buffer_mirror.rs", "next_char_len"),
    ("pmacs-gpu/src/main.rs", "optimistic_crdt_insert"),
    ("pmacs-gpu/src/main.rs", "optimistic_crdt_delete"),
    ("pmacs-gpu/src/main.rs", "finish_optimistic_edit"),
    ("pmacs-gpu/src/main.rs", "apply_loro_text_delta_batches"),
    ("pmacs-gpu/src/main.rs", "apply_remote_crdt_op"),
];

/// What is wrong with `text`, the source of `list.file`, against `list`.
fn problems_in(list: &List, text: &str) -> Vec<String> {
    let lines = found(text, list.needle);
    let mut problems = Vec::new();
    for (function, line) in &lines {
        if !list.allowed.iter().any(|(name, _, _)| name == function) {
            problems.push(format!(
                "{}:{line} {} in `{function}`, which may not: a key answers from \
                 bounded reads of the replica (E10k.3)",
                list.file, list.what
            ));
        }
    }
    for (name, allowed, why) in list.allowed {
        let at: Vec<usize> = lines
            .iter()
            .filter(|(function, _)| function == name)
            .map(|(_, line)| *line)
            .collect();
        if at.len() != *allowed {
            problems.push(format!(
                "`{name}` in {} is allowed {allowed} line(s) where it {} ({why}) and holds \
                 {} at lines {at:?}",
                list.file,
                list.what,
                at.len()
            ));
        }
    }
    problems
}

/// Everything wrong with the files, `read` giving each one's text.
fn problems(read: &dyn Fn(&str) -> String) -> Vec<String> {
    let mut all = problems_in(&MIRROR, &read(MIRROR.file));
    for list in &ATTACH {
        all.extend(problems_in(list, &read(list.file)));
    }
    all.extend(problems_in(&GPU, &read(GPU.file)));
    all.extend(problems_in(&RESYNC, &read(RESYNC.file)));
    for list in EDITS.iter().chain(CONVERT.iter()) {
        all.extend(problems_in(list, &read(list.file)));
    }
    all.extend(problems_in(&FORK, &read(FORK.file)));
    for list in [&CRDT_PER_KEY, &GPU_PER_KEY] {
        all.extend(forbidden_in(list, &read(list.file)));
    }
    for (file, name) in PER_KEY {
        let text = read(file);
        if !text.lines().any(|line| defined_fn(line) == Some(name)) {
            all.push(format!(
                "{file} defines no `{name}`: this guard reads nothing for it"
            ));
        }
    }
    all
}

#[test]
fn e10k_3_a_replica_answers_a_key_from_bounded_reads() {
    let problems = problems(&source);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The guard's own bite in the sources as they stand: the base's read
/// planted in `cursor_at_end_of_line` fails it by that helper's name, and
/// a whole-text copy planted in the GPU's import fails it by
/// `apply_remote_crdt_op`'s.
#[test]
fn e10k_3_a_whole_text_read_planted_in_a_per_key_path_fails_the_guard() {
    fn plant(text: &str, anchor: &str, line: &str) -> String {
        assert_eq!(
            text.matches(anchor).count(),
            1,
            "the anchor {anchor:?} occurs once"
        );
        text.replacen(anchor, &format!("{anchor}\n{line}"), 1)
    }
    let mirror = plant(
        &source("src/buffer_mirror.rs"),
        "pub fn cursor_at_end_of_line(&self, buffer_id: BufferId) -> Option<bool> {",
        "        let _content = self.states.get(&buffer_id)?.materialize_string();",
    );
    let found = problems(&|file| {
        if file == "src/buffer_mirror.rs" {
            mirror.clone()
        } else {
            source(file)
        }
    });
    assert!(
        found
            .iter()
            .any(|p| p.contains("copies the whole replica in `cursor_at_end_of_line`")),
        "{found:?}"
    );
    let gpu = plant(
        &source("pmacs-gpu/src/main.rs"),
        "    fn apply_remote_crdt_op(&mut self, buffer_id: BufferId, op: &CrdtOp) -> Option<ViewportSend> {",
        "        let _text = self.loro_doc.as_ref().map(|doc| doc.get_text(LORO_TEXT_CONTAINER).to_string());",
    );
    let found = problems(&|file| {
        if file == "pmacs-gpu/src/main.rs" {
            gpu.clone()
        } else {
            source(file)
        }
    });
    assert!(
        found
            .iter()
            .any(|p| p.contains("copies the whole replica in `apply_remote_crdt_op`")),
        "{found:?}"
    );
}

// E10k review 1, P2: two whole-text reads the structural guard did not
// see, each planted in the per-key path in the sources as they stand and
// asserted named. At `700e86f` both failed, because the guard read only
// the spelling `get_text(LORO_TEXT_CONTAINER).to_string()` in the GPU and
// nothing in `CrdtState`'s per-key functions, which the mirror's four
// helpers call. Committed at fix round 1 with the lists above.

/// P2a: `CrdtState::char_at_byte` is what `cursor_at_end_of_line`,
/// `cursor_at_end_of_line_safe_for_delete_back` and `next_char_len`
/// call; a whole-text copy there is a copy in each of them.
#[test]
fn p2a_a_whole_text_read_in_the_helpers_callee_fails_the_guard() {
    let anchor = "    pub fn char_at_byte(&self, pos: usize) -> Option<char> {";
    let crdt = source("src/crdt.rs");
    assert_eq!(crdt.matches(anchor).count(), 1, "the anchor");
    let planted = crdt.replacen(
        anchor,
        &format!("{anchor}\n        let _whole = self.materialize_string();"),
        1,
    );
    let found = problems(&|file| {
        if file == "src/crdt.rs" {
            planted.clone()
        } else {
            source(file)
        }
    });
    assert!(
        !found.is_empty(),
        "a materialize_string planted in CrdtState::char_at_byte passes the guard"
    );
}

/// P2b: the GPU's optimistic insert binds the text container and then
/// reads it; a whole-text copy through that binding is spelled
/// `text.to_string()`, not the guard's needle.
#[test]
fn p2b_a_whole_text_read_through_a_binding_fails_the_guard() {
    let anchor = "        let at = loro_char_index_on_boundary(&text, cursor)?;";
    let gpu = source("pmacs-gpu/src/main.rs");
    assert_eq!(gpu.matches(anchor).count(), 1, "the anchor");
    let planted = gpu.replacen(
        anchor,
        &format!("{anchor}\n        let _whole = text.to_string();"),
        1,
    );
    let found = problems(&|file| {
        if file == "pmacs-gpu/src/main.rs" {
            planted.clone()
        } else {
            source(file)
        }
    });
    assert!(
        !found.is_empty(),
        "a text.to_string() planted in optimistic_crdt_insert passes the guard"
    );
}
