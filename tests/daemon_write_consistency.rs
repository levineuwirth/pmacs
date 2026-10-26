//! E10d.3 --- the one thing the daemon's writes are held to: after its
//! bootstrap, a write to an attached session's stream goes through
//! `write_to_session` in `src/daemon.rs`, the one path that ends a
//! session's output at its first failed write and begins its drain.
//!
//! A direct `write_message(` call in `src/daemon.rs`, outside its
//! `#[cfg(test)] mod tests`, is read with the name of the function it
//! sits in, and the functions allowed one are listed in `ALLOWED` with
//! how many each holds: the path itself, and the writes E10d.2 leaves
//! out, each to a stream no attached session owns yet or any more. A
//! call in a function not on the list fails by file, line and function,
//! and so does a listed function holding more calls than its count, so
//! a ninth route cannot bypass the path unseen even inside a function
//! that is allowed one; a listed function holding fewer fails too, so
//! the list shrinks with the code instead of outliving it.
//!
//! One shape is read and nothing is parsed: a line containing
//! `write_message(` that is not a comment and not the `use` that
//! imports it, and the innermost `fn <name>` line above it.

use std::path::PathBuf;

/// Every function in `src/daemon.rs` allowed a direct `write_message`,
/// with how many it holds and why it is not the path's.
const ALLOWED: &[(&str, usize, &str)] = &[
    (
        "write_to_session",
        1,
        "the one path: a failed write takes the stream and begins the drain",
    ),
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
         to a provisional session that a failed write unregisters",
    ),
    (
        "send_buffer_snapshots",
        1,
        "the bootstrap's sweep to a provisional session, which stops at a failed \
         write and fails the bootstrap",
    ),
    (
        "handle_dispatcher_event",
        1,
        "the Goodbye of a detach the frontend's quit asked for, at its teardown",
    ),
    (
        "dispatcher_loop",
        2,
        "the shutdown's Goodbyes, as the dispatcher stops",
    ),
];

fn daemon_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/daemon.rs");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The name a line defines, if it is a `fn` line: `fn name(`,
/// `pub fn name<`, `pub(crate) fn name(`, at any indentation.
fn defined_fn(line: &str) -> Option<&str> {
    let mut rest = line.trim_start();
    if let Some(after) = rest.strip_prefix("pub") {
        rest = after
            .trim_start_matches(|c: char| c != ' ')
            .strip_prefix(' ')?;
    }
    let rest = rest.strip_prefix("fn ")?;
    let end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_'))?;
    (end > 0).then(|| &rest[..end])
}

/// Every direct `write_message(` call in `source` before its test
/// module, as (function, 1-based line).
fn direct_writes(source: &str) -> Vec<(String, usize)> {
    let mut current: Option<String> = None;
    let mut out = Vec::new();
    for (i, line) in source.lines().enumerate() {
        if line == "mod tests {" {
            break;
        }
        if let Some(name) = defined_fn(line) {
            current = Some(name.to_owned());
        }
        let code = line.trim_start();
        if code.starts_with("//") || code.starts_with("use ") {
            continue;
        }
        if code.contains("write_message(") {
            out.push((
                current.clone().unwrap_or_else(|| "<no fn>".to_owned()),
                i + 1,
            ));
        }
    }
    out
}

/// What is wrong with `source`'s direct writes against [`ALLOWED`], one
/// line per problem; empty when the daemon keeps to the path.
fn problems(source: &str) -> Vec<String> {
    let writes = direct_writes(source);
    let mut problems = Vec::new();
    for (function, line) in &writes {
        if !ALLOWED.iter().any(|(name, _, _)| name == function) {
            problems.push(format!(
                "src/daemon.rs:{line} writes to a stream directly in `{function}`, which may \
                 not: a write to an attached session's stream goes through \
                 `write_to_session`, so its first failure ends the session's output"
            ));
        }
    }
    for (name, allowed, why) in ALLOWED {
        let lines: Vec<usize> = writes
            .iter()
            .filter(|(function, _)| function == name)
            .map(|(_, line)| *line)
            .collect();
        if lines.len() != *allowed {
            problems.push(format!(
                "`{name}` is allowed {allowed} direct write(s) ({why}) and holds {} at \
                 src/daemon.rs lines {lines:?}: a new one goes through `write_to_session`, \
                 and a count the code no longer holds is corrected here",
                lines.len()
            ));
        }
    }
    problems
}

#[test]
fn every_write_to_an_attached_session_goes_through_the_one_path() {
    let problems = problems(&daemon_source());
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The guard's own bite, in the source as it stands: a direct write
/// planted in `confirm_cursor_early`, the route E10d.2 moved onto the
/// path last, fails it by that function's name and the planted line;
/// planted in a function the list allows, it fails on the count.
#[test]
fn a_direct_write_planted_outside_the_path_fails_the_guard() {
    let source = daemon_source();
    let plant = |anchor: &str| -> String {
        assert_eq!(
            source.matches(anchor).count(),
            1,
            "the anchor {anchor:?} occurs once in src/daemon.rs"
        );
        source.replacen(
            anchor,
            &format!("{anchor}\n    let _ = write_message(&mut planted, &planted_message);"),
            1,
        )
    };

    let outside = plant("fn confirm_cursor_early(");
    let found = problems(&outside);
    assert!(
        found
            .iter()
            .any(|p| p.contains("directly in `confirm_cursor_early`")),
        "a direct write in confirm_cursor_early fails the guard by name: {found:?}"
    );

    let inside = plant("fn dispatcher_loop(");
    let found = problems(&inside);
    assert!(
        found.iter().any(
            |p| p.contains("`dispatcher_loop` is allowed 2 direct write(s)")
                && p.contains("holds 3")
        ),
        "a third direct write in dispatcher_loop fails the guard on its count: {found:?}"
    );
}
