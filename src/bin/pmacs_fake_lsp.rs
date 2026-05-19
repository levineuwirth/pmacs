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
                            "documentFormattingProvider": true,
                            "diagnosticProvider": { "interFileDependencies": false, "workspaceDiagnostics": false },
                            "semanticTokensProvider": {
                                "legend": {
                                    "tokenTypes": ["namespace", "function", "variable"],
                                    "tokenModifiers": ["declaration", "readonly"]
                                },
                                "full": true
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
            ("textDocument/rename", Some(idv)) => {
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
            ("textDocument/inlayHint", Some(idv)) => {
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
