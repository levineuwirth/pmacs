//! A wire tee around a language server (E7c): the server runs behind
//! a shell wrapper that copies the client's bytes to one file and the
//! server's to another, and a test reads the JSON-RPC frames back
//! with a millisecond of its own choosing. The same instrument C7b's
//! review built inside `tests/e7b_review_wire_acceptance.rs`, made
//! sharable; that suite keeps its own copy, which a later round may
//! point here.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use serde_json::Value;

/// The tee's two capture files and the wrapper that feeds them.
pub struct Capture {
    /// Every byte the client sent, appended.
    pub to_server: PathBuf,
    /// Every byte the server sent, appended.
    pub from_server: PathBuf,
    /// The wrapper script to run as the server's command.
    pub wrapper: PathBuf,
}

/// Write a wrapper in `dir` that runs `server` (a command on `PATH`
/// or a path) with the client's bytes teed to `<dir>/to-server.jsonrpc`
/// and the server's to `<dir>/from-server.jsonrpc`, its stderr to
/// `<dir>/server.stderr`. Needs `sh` and `tee`.
pub fn install(dir: &Path, server: &str) -> Capture {
    let to_server = dir.join("to-server.jsonrpc");
    let from_server = dir.join("from-server.jsonrpc");
    let wrapper = dir.join(format!("{server}-captured"));
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\n\
             tee -a {to} | {server} \"$@\" 2>>{err} | tee -a {from}\n",
            to = shell_quote(&to_server),
            err = shell_quote(&dir.join("server.stderr")),
            from = shell_quote(&from_server),
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    Capture {
        to_server,
        from_server,
        wrapper,
    }
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

/// Every complete JSON-RPC frame in a capture file, in order; a
/// trailing partial frame is ignored.
pub fn frames(path: &Path) -> Vec<Value> {
    parse_frames(&std::fs::read(path).unwrap_or_default()).0
}

/// An incremental reader over one capture file: `new_frames` returns
/// the complete frames appended since the last call and keeps a
/// partial trailing frame for the next.
pub struct Tap {
    path: PathBuf,
    offset: u64,
    pending: Vec<u8>,
}

impl Tap {
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            offset: 0,
            pending: Vec::new(),
        }
    }

    pub fn new_frames(&mut self) -> Vec<Value> {
        use std::io::{Read, Seek, SeekFrom};
        let Ok(mut file) = std::fs::File::open(&self.path) else {
            return Vec::new();
        };
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return Vec::new();
        }
        let mut fresh = Vec::new();
        if file.read_to_end(&mut fresh).is_err() {
            return Vec::new();
        }
        self.offset += fresh.len() as u64;
        self.pending.extend_from_slice(&fresh);
        let (out, consumed) = parse_frames(&self.pending);
        self.pending.drain(..consumed);
        out
    }
}

/// Every complete frame at the front of `bytes`, and how many bytes
/// they took.
pub fn parse_frames(bytes: &[u8]) -> (Vec<Value>, usize) {
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let Some(header_end) = find(&bytes[at..], b"\r\n\r\n") else {
            break;
        };
        let header = String::from_utf8_lossy(&bytes[at..at + header_end]);
        let length = header
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length: "))
            .and_then(|n| n.trim().parse::<usize>().ok())
            .expect("a Content-Length header on every frame");
        let body_start = at + header_end + 4;
        let body_end = body_start + length;
        if body_end > bytes.len() {
            break;
        }
        out.push(serde_json::from_slice(&bytes[body_start..body_end]).expect("a JSON body"));
        at = body_end;
    }
    (out, at)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// A frame's method, or `None` for a response.
pub fn method_of(frame: &Value) -> Option<&str> {
    frame.get("method").and_then(Value::as_str)
}

/// One line for a frame: the method, with a `$/progress` frame's token
/// and kind, a `publishDiagnostics` frame's count, a `didChange`'s
/// version and shape.
pub fn describe(f: &Value) -> String {
    match method_of(f) {
        Some("$/progress") => format!(
            "$/progress {} {}{}",
            f["params"]["token"],
            f["params"]["value"]["kind"].as_str().unwrap_or("?"),
            f["params"]["value"]["title"]
                .as_str()
                .map(|t| format!(" ({t})"))
                .unwrap_or_default()
        ),
        Some("textDocument/publishDiagnostics") => format!(
            "textDocument/publishDiagnostics n={} version={}",
            f["params"]["diagnostics"].as_array().map_or(0, Vec::len),
            f["params"]["version"]
        ),
        Some("textDocument/didChange") => format!(
            "textDocument/didChange version={} {}",
            f["params"]["textDocument"]["version"],
            if f["params"]["contentChanges"]
                .as_array()
                .is_some_and(|c| c.iter().all(|ch| ch.get("range").is_some()))
            {
                "ranged"
            } else {
                "whole"
            }
        ),
        Some(m) => m.to_owned(),
        None => match (f.get("id"), f.get("error")) {
            (Some(id), Some(err)) => format!("<response {id} error {}>", err["code"]),
            (Some(id), None) => format!("<response {id}>"),
            _ => "<response>".to_owned(),
        },
    }
}
