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
//! * If launched with `PMACS_FAKE_LSP_MODE=rooturi`: writes the
//!   `rootUri` received in `initialize` to the file named by
//!   `PMACS_FAKE_LSP_ROOT_SINK`, so a test can assert the
//!   auto-attach path derives the project root from the opened file.
//! * If launched with `PMACS_FAKE_LSP_MODE=fullonly`: advertises a
//!   full-only `semanticTokensProvider` (`"full": true`, no delta
//!   member) and rejects `semanticTokens/full/delta` with a JSON-RPC
//!   error — a conforming full-only server, for testing that the
//!   client never requests delta without the negotiated capability.
//! * If launched with `PMACS_FAKE_LSP_MODE=rangeonly`: advertises a
//!   range-only `semanticTokensProvider` (`"range": true`, no `full`)
//!   and rejects `semanticTokens/full` — per LSP, `full` and `range`
//!   are optional, independent capabilities.
//! * If launched with `PMACS_FAKE_LSP_MODE=rangeonly16`: `rangeonly`
//!   plus UTF-16 position encoding, with strict UTF-16 bounds
//!   validation on `/range` — rejects a client that sent raw byte
//!   columns for non-ASCII text.
//! * If launched with `PMACS_FAKE_LSP_MODE=sighelp`: additionally
//!   advertises `signatureHelpProvider` with `(` / `,` triggers, so a
//!   test can drive the Arc 1d auto-trigger. Every other mode omits the
//!   capability and therefore never auto-triggers.
//! * If `PMACS_FAKE_LSP_CHANGE_SINK` names a file (any mode): appends
//!   one `{"method", "text"}` JSON line per received didOpen /
//!   didChange, so a test can replay the exact document-sync sequence
//!   the server saw — the auto-pairing Q#AP7 ordering observable
//!   ("the first didChange after `(` carries `()`").

use std::collections::HashMap;
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
    let mut open_docs: HashMap<String, String> = HashMap::new();
    // `fullonly` observability: counts /full responses (rid-1, rid-2…).
    let mut full_count: u32 = 0;
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
        // T M4.5 `wsconfig`: the client's reply to the
        // `workspace/configuration` request we sent at `initialized`
        // arrives here as a response (id 9001, has `result`, no
        // method). Echo its result array back as a notification so
        // the test can assert what pmacs answered.
        if mode == "wsconfig"
            && method.is_empty()
            && msg.get("result").is_some()
            && id.as_ref().and_then(serde_json::Value::as_u64) == Some(9001)
        {
            let echo = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "pmacs/wsconfig",
                "params": { "answer": msg.get("result").cloned() }
            });
            write_frame(&mut stdout, &echo);
            continue;
        }
        // T M4.5 async-bridge failure-path test modes:
        //  * `error`  — answer every `textDocument/*` request with a
        //    JSON-RPC error object (drives `Handle:await()` -> failed).
        //  * `silent` — accept the request but never answer it while
        //    staying alive (drives the request-timeout sweep, and
        //    makes supersede deterministic: a superseded handle can
        //    only settle via cancellation, never racing a response).
        if method.starts_with("textDocument/") {
            if mode == "error" && id.is_some() {
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id.clone(),
                    "error": { "code": -32603, "message": "synthetic error" }
                });
                write_frame(&mut stdout, &resp);
                continue;
            }
            if mode == "silent" {
                continue;
            }
        }
        match (method.as_str(), id) {
            ("initialize", Some(idv)) => {
                let mut resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": idv,
                    "result": {
                        "capabilities": {
                            "textDocumentSync": 1,
                            "hoverProvider": true,
                            "completionProvider": { "triggerCharacters": ["."] },
                            "definitionProvider": true,
                            "inlayHintProvider": true,
                            "documentFormattingProvider": true,
                            "diagnosticProvider": { "interFileDependencies": false, "workspaceDiagnostics": false },
                            "semanticTokensProvider": {
                                "legend": {
                                    "tokenTypes": ["namespace", "function", "variable"],
                                    "tokenModifiers": ["declaration", "readonly"]
                                },
                                // The default mode implements /full/delta, so
                                // it truthfully NEGOTIATES delta. Clients may
                                // only send /full/delta when `full` is
                                // `{ "delta": true }`; a bare `true` (the
                                // `fullonly` override below) is full-only.
                                "full": { "delta": true }
                            }
                        },
                        "serverInfo": { "name": "pmacs-fake-lsp", "version": "0.1.0" }
                    }
                });
                // T M4.5 Option B: `posecho` negotiates UTF-16 and
                // echoes request positions back (see definition arm)
                // so a test can prove the byte↔UTF-16 round-trip.
                if mode == "posecho" {
                    resp["result"]["capabilities"]["positionEncoding"] =
                        serde_json::Value::from("utf-16");
                }
                // T M4.5: advertise prepareRename only in the
                // prepare-* modes, so the default `rename` mode keeps
                // exercising the no-prepare path.
                if mode == "prepare" || mode == "preprefuse" {
                    resp["result"]["capabilities"]["renameProvider"] =
                        serde_json::json!({ "prepareProvider": true });
                }
                // `fullonly`: a conforming FULL-ONLY semantic-token
                // server — advertises `"full": true` (no delta member)
                // and REJECTS /full/delta below. Exercises the client
                // rule that a stored resultId alone must never cause a
                // delta request.
                if mode == "fullonly" {
                    resp["result"]["capabilities"]["semanticTokensProvider"]["full"] =
                        serde_json::Value::from(true);
                }
                // `rangeonly`: LSP allows a provider to advertise
                // `range` WITHOUT `full` — the /full arm below rejects
                // in this mode, so a client that ignores the split gets
                // a visible failure instead of silent staleness.
                if mode.starts_with("rangeonly") {
                    let p = &mut resp["result"]["capabilities"]["semanticTokensProvider"];
                    if let Some(obj) = p.as_object_mut() {
                        obj.remove("full");
                        obj.insert("range".into(), serde_json::Value::from(true));
                    }
                }
                // `rangeonly16` additionally negotiates UTF-16, so the
                // /range arm can validate that the client converted its
                // byte columns to UTF-16 code units.
                if mode == "rangeonly16" {
                    resp["result"]["capabilities"]["positionEncoding"] =
                        serde_json::Value::from("utf-16");
                }
                // Arc 1d: advertise signature help only in `sighelp`, so
                // every other mode keeps the no-auto-trigger path (the
                // `textDocument/signatureHelp` arm below still answers
                // the manual `M-x lsp.signature-help` in any mode).
                if mode == "sighelp" {
                    // "«" (U+00AB, 2 UTF-8 bytes) exercises the rule
                    // that LSP trigger characters are strings, not
                    // ASCII bytes.
                    resp["result"]["capabilities"]["signatureHelpProvider"] = serde_json::json!({
                        "triggerCharacters": ["(", "\u{ab}"],
                        "retriggerCharacters": [","]
                    });
                }
                // T M4.5 hardening `rooturi`: record the `rootUri` the
                // client sent in `initialize` to a side-channel file
                // (env `PMACS_FAKE_LSP_ROOT_SINK`). Lets a test prove
                // the auto-attach path derives the project root from
                // the opened file, not the editor's cwd. Mirrors the
                // `filewatch` mode's `.received` disk side-channel.
                if mode == "rooturi"
                    && let Ok(sink) = std::env::var("PMACS_FAKE_LSP_ROOT_SINK")
                {
                    let recorded = params
                        .get("rootUri")
                        .and_then(|v| v.as_str())
                        .unwrap_or("<null>");
                    let _ = std::fs::write(&sink, recorded);
                }
                write_frame(&mut stdout, &resp);
                if mode == "crash" {
                    crashed_after_init = true;
                }
            }
            // T M4.5 `wsconfig`: pull config the way gopls / pyright
            // / clangd do right after initialize.
            ("initialized", _) if mode == "wsconfig" => {
                let req = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 9001,
                    "method": "workspace/configuration",
                    "params": { "items": [
                        { "section": "pmacs.probe" },
                        { "section": "does.not.exist" }
                    ] }
                });
                write_frame(&mut stdout, &req);
            }
            // T M4.5 `inlayrefresh` / `semantictokensrefresh`: right
            // after initialize, signal that cached inlay hints /
            // semantic tokens are stale via the matching server→client
            // refresh request. The client must answer (null) and
            // re-pull the corresponding `textDocument/*`.
            ("initialized", _) if mode == "inlayrefresh" => {
                let req = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 9200,
                    "method": "workspace/inlayHint/refresh",
                    "params": serde_json::Value::Null
                });
                write_frame(&mut stdout, &req);
            }
            ("initialized", _) if mode == "semantictokensrefresh" => {
                let req = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 9201,
                    "method": "workspace/semanticTokens/refresh",
                    "params": serde_json::Value::Null
                });
                write_frame(&mut stdout, &req);
            }
            // T M4.5 `filewatch`: dynamically register a
            // `workspace/didChangeWatchedFiles` watcher (RelativePattern
            // rooted at PMACS_FAKE_LSP_WATCH_BASE, `**/*.txt`, all
            // kinds). The client must reply null and start watching.
            ("initialized", _) if mode == "filewatch" => {
                let base = std::env::var("PMACS_FAKE_LSP_WATCH_BASE").unwrap_or_default();
                let req = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 9300,
                    "method": "client/registerCapability",
                    "params": { "registrations": [{
                        "id": "watch-1",
                        "method": "workspace/didChangeWatchedFiles",
                        "registerOptions": { "watchers": [{
                            "globPattern": {
                                "baseUri": format!("file://{base}"),
                                "pattern": "**/*.txt"
                            },
                            "kind": 7
                        }] }
                    }] }
                });
                write_frame(&mut stdout, &req);
            }
            ("initialized", _) => {}
            // T M4.5: the client's file-watch notifications. Append
            // `type uri` lines to `<base>/.received` as a test
            // side-channel (the protocol stream is drained by the Lua
            // server-request pump, so a disk channel is observable).
            ("workspace/didChangeWatchedFiles", _) => {
                let base = std::env::var("PMACS_FAKE_LSP_WATCH_BASE").unwrap_or_default();
                if !base.is_empty()
                    && let Some(changes) = params.get("changes").and_then(|c| c.as_array())
                    && let Ok(mut f) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(format!("{base}/.received"))
                {
                    use std::io::Write as _;
                    for ch in changes {
                        let t = ch
                            .get("type")
                            .and_then(serde_json::Value::as_i64)
                            .unwrap_or(0);
                        let u = ch
                            .get("uri")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("");
                        let _ = writeln!(f, "{t} {u}");
                    }
                }
            }
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
            ("workspace/didChangeConfiguration", _) => {
                // Record the pushed `settings` so a test can assert the
                // daemon delivered configuration after `initialized` (the
                // push-model config-delivery path push-only servers like the
                // VS Code JSON server rely on). One JSON line per push.
                if let Ok(sink) = std::env::var("PMACS_FAKE_LSP_CONFIG_SINK") {
                    use std::io::Write as _;
                    if let Ok(mut f) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&sink)
                    {
                        let settings = params
                            .get("settings")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null);
                        let _ = writeln!(f, "{settings}");
                    }
                }
            }
            ("textDocument/didOpen" | "textDocument/didChange", _) => {
                let uri = params
                    .get("textDocument")
                    .and_then(|t| t.get("uri"))
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let text = if method == "textDocument/didOpen" {
                    params
                        .get("textDocument")
                        .and_then(|t| t.get("text"))
                        .and_then(serde_json::Value::as_str)
                } else {
                    params
                        .get("contentChanges")
                        .and_then(serde_json::Value::as_array)
                        .and_then(|a| a.first())
                        .and_then(|c| c.get("text"))
                        .and_then(serde_json::Value::as_str)
                };
                if let (Some(uri_s), Some(text)) = (uri.as_str(), text) {
                    open_docs.insert(uri_s.to_owned(), text.to_owned());
                }
                // Auto-pairing Q#AP7: the ordering observable is "the
                // FIRST didChange after `(` carries `()`" — provable
                // only from what the server actually received, in
                // order. Mirror of `PMACS_FAKE_LSP_ROOT_SINK`: append
                // one JSON line per didOpen/didChange to the sink
                // file so a test can replay the exact sequence.
                if let (Ok(sink), Some(text)) = (std::env::var("PMACS_FAKE_LSP_CHANGE_SINK"), text)
                {
                    use std::io::Write as _;
                    if let Ok(mut f) = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&sink)
                    {
                        let line = serde_json::json!({ "method": method, "text": text });
                        let _ = writeln!(f, "{line}");
                    }
                }
                let echo = serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "pmacs/echo",
                    "params": {
                        "method": method,
                        "uri": uri,
                    }
                });
                write_frame(&mut stdout, &echo);
                // Arc 8 Stage 3b: `leanprogress` mode emits one
                // `$/lean/fileProgress` covering line 0, so the Lean
                // subscriber can be pinned end-to-end through the real
                // drain rather than by calling its handler directly.
                if mode == "leanprogress" && uri.is_string() {
                    let progress = serde_json::json!({
                        "jsonrpc": "2.0",
                        "method": "$/lean/fileProgress",
                        "params": {
                            "textDocument": { "uri": uri, "version": 1 },
                            "processing": [{
                                "range": {
                                    "start": { "line": 0, "character": 0 },
                                    "end":   { "line": 1, "character": 0 }
                                },
                                "kind": 1
                            }]
                        }
                    });
                    write_frame(&mut stdout, &progress);
                }
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
                // `posecho`: echo the request's own position back as
                // the range (so the stored byte offset round-trips iff
                // encode∘decode is correct) AND stamp the *wire*
                // `character` the client sent into the result `uri` as
                // `pos:N` (so the test can see the intermediate UTF-16
                // value and prove the outbound encode was non-identity;
                // the `uri` string is not a Position so the client's
                // inbound rewrite leaves it untouched).
                let (uri, range) = if mode == "posecho" {
                    let pos = params
                        .get("position")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    let ch = pos
                        .get("character")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    (
                        serde_json::Value::from(format!("pos:{ch}")),
                        serde_json::json!({ "start": pos, "end": pos }),
                    )
                } else if mode == "defenv" {
                    // T M4.5 L1 cross-file: point the definition at a
                    // *different* file URI supplied via env, so the
                    // client must decode the URI, open-or-reuse that
                    // buffer, and reposition (SP-4 Gap A path).
                    let target = std::env::var("PMACS_FAKE_LSP_DEF_URI").unwrap_or_default();
                    (
                        serde_json::Value::from(target),
                        serde_json::json!({
                            "start": { "line": 2, "character": 0 },
                            "end":   { "line": 2, "character": 3 }
                        }),
                    )
                } else {
                    (
                        uri,
                        serde_json::json!({
                            "start": { "line": 7, "character": 4 },
                            "end":   { "line": 7, "character": 9 }
                        }),
                    )
                };
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": idv,
                    "result": [{ "uri": uri, "range": range }]
                });
                write_frame(&mut stdout, &resp);
            }
            // T M4.5 Location-shaped nav. Distinct line per method so
            // a test can confirm each routes into its own kind slot.
            (
                m @ ("textDocument/references"
                | "textDocument/declaration"
                | "textDocument/typeDefinition"
                | "textDocument/implementation"),
                Some(idv),
            ) => {
                let uri = params
                    .get("textDocument")
                    .and_then(|t| t.get("uri"))
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let line = match m {
                    "textDocument/references" => 11,
                    "textDocument/declaration" => 21,
                    "textDocument/typeDefinition" => 31,
                    _ => 41, // implementation
                };
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": idv,
                    "result": [{
                        "uri": uri,
                        "range": {
                            "start": { "line": line, "character": 2 },
                            "end":   { "line": line, "character": 6 }
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
            ("textDocument/prepareRename", Some(idv)) => {
                // `posecho` negotiates UTF-16. Validate the request
                // position before replying so the position-codec test
                // below catches byte-column regressions in this builder.
                if mode == "posecho"
                    && let Some(message) = utf16_position_error(&params, &open_docs)
                {
                    let resp = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": idv,
                        "error": { "code": -32602, "message": message }
                    });
                    write_frame(&mut stdout, &resp);
                    continue;
                }
                // T M4.5: `preprefuse` → null (not renameable here);
                // otherwise the `{ range, placeholder }` shape over
                // the line-0 cols 3..6 span ("foo").
                let result = if mode == "preprefuse" {
                    serde_json::Value::Null
                } else {
                    serde_json::json!({
                        "range": {
                            "start": { "line": 0, "character": 3 },
                            "end":   { "line": 0, "character": 6 }
                        },
                        "placeholder": "foo"
                    })
                };
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": idv,
                    "result": result
                });
                write_frame(&mut stdout, &resp);
            }
            ("textDocument/rename", Some(idv)) => {
                // Same UTF-16 validation as prepareRename: rename and
                // prepareRename both carry a single Position.
                if mode == "posecho"
                    && let Some(message) = utf16_position_error(&params, &open_docs)
                {
                    let resp = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": idv,
                        "error": { "code": -32602, "message": message }
                    });
                    write_frame(&mut stdout, &resp);
                    continue;
                }
                // T M4.5 L2: reply with a `WorkspaceEdit`. The edit
                // replaces the 3-char span at line 0, cols 3..6 with
                // the requested `newName` (so the test can assert the
                // buffer text changed). In `rename` mode a *second*
                // file URI is taken from `PMACS_FAKE_LSP_RENAME_URI`
                // and given the same edit, plus a `create` resource
                // op — exercising the cross-file applier and the
                // unsupported-op count. Otherwise the edit is
                // single-file (the request's own document).
                let uri = params
                    .get("textDocument")
                    .and_then(|t| t.get("uri"))
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let new_name = params
                    .get("newName")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("renamed")
                    .to_owned();
                let edit = serde_json::json!([{
                    "range": {
                        "start": { "line": 0, "character": 3 },
                        "end":   { "line": 0, "character": 6 }
                    },
                    "newText": new_name
                }]);
                let workspace_edit = if mode == "rename" {
                    let second = std::env::var("PMACS_FAKE_LSP_RENAME_URI").unwrap_or_default();
                    serde_json::json!({
                        "documentChanges": [
                            {
                                "textDocument": { "uri": uri, "version": 1 },
                                "edits": edit.clone()
                            },
                            {
                                "textDocument": { "uri": second, "version": 1 },
                                "edits": edit.clone()
                            }
                        ]
                    })
                } else {
                    let mut changes = serde_json::Map::new();
                    changes.insert(uri.as_str().unwrap_or("").to_owned(), edit);
                    serde_json::json!({ "changes": changes })
                };
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": idv,
                    "result": workspace_edit
                });
                write_frame(&mut stdout, &resp);
            }
            ("textDocument/codeAction", Some(idv)) => {
                // T M4.5 L3: two actions — one with an inline edit
                // (replace line-0 cols 3..6 with "ED1"), one that is
                // command-only (the client must `executeCommand` it,
                // and we then drive the change via a server→client
                // `applyEdit`). The command carries the document URI
                // as its argument so the executeCommand arm knows
                // what to edit.
                let uri = params
                    .get("textDocument")
                    .and_then(|t| t.get("uri"))
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let mut changes = serde_json::Map::new();
                changes.insert(
                    uri.as_str().unwrap_or("").to_owned(),
                    serde_json::json!([{
                        "range": {
                            "start": { "line": 0, "character": 3 },
                            "end":   { "line": 0, "character": 6 }
                        },
                        "newText": "ED1"
                    }]),
                );
                // Command action first so a "apply the first action"
                // client drives the executeCommand→applyEdit path;
                // the inline-edit action second still exercises the
                // CodeAction.edit normalisation in the store.
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": idv,
                    "result": [
                        {
                            "title": "Run server command",
                            "kind": "refactor",
                            "command": {
                                "title": "Run",
                                "command": "pmacs.fake.applyEdit",
                                "arguments": [uri]
                            }
                        },
                        {
                            "title": "Inline fix",
                            "kind": "quickfix",
                            "edit": { "changes": changes }
                        }
                    ]
                });
                write_frame(&mut stdout, &resp);
            }
            ("workspace/executeCommand", Some(idv)) => {
                // T M4.5 L3: the real edit is delivered out of band
                // via a server→client `workspace/applyEdit` request
                // (id 9100), exactly as rust-analyzer et al. do.
                // Replace line-1 cols 0..3 with "ED2". Then answer
                // the original executeCommand with a null result; the
                // client's reply to 9100 lands in the default arm and
                // is ignored.
                let cmd = params
                    .get("command")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                if cmd == "pmacs.fake.applyEdit" {
                    let target = params
                        .get("arguments")
                        .and_then(serde_json::Value::as_array)
                        .and_then(|a| a.first())
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    // T M4.5 L4 `resourceops`: deliver an ordered
                    // documentChanges that creates a file, fills it
                    // (create-before-edit ordering), renames a
                    // sibling, and deletes another — paths derived
                    // from the request URI's directory so the test
                    // doesn't have to thread them through env.
                    let we = if mode == "resourceops" {
                        let s = target.as_str().unwrap_or("");
                        let base = match s.rfind('/') {
                            Some(i) => &s[..=i],
                            None => "",
                        };
                        let created = format!("{base}created.rs");
                        let b = format!("{base}b.rs");
                        let b2 = format!("{base}b2.rs");
                        let c = format!("{base}c.rs");
                        serde_json::json!({
                            "documentChanges": [
                                { "kind": "create", "uri": created },
                                {
                                    "textDocument": { "uri": created, "version": 1 },
                                    "edits": [{
                                        "range": {
                                            "start": { "line": 0, "character": 0 },
                                            "end":   { "line": 0, "character": 0 }
                                        },
                                        "newText": "NEW"
                                    }]
                                },
                                { "kind": "rename", "oldUri": b, "newUri": b2 },
                                { "kind": "delete", "uri": c }
                            ]
                        })
                    } else {
                        serde_json::json!({
                            "documentChanges": [{
                                "textDocument": { "uri": target, "version": 1 },
                                "edits": [{
                                    "range": {
                                        "start": { "line": 1, "character": 0 },
                                        "end":   { "line": 1, "character": 3 }
                                    },
                                    "newText": "ED2"
                                }]
                            }]
                        })
                    };
                    let apply = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 9100,
                        "method": "workspace/applyEdit",
                        "params": { "label": "fake refactor", "edit": we }
                    });
                    write_frame(&mut stdout, &apply);
                }
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": idv,
                    "result": serde_json::Value::Null
                });
                write_frame(&mut stdout, &resp);
            }
            ("textDocument/semanticTokens/full", Some(idv)) => {
                // `rangeonly`: a range-only provider rejects /full —
                // the client should have sent a range request.
                if mode.starts_with("rangeonly") {
                    let resp = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": idv,
                        "error": {
                            "code": -32601,
                            "message": "semanticTokens/full not supported"
                        }
                    });
                    write_frame(&mut stdout, &resp);
                    continue;
                }
                // `fullonly`: bump the resultId per request so a test
                // can observe WHICH pull refreshed the store — a
                // repull that wrongly went to /full/delta is rejected
                // and leaves the previous rid in place.
                if mode == "fullonly" {
                    full_count += 1;
                    let resp = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": idv,
                        "result": {
                            "resultId": format!("rid-{full_count}"),
                            "data": [0, 0, 4, 1, 1, 0, 5, 3, 2, 0, 2, 2, 7, 0, 2]
                        }
                    });
                    write_frame(&mut stdout, &resp);
                    continue;
                }
                // T M4.5: relative-encoded `data`. Three tokens:
                //   [0,0,4,1,1]  line 0 col 0 len 4, function, decl
                //   [0,5,3,2,0]  same line col 5 len 3, variable
                //   [2,2,7,0,2]  +2 lines col 2 len 7, namespace, ro
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": idv,
                    "result": {
                        "resultId": "rid-1",
                        "data": [0, 0, 4, 1, 1, 0, 5, 3, 2, 0, 2, 2, 7, 0, 2]
                    }
                });
                write_frame(&mut stdout, &resp);
            }
            ("textDocument/semanticTokens/range", Some(idv)) => {
                // `rangeonly16`: strict bounds validation in UTF-16
                // units. A client that sent raw byte columns for
                // non-ASCII text overshoots the last line's UTF-16
                // length and is rejected — the fixture for the
                // outbound-position conversion.
                if mode == "rangeonly16"
                    && let Ok(sink) = std::env::var("PMACS_FAKE_RANGE_SINK")
                {
                    let _ = std::fs::write(&sink, format!("{params}"));
                }
                if mode == "rangeonly16"
                    && let Some(message) = utf16_range_error(&params, &open_docs)
                {
                    let resp = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": idv,
                        "error": { "code": -32602, "message": message }
                    });
                    write_frame(&mut stdout, &resp);
                    continue;
                }
                // T M4.5: same shape as /full, scoped to a range.
                // One token: line 1 col 0 len 3, variable.
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": idv,
                    "result": { "resultId": "rid-range", "data": [1, 0, 3, 2, 0] }
                });
                write_frame(&mut stdout, &resp);
            }
            ("textDocument/semanticTokens/full/delta", Some(idv)) => {
                // `fullonly`: a conforming full-only server rejects a
                // delta request outright — the client should never have
                // sent it (capabilities advertised `"full": true` with
                // no delta member).
                if mode == "fullonly" {
                    let resp = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": idv,
                        "error": {
                            "code": -32601,
                            "message": "semanticTokens/full/delta not supported"
                        }
                    });
                    write_frame(&mut stdout, &resp);
                    continue;
                }
                // T M4.5: a `SemanticTokensDelta` over the /full data
                // `[0,0,4,1,1, 0,5,3,2,0, 2,2,7,0,2]` — replace the
                // last 5-int group (idx 10..15) with [3,0,9,1,0], so
                // token 3 becomes line 3 col 0 len 9, function.
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": idv,
                    "result": {
                        "resultId": "rid-2",
                        "edits": [
                            { "start": 10, "deleteCount": 5, "data": [3, 0, 9, 1, 0] }
                        ]
                    }
                });
                write_frame(&mut stdout, &resp);
            }
            ("textDocument/inlayHint", Some(idv)) => {
                if mode == "inlaybounds"
                    && let Some(message) = inlay_range_error(&params, &open_docs)
                {
                    let resp = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": idv,
                        "error": { "code": -32603, "message": message }
                    });
                    write_frame(&mut stdout, &resp);
                    continue;
                }
                // T M4.5: a type hint (string label, kind 1) and a
                // parameter hint (label *parts*, kind 2) so both
                // label shapes are exercised.
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": idv,
                    "result": [
                        {
                            "position": { "line": 0, "character": 9 },
                            "label": ": i32",
                            "kind": 1,
                            "paddingLeft": false,
                            "paddingRight": false,
                            "tooltip": "inferred type"
                        },
                        {
                            "position": { "line": 1, "character": 4 },
                            "label": [ { "value": "count" }, { "value": ":" } ],
                            "kind": 2,
                            "paddingLeft": false,
                            "paddingRight": true
                        }
                    ]
                });
                write_frame(&mut stdout, &resp);
            }
            // T M4.5 symbols/highlight. documentSymbol returns the
            // *hierarchical* DocumentSymbol shape (exercises tree
            // flatten + depth + parent); workspace/symbol the flat
            // SymbolInformation shape (exercises location.uri);
            // documentHighlight a two-occurrence list.
            ("textDocument/documentSymbol", Some(idv)) => {
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": idv,
                    "result": [{
                        "name": "Outer", "kind": 5,
                        "range": { "start": { "line": 1, "character": 0 }, "end": { "line": 9, "character": 0 } },
                        "selectionRange": { "start": { "line": 1, "character": 6 }, "end": { "line": 1, "character": 11 } },
                        "children": [{
                            "name": "inner", "kind": 6,
                            "range": { "start": { "line": 3, "character": 2 }, "end": { "line": 5, "character": 2 } },
                            "selectionRange": { "start": { "line": 3, "character": 7 }, "end": { "line": 3, "character": 12 } }
                        }]
                    }]
                });
                write_frame(&mut stdout, &resp);
            }
            ("workspace/symbol", Some(idv)) => {
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": idv,
                    "result": [{
                        "name": "WsThing", "kind": 12,
                        "location": {
                            "uri": "file:///ws.rs",
                            "range": { "start": { "line": 7, "character": 3 }, "end": { "line": 7, "character": 10 } }
                        },
                        "containerName": "modw"
                    }]
                });
                write_frame(&mut stdout, &resp);
            }
            ("textDocument/documentHighlight", Some(idv)) => {
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": idv,
                    "result": [
                        { "range": { "start": { "line": 2, "character": 4 }, "end": { "line": 2, "character": 9 } }, "kind": 2 },
                        { "range": { "start": { "line": 6, "character": 0 }, "end": { "line": 6, "character": 5 } } }
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
        if let Some((k, v)) = line.split_once(':')
            && k.trim().eq_ignore_ascii_case("content-length")
        {
            content_length = v.trim().parse().ok();
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

fn document_end_position(text: &str) -> (u64, u64) {
    let mut line = 0;
    let mut col = 0;
    for byte in text.bytes() {
        if byte == b'\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// `rangeonly16` bounds validation: the request's end position must not
/// exceed the document end measured in UTF-16 code units (the
/// negotiated encoding). Byte-column overshoot on non-ASCII text is
/// exactly the client bug this catches.
fn utf16_range_error(
    params: &serde_json::Value,
    open_docs: &HashMap<String, String>,
) -> Option<String> {
    // Fail-CLOSED: a fixture that silently skips validation on an
    // unexpected state (missing uri / unrecorded doc) reads as a pass.
    let Some(uri) = params
        .get("textDocument")
        .and_then(|t| t.get("uri"))
        .and_then(serde_json::Value::as_str)
    else {
        return Some("range request carried no textDocument.uri".into());
    };
    let Some(text) = open_docs.get(uri) else {
        return Some(format!("no didOpen text recorded for {uri}"));
    };
    let (mut line, mut col) = (0u64, 0u64);
    for ch in text.chars() {
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += ch.len_utf16() as u64;
        }
    }
    let end = params.get("range")?.get("end")?;
    let end_line = end.get("line")?.as_u64()?;
    let end_col = end.get("character")?.as_u64()?;
    if end_line > line || (end_line == line && end_col > col) {
        Some(format!(
            "invalid utf-16 range end {end_line}:{end_col}; document ends at {line}:{col}"
        ))
    } else {
        None
    }
}

/// `posecho` validation for single-position requests. Unlike the
/// whole-document range fixture, this checks the requested line itself
/// so an overlarge byte column cannot hide behind a later line.
fn utf16_position_error(
    params: &serde_json::Value,
    open_docs: &HashMap<String, String>,
) -> Option<String> {
    let Some(uri) = params
        .get("textDocument")
        .and_then(|t| t.get("uri"))
        .and_then(serde_json::Value::as_str)
    else {
        return Some("position request carried no textDocument.uri".into());
    };
    let Some(text) = open_docs.get(uri) else {
        return Some(format!("no didOpen text recorded for {uri}"));
    };
    let Some(position) = params.get("position") else {
        return Some("position request carried no position".into());
    };
    let Some(line) = position.get("line").and_then(serde_json::Value::as_u64) else {
        return Some("position request carried no numeric line".into());
    };
    let Some(col) = position
        .get("character")
        .and_then(serde_json::Value::as_u64)
    else {
        return Some("position request carried no numeric character".into());
    };
    let Ok(line_index) = usize::try_from(line) else {
        return Some(format!("invalid utf-16 position line {line}"));
    };
    let Some(line_text) = text.split('\n').nth(line_index) else {
        return Some(format!("invalid utf-16 position line {line}"));
    };
    let max_col = line_text.chars().map(char::len_utf16).sum::<usize>() as u64;
    if col > max_col {
        Some(format!(
            "invalid utf-16 position {line}:{col}; line ends at {line}:{max_col}"
        ))
    } else {
        None
    }
}

fn inlay_range_error(
    params: &serde_json::Value,
    open_docs: &HashMap<String, String>,
) -> Option<String> {
    let uri = params
        .get("textDocument")
        .and_then(|t| t.get("uri"))
        .and_then(serde_json::Value::as_str)?;
    let text = open_docs.get(uri)?;
    let (last_line, last_col) = document_end_position(text);
    let end = params.get("range")?.get("end")?;
    let line = end.get("line")?.as_u64()?;
    let col = end.get("character")?.as_u64()?;
    if line > last_line || (line == last_line && col > last_col) {
        Some(format!(
            "invalid inlay range end {line}:{col}; document ends at {last_line}:{last_col}"
        ))
    } else {
        None
    }
}
