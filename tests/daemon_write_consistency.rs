//! E10d.3 --- the one thing the daemon's writes are held to: after its
//! bootstrap, a write to an attached session's stream goes through
//! `write_to_session` in `src/daemon.rs`, the one path that ends a
//! session's output at its first socket error and begins its drain.
//!
//! Since E10d review 1's Low 2 the compiler holds most of it: a
//! `Session`'s stream is private to `src/presence.rs`, and the registry
//! reaches it only in functions that hand it out to no one ---
//! `write_output`, the path's write, `write_goodbye`, a teardown's, and
//! the ones that install, shut and test for it --- so daemon code that
//! writes an attached session's stream past the path does not compile.
//! This file holds what the compiler cannot, each list by function and
//! count, a function not on its list failing by file, line and name and
//! a listed function holding more or fewer than its count failing too,
//! so a list shrinks with the code instead of outliving it:
//!
//! - [`STREAM`]: the functions in `src/presence.rs` that touch a
//!   session's `.stream`, so an accessor handing the stream out (the
//!   bypass review 1 planted needed one) fails here; the field stays
//!   private, which [`public_stream_field`] holds;
//! - [`WRITERS`]: the calls in `src/daemon.rs` of the registry's two
//!   writers, so nothing writes past `write_to_session`'s log line and
//!   the editor's count of draining frontends, and a `Goodbye` is
//!   written only at a teardown;
//! - [`DIRECT`]: every direct `write_message(` call in `src/daemon.rs`,
//!   each to a stream no attached session owns yet: the handshake's and
//!   the bootstrap's.
//!
//! One shape is read and nothing is parsed: a line holding the needle
//! that is not a comment and not a `use`, before the file's
//! `mod tests {`, and the innermost `fn <name>` line above it.

#[path = "common/source_scan.rs"]
mod source_scan;

use source_scan::{found, source};

/// One file's list: what is looked for and the functions allowed it,
/// with how many lines each holds and why.
struct List {
    file: &'static str,
    needle: &'static str,
    what: &'static str,
    allowed: &'static [(&'static str, usize, &'static str)],
}

/// The functions in `src/presence.rs` that touch a session's stream.
const STREAM: List = List {
    file: "src/presence.rs",
    needle: ".stream",
    what: "touches a session's stream",
    allowed: &[
        (
            "has_stream",
            1,
            "whether a stream is installed and no write to it has failed",
        ),
        ("fmt", 1, "the session's Debug, which prints it"),
        (
            "write_output",
            2,
            "the one path's write, which takes the stream at a socket error",
        ),
        (
            "write_goodbye",
            1,
            "a teardown's Goodbye, its failure ignored",
        ),
        (
            "install_stream",
            1,
            "the bootstrap's handover once its own writes are done",
        ),
        (
            "shut_stream",
            1,
            "a failed bootstrap's or a requested departure's shutdown",
        ),
    ],
};

/// The calls in `src/daemon.rs` of the registry's writers.
const WRITERS: [List; 2] = [
    List {
        file: "src/daemon.rs",
        needle: ".write_output(",
        what: "writes an attached session's stream",
        allowed: &[(
            "write_to_session",
            1,
            "the one path: it logs the route's line and counts the drain",
        )],
    },
    List {
        file: "src/daemon.rs",
        needle: ".write_goodbye(",
        what: "writes a Goodbye to an attached session",
        allowed: &[
            (
                "dispatcher_loop",
                2,
                "the shutdown's Goodbyes, as the dispatcher stops",
            ),
            (
                "handle_dispatcher_event",
                1,
                "the Goodbye of a detach the frontend's quit asked for, at its teardown",
            ),
        ],
    },
];

/// Every function in `src/daemon.rs` allowed a direct `write_message`,
/// with how many it holds and why it is not the path's.
const DIRECT: List = List {
    file: "src/daemon.rs",
    needle: "write_message(",
    what: "writes to a stream directly",
    allowed: &[
        (
            "per_attach_thread",
            5,
            "the handshake's Hello and refusals, before a session exists",
        ),
        (
            "send_initial_target_failure",
            1,
            "a refused attach target, before a session exists",
        ),
        (
            "handle_session_established",
            4,
            "the bootstrap's target and resumed snapshots, its result and its token, \
             to its own socket before the session takes the stream",
        ),
        (
            "send_buffer_snapshots",
            1,
            "the bootstrap's sweep to its own socket, which stops at a failed \
             write and fails the bootstrap",
        ),
    ],
};

/// What is wrong with `source`, the text of `list.file`, against
/// `list`, one line per problem; empty when it keeps to the list.
fn problems_in(list: &List, source: &str) -> Vec<String> {
    let lines = found(source, list.needle);
    let mut problems = Vec::new();
    for (function, line) in &lines {
        if !list.allowed.iter().any(|(name, _, _)| name == function) {
            problems.push(format!(
                "{}:{line} {} in `{function}`, which may not: a write to an attached \
                 session's stream goes through `write_to_session`, so its first socket \
                 error ends the session's output",
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
                 {} at lines {at:?}: a new one goes through `write_to_session`, and a count \
                 the code no longer holds is corrected here",
                list.file,
                list.what,
                at.len()
            ));
        }
    }
    problems
}

/// The line declaring `Session`'s stream with a visibility, if any: a
/// public field would let daemon code reach the stream past every list
/// here.
fn public_stream_field(presence: &str) -> Option<String> {
    presence
        .lines()
        .take_while(|line| *line != "mod tests {")
        .map(str::trim_start)
        .find(|code| {
            code.strip_prefix("pub").is_some_and(|after| {
                after
                    .trim_start_matches(|c: char| c != ' ')
                    .trim_start()
                    .starts_with("stream:")
            })
        })
        .map(str::to_owned)
}

/// Everything wrong with the two files as given.
fn problems(presence: &str, daemon: &str) -> Vec<String> {
    let mut all = problems_in(&STREAM, presence);
    if let Some(line) = public_stream_field(presence) {
        all.push(format!(
            "src/presence.rs declares `{line}`: a session's stream is private to the \
             registry, so daemon code cannot write it past the path"
        ));
    }
    for list in &WRITERS {
        all.extend(problems_in(list, daemon));
    }
    all.extend(problems_in(&DIRECT, daemon));
    all
}

#[test]
fn every_write_to_an_attached_session_goes_through_the_one_path() {
    let problems = problems(&source("src/presence.rs"), &source("src/daemon.rs"));
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// Plant `line` after the one line of `source` holding `anchor`.
fn plant(source: &str, anchor: &str, line: &str) -> String {
    assert_eq!(
        source.matches(anchor).count(),
        1,
        "the anchor {anchor:?} occurs once"
    );
    source.replacen(anchor, &format!("{anchor}\n{line}"), 1)
}

/// The guard's own bite, in the sources as they stand: a direct write
/// planted in `confirm_cursor_early`, the route E10d.2 moved onto the
/// path last, fails it by that function's name and the planted line;
/// planted in a function the list allows, it fails on the count; a call
/// of the path's own writer planted beside it fails by name too.
#[test]
fn a_direct_write_planted_outside_the_path_fails_the_guard() {
    let presence = source("src/presence.rs");
    let daemon = source("src/daemon.rs");
    let write = "    let _ = write_message(&mut planted, &planted_message);";

    let outside = plant(&daemon, "fn confirm_cursor_early(", write);
    let found = problems(&presence, &outside);
    assert!(
        found
            .iter()
            .any(|p| p.contains("writes to a stream directly in `confirm_cursor_early`")),
        "a direct write in confirm_cursor_early fails the guard by name: {found:?}"
    );

    let inside = plant(&daemon, "fn handle_session_established(", write);
    let found = problems(&presence, &inside);
    assert!(
        found.iter().any(|p| p
            .contains("`handle_session_established` in src/daemon.rs is allowed 4 line(s)")
            && p.contains("holds 5")),
        "a fifth direct write in handle_session_established fails the guard on its count: \
         {found:?}"
    );

    let writer = plant(
        &daemon,
        "fn confirm_cursor_early(",
        "    let _ = session_registry.write_output(source, &planted_message);",
    );
    let found = problems(&presence, &writer);
    assert!(
        found
            .iter()
            .any(|p| p.contains("writes an attached session's stream in `confirm_cursor_early`")),
        "the registry's writer called past write_to_session fails the guard by name: \
         {found:?}"
    );
}

/// E10d review 1 (Low 2), adapted at fix round 1 to what then held. The
/// review planted, in `confirm_cursor_early`,
/// `let _ = send_buffer_snapshots(editor,
/// session_registry.stream_mut(source).expect("an attached session's
/// stream"));`: an attached session's stream handed to a writer the
/// list allowed, past the one path, so its failure began no drain. It
/// compiled and passed the guard at `fd38a8a`, which read where a write
/// is spelled, not whose stream it writes. Since fix round 1 it does not
/// compile: the stream is private to `src/presence.rs` and
/// `SessionRegistry::stream_mut` is gone. What the bypass needs is an
/// accessor that hands the stream out, so this row plants the review's
/// line with its accessor restored as it stood at `fd38a8a`, and then
/// with the field made public instead, and the guard fails on each.
#[test]
fn e10d_review1_a_session_stream_handed_to_an_allowed_writer_fails_the_guard() {
    let presence = source("src/presence.rs");
    let daemon = plant(
        &source("src/daemon.rs"),
        "fn confirm_cursor_early(",
        "    let _ = send_buffer_snapshots(editor, \
         session_registry.stream_mut(source).expect(\"an attached session's stream\"));",
    );
    assert!(
        !presence.contains("fn stream_mut("),
        "the accessor the review's plant calls is gone, so the plant does not compile"
    );

    let accessor = plant(
        &presence,
        "impl SessionRegistry {",
        "    pub fn stream_mut(&mut self, frontend_id: FrontendId) -> Option<&mut UnixStream> {\n\
         \x20       self.sessions\n\
         \x20           .get_mut(&frontend_id)\n\
         \x20           .and_then(|session| session.stream.as_mut())\n\
         \x20   }",
    );
    let found = problems(&accessor, &daemon);
    assert!(
        found
            .iter()
            .any(|p| p.contains("touches a session's stream in `stream_mut`")),
        "an accessor handing a session's stream out fails the guard: {found:?}"
    );

    let field = "    stream: Option<UnixStream>,";
    assert_eq!(
        presence.matches(field).count(),
        1,
        "the field {field:?} occurs once"
    );
    let public = presence.replacen(field, "    pub stream: Option<UnixStream>,", 1);
    let found = problems(&public, &daemon);
    assert!(
        found
            .iter()
            .any(|p| p.contains("declares `pub stream: Option<UnixStream>,`")),
        "a public stream field fails the guard: {found:?}"
    );
}
