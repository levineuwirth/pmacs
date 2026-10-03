//! The worker binary itself, driven through its protocol (E7i.2): what the
//! editor spawns per buffer answers `Hello` with this build's protocol,
//! parses, answers a read of the tree it installed while a newer parse is
//! queued behind it, and exits when its input closes. Being an integration
//! test of this crate, it also makes `cargo test --workspace` build the
//! `pmacs-parse-unit` binary the editor's own tests spawn.

use std::io::BufReader;
use std::process::{Command, Stdio};

use pmacs_parse_unit::{
    ParseCall, Request, Response, TextUpdate, WORKER_ENV, read_frame, write_frame,
};

fn parse(text: &[u8]) -> ParseCall {
    ParseCall {
        language: "rust".into(),
        text: TextUpdate::Full,
        edits: Vec::new(),
        expect_len: text.len() as u32,
        aliases: Vec::new(),
        deadline_ms: Some(5_000),
        interest: vec![(0, 64)],
    }
}

#[test]
fn the_worker_answers_hello_a_parse_and_a_read_then_exits_at_eof() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_pmacs-parse-unit"))
        .envs(WORKER_ENV.iter().copied())
        .args(["--memory-limit-mb", "256"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn the worker");
    let mut stdin = child.stdin.take().expect("stdin");
    let mut stdout = BufReader::new(child.stdout.take().expect("stdout"));
    let mut ask = |id: u64, request: Request, payload: &[u8]| {
        write_frame(&mut stdin, &(id, &request), payload).expect("write");
    };
    let mut answer = || -> (u64, Response) {
        loop {
            let ((id, response), _) = read_frame::<_, (u64, Response)>(&mut stdout)
                .expect("read")
                .expect("a frame");
            if id != 0 {
                return (id, response);
            }
        }
    };
    ask(1, Request::Hello, &[]);
    assert!(
        matches!(answer(), (1, Response::Hello { protocol }) if protocol == pmacs_parse_unit::PROTOCOL),
        "Hello names this build's protocol"
    );
    let text = b"fn main() {\n    let x = 1;\n}\n";
    ask(2, Request::Parse(parse(text)), text);
    let (id, parsed) = answer();
    let Response::Parsed(parsed) = parsed else {
        panic!("the parse installed: {parsed:?}");
    };
    assert_eq!(id, 2);
    // A second parse, then a read of the first tree: both answered.
    ask(3, Request::Parse(parse(text)), text);
    ask(
        4,
        Request::Describe {
            generation: parsed.generation,
            path: Vec::new(),
            children: true,
        },
        &[],
    );
    let mut answers = vec![answer(), answer()];
    answers.sort_by_key(|(id, _)| *id);
    assert!(
        matches!(answers[0], (3, Response::Parsed(_))),
        "{answers:?}"
    );
    match &answers[1] {
        (4, Response::Nodes(nodes)) => {
            assert_eq!(nodes[0].kind, "source_file");
            assert_eq!(nodes.len(), 2, "the root and its one function");
        }
        other => panic!("the read was answered: {other:?}"),
    }
    drop(stdin);
    let status = child.wait().expect("wait");
    assert!(
        status.success(),
        "the worker exits cleanly at EOF: {status:?}"
    );
}

/// A worker never outlives its editor (E7i review 1, Medium 2). The editor
/// alone holds its stdin, so the end of stdin is the editor's end, however
/// it came; the worker exits then, abandoning a parse still running, since
/// the deadline that bounded the parse was the editor's. Here the parse is
/// #301's opener series at forty openers with no deadline, which does not
/// return for minutes, and stdin closes 300 ms into it.
#[test]
fn the_worker_exits_when_its_input_closes_mid_parse() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_pmacs-parse-unit"))
        .envs(WORKER_ENV.iter().copied())
        .args(["--memory-limit-mb", "1024"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn the worker");
    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let text = format!("*bar**\n{}f*bark]", "f![".repeat(40));
    let call = ParseCall {
        language: "markdown".into(),
        deadline_ms: None,
        ..parse(text.as_bytes())
    };
    write_frame(&mut stdin, &(1u64, &Request::Parse(call)), text.as_bytes()).expect("write");
    std::thread::sleep(std::time::Duration::from_millis(300));
    let closed = std::time::Instant::now();
    drop(stdin);
    let mut exited = None;
    while closed.elapsed() < std::time::Duration::from_secs(5) {
        if let Some(status) = child.try_wait().expect("try_wait") {
            exited = Some((status, closed.elapsed()));
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if exited.is_none() {
        let _ = child.kill();
    }
    // Reaps it if still running; returns the status `try_wait` saw if not.
    let _ = child.wait();
    let mut answers = Vec::new();
    let mut out = BufReader::new(stdout);
    while let Ok(Some(((id, response), _))) = read_frame::<_, (u64, Response)>(&mut out) {
        answers.push((id, response));
    }
    let (status, after) = exited.expect("the worker exited within 5 s of its input closing");
    assert!(
        after < std::time::Duration::from_secs(1),
        "the worker exited at once, not when its parse finished: {after:?}"
    );
    assert!(status.success(), "{status:?}");
    assert!(
        !answers
            .iter()
            .any(|(id, r)| *id == 1 && matches!(r, Response::Parsed(_) | Response::Failed(_))),
        "the parse was still running and was abandoned, not answered: {answers:?}"
    );
}
