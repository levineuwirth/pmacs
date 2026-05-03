// pmacs_fake_lsp.rs --- Test helper. Tiny LSP-protocol echo peer.

//! Test helper binary, used only by `tests/m4_acceptance.rs` to
//! exercise the M4.5 LSP transport without depending on a real
//! language server being installed.
//!
//! Behaviour:
//!
//! * Reads `Content-Length`-framed JSON-RPC bodies from stdin.
//! * On `initialize`: replies with a minimal capabilities object.
//! * On `initialized`: silent.
//! * On `textDocument/didOpen` or `textDocument/didChange`: echoes
//!   the document URI back as a `pmacs/echo` notification so the
//!   test can verify the wire is alive, and pushes a synthetic
//!   `publishDiagnostics` so M4.6 tests have content to read.
//! * On `textDocument/completion`: returns a deterministic
//!   `CompletionList` with three items so M4.7 tests can read it.
//! * On `textDocument/hover`: returns a `MarkupContent`-shaped
//!   markdown payload.
//! * On `textDocument/signatureHelp`: returns a one-signature
//!   payload with two parameters and the second one active.
//! * On any other request: replies with `result: {"echo": params}`.
//! * On `shutdown`: replies with `null` and waits for `exit`.
//! * On `exit`: exits 0.
//! * If launched with `PMACS_FAKE_LSP_MODE=garbage`: writes
//!   intentionally malformed framing once and exits 0, so the
//!   client can verify protocol-violation handling.
//! * If launched with `PMACS_FAKE_LSP_MODE=crash`: replies to
//!   `initialize`, then exits with code 7 immediately, so the
//!   client can verify crash + restart handling.

use std::io::{self, Read, Write};

#[allow(
    clippy::too_many_lines,
    reason = "linear LSP-method dispatch; splitting a test helper this much fragments the read"
)]
fn main() {
    let mode = std::env::var("PMACS_FAKE_LSP_MODE").unwrap_or_default();
    if mode == "garbage" {
        write_garbage();
        return;
    }
    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    let mut crashed_after_init = false;
    loop {
        let body = match read_frame(&mut stdin) {
            Ok(Some(b)) => b,
            Ok(None) => return, // EOF
            Err(e) => {
                eprintln!("frame read error: {e}");
                return;
            }
        };
        let msg: serde_json::Value = match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("bad json: {e}");
                continue;
            }
        };
        let method = msg
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();
        let id = msg.get("id").cloned();
        let params = msg
            .get("params")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        match (method.as_str(), id) {
            ("initialize", Some(idv)) => {
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": idv,
                    "result": {
                        "capabilities": {
                            "textDocumentSync": 1,
                            "hoverProvider": true,
                            "completionProvider": { "triggerCharacters": ["."] },
                            "definitionProvider": true,
                            "documentFormattingProvider": true,
                            "diagnosticProvider": { "interFileDependencies": false, "workspaceDiagnostics": false }
                        },
                        "serverInfo": { "name": "pmacs-fake-lsp", "version": "0.1.0" }
                    }
                });
                write_frame(&mut stdout, &resp);
                if mode == "crash" {
                    crashed_after_init = true;
                }
            }
            ("initialized", _) => {}
            ("shutdown", Some(idv)) => {
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": idv,
                    "result": serde_json::Value::Null
                });
                write_frame(&mut stdout, &resp);
            }
            ("exit", _) => return,
            ("textDocument/completion", Some(idv)) => {
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": idv,
                    "result": {
                        "isIncomplete": false,
                        "items": [
                            {
                                "label": "println",
                                "kind": 3,
                                "detail": "macro println!",
                                "documentation": {
                                    "kind": "markdown",
                                    "value": "Prints to stdout with a newline."
                                },
                                "insertText": "println!"
                            },
                            {
                                "label": "print",
                                "kind": 3,
                                "detail": "macro print!",
                                "insertText": "print!"
                            },
                            {
                                "label": "panic",
                                "kind": 3,
                                "detail": "macro panic!",
                                "insertText": "panic!"
                            }
                        ]
                    }
                });
                write_frame(&mut stdout, &resp);
            }
            ("textDocument/hover", Some(idv)) => {
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": idv,
                    "result": {
                        "contents": {
                            "kind": "markdown",
                            "value": "# pmacs-fake-lsp\n\nSynthetic hover content for the symbol under cursor."
                        },
                        "range": {
                            "start": { "line": 0, "character": 0 },
                            "end":   { "line": 0, "character": 4 }
                        }
                    }
                });
                write_frame(&mut stdout, &resp);
            }
            ("textDocument/signatureHelp", Some(idv)) => {
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": idv,
                    "result": {
                        "signatures": [
                            {
                                "label": "fn echo(name: &str, count: usize) -> String",
                                "documentation": "Echoes `name` `count` times.",
                                "parameters": [
                                    { "label": "name: &str" },
                                    { "label": "count: usize" }
                                ],
                                "activeParameter": 1
                            }
                        ],
                        "activeSignature": 0,
                        "activeParameter": 1
                    }
                });
                write_frame(&mut stdout, &resp);
            }
            ("textDocument/didOpen" | "textDocument/didChange", _) => {
                let uri = params
                    .get("textDocument")
                    .and_then(|t| t.get("uri"))
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let echo = serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "pmacs/echo",
                    "params": {
                        "method": method,
                        "uri": uri,
                    }
                });
                write_frame(&mut stdout, &echo);
                // Also push a synthetic `publishDiagnostics`
                // notification with two entries (one Error, one
                // Warning) so M4.6 tests can exercise the store.
                if uri.is_string() {
                    let diags = serde_json::json!({
                        "jsonrpc": "2.0",
                        "method": "textDocument/publishDiagnostics",
                        "params": {
                            "uri": uri,
                            "diagnostics": [
                                {
                                    "range": {
                                        "start": { "line": 0, "character": 4 },
                                        "end":   { "line": 0, "character": 8 },
                                    },
                                    "severity": 1,
                                    "message": "synthetic error",
                                    "source": "pmacs-fake-lsp",
                                    "code": "E0001"
                                },
                                {
                                    "range": {
                                        "start": { "line": 2, "character": 0 },
                                        "end":   { "line": 2, "character": 5 },
                                    },
                                    "severity": 2,
                                    "message": "synthetic warning",
                                    "source": "pmacs-fake-lsp",
                                    "code": "W0001"
                                }
                            ]
                        }
                    });
                    write_frame(&mut stdout, &diags);
                }
            }
            ("textDocument/definition", Some(idv)) => {
                let uri = params
                    .get("textDocument")
                    .and_then(|t| t.get("uri"))
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": idv,
                    "result": [{
                        "uri": uri,
                        "range": {
                            "start": { "line": 7, "character": 4 },
                            "end":   { "line": 7, "character": 9 }
                        }
                    }]
                });
                write_frame(&mut stdout, &resp);
            }
            ("textDocument/formatting", Some(idv)) => {
                // Synthetic two-edit reply: trim leading whitespace on
                // line 0 and append a semicolon at line 3, col 7.
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": idv,
                    "result": [
                        {
                            "range": {
                                "start": { "line": 0, "character": 0 },
                                "end":   { "line": 0, "character": 4 }
                            },
                            "newText": ""
                        },
                        {
                            "range": {
                                "start": { "line": 3, "character": 7 },
                                "end":   { "line": 3, "character": 7 }
                            },
                            "newText": ";"
                        }
                    ]
                });
                write_frame(&mut stdout, &resp);
            }
            (_, Some(idv)) => {
                // Generic echo response.
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": idv,
                    "result": { "echo": params, "method": method }
                });
                write_frame(&mut stdout, &resp);
            }
            _ => {}
        }
        if crashed_after_init {
            // Honour the test's request to die after the first
            // useful exchange.
            std::process::exit(7);
        }
    }
}

fn read_frame<R: Read>(r: &mut R) -> io::Result<Option<Vec<u8>>> {
    // Read header byte-by-byte until \r\n\r\n, then read body.
    let mut header = Vec::new();
    let mut window = [0u8; 4];
    let mut filled = 0usize;
    loop {
        let mut byte = [0u8; 1];
        match r.read(&mut byte) {
            Ok(0) => return Ok(None),
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
        header.push(byte[0]);
        if filled < 4 {
            window[filled] = byte[0];
            filled += 1;
        } else {
            window.copy_within(1..4, 0);
            window[3] = byte[0];
        }
        if &window == b"\r\n\r\n" {
            break;
        }
    }
    let header_str =
        std::str::from_utf8(&header).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut content_length: Option<usize> = None;
    for line in header_str.split("\r\n") {
        if line.is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse().ok();
            }
        }
    }
    let n = content_length
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length"))?;
    let mut body = vec![0u8; n];
    r.read_exact(&mut body)?;
    Ok(Some(body))
}

fn write_frame<W: Write>(w: &mut W, body: &serde_json::Value) {
    let bytes = serde_json::to_vec(body).expect("json serialize");
    let _ = write!(w, "Content-Length: {}\r\n\r\n", bytes.len());
    let _ = w.write_all(&bytes);
    let _ = w.flush();
}

fn write_garbage() {
    let mut stdout = io::stdout().lock();
    let _ = stdout.write_all(b"NotAValidLspFrame\r\nGarbageHeader\r\n\r\n{}");
    let _ = stdout.flush();
}
