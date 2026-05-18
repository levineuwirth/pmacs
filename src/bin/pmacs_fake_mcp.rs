// pmacs_fake_mcp.rs --- Test helper. Tiny MCP-protocol echo peer.

//! Test helper binary, used by `tests/m9_1_acceptance.rs` to exercise
//! the M9.1 MCP transport without depending on a real MCP server
//! being installed.
//!
//! Mirrors the shape of [`pmacs_fake_lsp`], but speaks
//! newline-delimited JSON-RPC 2.0 (the MCP-stdio framing) instead of
//! `Content-Length`-framed bodies. One JSON message per line; no
//! `\n` inside a body.
//!
//! Behaviour:
//!
//! * Reads newline-delimited JSON-RPC bodies from stdin.
//! * On `initialize`: replies with a minimal capabilities object
//!   advertising `resources`, `tools`, and `prompts` so M9.5–M9.7
//!   have a credible shape to grow into.
//! * On `notifications/initialized`: silent.
//! * On `ping`: replies with `result: {}` (the MCP no-op heartbeat).
//! * On `pmacs/echo`: replies with `result: { "echo": params }` so
//!   tests can verify the wire is alive in both directions.
//! * On any other request: replies with `result: { "echo": params,
//!   "method": method }`.
//! * On `shutdown`: replies with `null`.
//! * On `exit` (notification or EOF): exits 0.
//! * If launched with `PMACS_FAKE_MCP_MODE=garbage`: writes a
//!   non-JSON line and exits 0, so the client can verify
//!   protocol-violation handling.
//! * If launched with `PMACS_FAKE_MCP_MODE=crash`: replies to
//!   `initialize`, then exits with code 7 immediately, so the
//!   client can verify crash + restart handling (mirrors fake-lsp).
//! * If launched with `PMACS_FAKE_MCP_MODE=init_error`: replies to
//!   `initialize` with a JSON-RPC error rather than a result.
//!   Verifies Pass-2 finding 2 (failed initialize must not
//!   transition the client to Initialized).
//! * If launched with `PMACS_FAKE_MCP_MODE=bad_version`: replies
//!   to `initialize` with `protocolVersion = "1999-01-01"`. Verifies
//!   Pass-2 finding 3 (unsupported version is rejected).
//! * If launched with `PMACS_FAKE_MCP_MODE=clean_exit_after_init`:
//!   replies to `initialize` and exits 0. Verifies Pass-2 finding 5
//!   (clean exit + `OnCrash` policy does not restart).
//! * If launched with `PMACS_FAKE_MCP_MODE=missing_caps`: replies
//!   to `initialize` with a valid `protocolVersion` but no
//!   `capabilities` field. Verifies Pass-3 finding 2 (the MCP spec
//!   requires capabilities in the initialize result).
//! * If launched with `PMACS_FAKE_MCP_MODE=crash_on_protocol_shutdown`:
//!   exits with code 99 if it ever sees a `shutdown` method or
//!   `exit` notification on the wire — verifies Pass-3 finding 1
//!   (MCP stdio has no protocol-level shutdown messages; the client
//!   must not send them). Otherwise exits 0 on stdin EOF.
//! * If launched with `PMACS_FAKE_MCP_MODE=ignore_eof_sleep`: replies
//!   normally, then sleeps forever after stdin EOF so the client can
//!   verify live shutdown escalation.
//! * If launched with `PMACS_FAKE_MCP_MODE=crash_after_first_request`:
//!   replies to `initialize` then exits with code 77 on the first
//!   non-handshake request (typically `resources/read`). Used by
//!   M9.2's in-flight-failure test to verify the cache machinery
//!   propagates the failure to every concurrent awaiter.
//!
//! For `resources/read`: the fake increments a per-process counter
//! and returns synthetic text containing it, so M9.2 tests can
//! distinguish "cache hit" (same text on repeat) from "refetch"
//! (different text after invalidation).
//!
//! * If launched with `PMACS_FAKE_MCP_MODE=slow_resources_read`:
//!   `resources/read` sleeps 250ms before responding. Used by the
//!   per-sibling cancellation test to widen the in-flight window so
//!   the test can cancel one awaiter before the response arrives.
//!
//! For `tools/call` (T M9.3):
//!
//! * `name = "echo"` returns `{ content: [{ type: "text", text: "<echo>: <input>" }], isError: false }`.
//! * `name = "fail"` returns `{ content: [{ type: "text", text: "synthetic tool failure" }], isError: true }` —
//!   the MCP spec's "tool errored at semantic level" path.
//! * Any other `name` returns a JSON-RPC `error: -32602 unknown tool: <name>`.
//!
//! For `tools/list` (T M9.6):
//!
//! * Replies with `{ tools: [<entry>, ...] }` from a live, mutable
//!   list seeded by `initial_tool_list`. The `tools.listChanged`
//!   capability is advertised so M9.6 packages know to subscribe.
//! * `name = "nullary"` is a no-required-arg tool (immediate-dispatch
//!   coverage).
//! * `name = "mcp_test/greet"` requires `{ name, greeting }` and
//!   exercises the multi-prompt flow.
//! * `name = "mcp_test/add_tool"`, `name = "mcp_test/remove_tool"`,
//!   `name = "mcp_test/change_tool_schema"` mutate the live tool list
//!   and emit `notifications/tools/list_changed` so the
//!   reconciliation test can drive deterministic dispatcher events.
//! * `name = "typed_int"`, `"typed_number"`, `"typed_bool"` advertise
//!   non-string scalar types in their inputSchema; the tools/call
//!   handler echoes back the JSON kind it received so the M9.6
//!   typed-arg-coercion test can assert the package coerced a
//!   minibuffer string into the schema-declared shape before sending.
//!
//! * If launched with `PMACS_FAKE_MCP_MODE=slow_tools_call`:
//!   `tools/call` sleeps 250ms before responding. Mirrors
//!   `slow_resources_read`; used by the cancellation-reaches-server
//!   test.
//!
//! * If `PMACS_FAKE_MCP_CANCEL_DIR` is set, on receiving
//!   `notifications/cancelled` the fake writes an empty sentinel
//!   file `${PMACS_FAKE_MCP_CANCEL_DIR}/cancelled-<request_id>`.
//!   Tests poll for the file's existence to verify the cancellation
//!   reached the server.
//!
//! For `prompts/get` (T M9.4 + T M9.7):
//!
//! * `name = "code_review"` requires `{ language, source }` arguments.
//!   Returns `messages` referencing the args so round-trip tests can
//!   verify they were threaded through. Missing either arg → JSON-RPC
//!   `error: -32602 missing required argument: <name>`.
//! * `name = "simple"` takes no arguments. Returns a single
//!   user-role message.
//! * `name = "code_demo"` (T M9.7): no args. Returns a Rust code
//!   sample with `_meta.format = "code"` and `_meta.language = "rust"`
//!   so the M9.7 fixture package routes the buffer through the rust
//!   tree-sitter highlight path.
//! * `name = "markdown_demo"` (T M9.7): no args. Returns a structured
//!   markdown document with `_meta.format = "markdown"` so the
//!   package routes through the markdown highlight path.
//! * `name = "multi_message"` (T M9.7): no args. Returns three
//!   messages (system, user, assistant) so the role-header rendering
//!   path is exercised.
//! * `name = "unknown_format"` (T M9.7): no args. Returns
//!   `_meta.format = "this-is-not-a-recognized-format"` so the
//!   unknown-format-falls-back-to-text test can pin the warning path.
//! * `name = "mixed_content"` (T M9.7): no args. Returns one text
//!   content entry plus one image content entry so the placeholder-
//!   rendering test can verify non-text content surfaces as a
//!   `[image: <mimeType>]` line rather than being silently dropped.
//! * `name = "code_unknown_lang"` (T M9.7): no args. Returns
//!   `_meta.format = "code"` with `_meta.language = "klingon"` so the
//!   unknown-grammar fallback test can verify the package pcalls the
//!   dispatch+attach pair and falls back to text rendering with a
//!   warning rather than letting an "unknown language" error escape.
//! * `name = "markdown_inline"` (T M9.7 floor test): no args.
//!   Returns markdown body with inline syntax (`**bold**`, `_em_`,
//!   `[link](url)`, `` `inline code` ``) that the block-only grammar
//!   can't highlight. Pins the floor that block-level highlighting
//!   without inline coverage is acceptable today and any future
//!   inline coverage is additive, not a regression.
//! * Any other `name` returns JSON-RPC `error: -32602 unknown prompt: <name>`.
//!
//! For `prompts/list` (T M9.7):
//!
//! * Replies with `{ prompts: [<entry>, ...] }` from a live, mutable
//!   `BTreeMap` seeded by `initial_prompt_list`. The
//!   `prompts.listChanged` capability is advertised so M9.7 packages
//!   know to subscribe.
//! * `mcp_test/{add,remove,change_prompt}_prompt` tools (T M9.7)
//!   mutate the live list and emit `notifications/prompts/list_changed`
//!   — same shape as M9.6's `mcp_test/{add,remove,change_schema}_tool`
//!   so the M9.7 reconciliation test runs the same drill.
//!
//! * If `PMACS_FAKE_MCP_PROMPT_RECORD_DIR` is set, every
//!   `prompts/get` request's params object is written to
//!   `${PMACS_FAKE_MCP_PROMPT_RECORD_DIR}/prompt-<request_id>.json`
//!   so tests can verify the wire shape of `arguments` (e.g.
//!   `{}` vs `null` vs omitted for the no-args case).
//!
//! For `resources/*` (T M9.5):
//!
//! * `resources/subscribe` and `resources/unsubscribe` requests
//!   record the subscription server-side. The server then emits
//!   `notifications/resources/updated` for that uri whenever its
//!   stored content changes.
//! * The fake stores per-uri content in a side table; `resources/read`
//!   for known synthetic uris returns the stored content (URI-keyed
//!   text, mimeType, and optional `contents` shape for directory
//!   listings).
//! * Three synthetic resources are pre-populated:
//!     - `mcp://text/doc.txt` — text/plain
//!     - `mcp://text/readme.md` — text/markdown
//!     - `mcp://dir/` — directory containing the two text uris
//! * Tool `mcp_test/trigger_update { uri, new_text }` updates the
//!   stored content for `uri` to `new_text`, then emits
//!   `notifications/resources/updated { uri }`. M9.5's
//!   subscription test uses this to deterministically drive
//!   server-pushed updates.

use std::io::{self, BufRead, Write};

/// T M9.7: build the initial advertised prompt list. Same shape as
/// M9.6's `initial_tool_list` but for `prompts/list`. Each entry is
/// the MCP `prompts/list` response item (`{ name, description?,
/// arguments? }`); the per-prompt response *body* (messages, _meta) is
/// hard-coded in the `prompts/get` handler. The live `BTreeMap` supports
/// mutation by `mcp_test/{add,remove,change_prompt}_prompt` for
/// reconciliation testing.
///
/// Argument validation reads `arguments[].required` from the live
/// entry, replacing M9.6-era `const PROMPT_SCHEMA`. Synthetic prompts
/// added at runtime (via `add_prompt_prompt`) get a generic echo
/// response from the catchall branch.
#[allow(
    clippy::too_many_lines,
    reason = "declarative prompt table — one entry per advertised prompt reads better as one block than fragmented helpers"
)]
fn initial_prompt_list() -> std::collections::BTreeMap<String, serde_json::Value> {
    let mut m = std::collections::BTreeMap::new();
    m.insert(
        "simple".to_owned(),
        serde_json::json!({
            "name": "simple",
            "description": "A trivial prompt with no arguments.",
            "arguments": []
        }),
    );
    m.insert(
        "code_review".to_owned(),
        serde_json::json!({
            "name": "code_review",
            "description": "Review this {language} code.",
            "arguments": [
                { "name": "language", "description": "Source language.", "required": true },
                { "name": "source", "description": "Source code to review.", "required": true }
            ]
        }),
    );
    m.insert(
        "code_demo".to_owned(),
        serde_json::json!({
            "name": "code_demo",
            "description": "Returns a Rust code sample with code-format hint.",
            "arguments": []
        }),
    );
    m.insert(
        "markdown_demo".to_owned(),
        serde_json::json!({
            "name": "markdown_demo",
            "description": "Returns a structured markdown document.",
            "arguments": []
        }),
    );
    m.insert(
        "multi_message".to_owned(),
        serde_json::json!({
            "name": "multi_message",
            "description": "Returns a three-message conversation (system, user, assistant).",
            "arguments": []
        }),
    );
    m.insert(
        "unknown_format".to_owned(),
        serde_json::json!({
            "name": "unknown_format",
            "description": "Returns _meta.format = an unrecognized value.",
            "arguments": []
        }),
    );
    m.insert(
        "mixed_content".to_owned(),
        serde_json::json!({
            "name": "mixed_content",
            "description": "Returns one text content entry plus one image content entry.",
            "arguments": []
        }),
    );
    m.insert(
        "code_unknown_lang".to_owned(),
        serde_json::json!({
            "name": "code_unknown_lang",
            "description": "Returns _meta.format = code with a language pmacs has no grammar for.",
            "arguments": []
        }),
    );
    m.insert(
        "markdown_inline".to_owned(),
        serde_json::json!({
            "name": "markdown_inline",
            "description": "Markdown body containing inline syntax the block grammar can't highlight (floor test).",
            "arguments": []
        }),
    );
    // T M9.8: AI-assistance fixture prompts — `pmacs-mcp-ai`'s three
    // command shapes (function-context, project-context, freeform).
    // The fake echoes args back through the response body so the
    // package's tests can verify context threading end-to-end without
    // asking what a real model would have answered.
    m.insert(
        "review_function".to_owned(),
        serde_json::json!({
            "name": "review_function",
            "description": "AI-style function review. Echoes the function source back with a code-format hint.",
            "arguments": [
                { "name": "language",  "description": "Source language.",   "required": true },
                { "name": "file_path", "description": "File the function is in.", "required": true },
                { "name": "source",    "description": "Function source code.", "required": true }
            ]
        }),
    );
    m.insert(
        "review_project".to_owned(),
        serde_json::json!({
            "name": "review_project",
            "description": "AI-style project review. Takes a structured `files: [{path, content}, ...]` array.",
            "arguments": [
                { "name": "files", "description": "Array of {path, content} objects.", "required": true }
            ]
        }),
    );
    m.insert(
        "ask_freeform".to_owned(),
        serde_json::json!({
            "name": "ask_freeform",
            "description": "Freeform AI question. Echoes the question back as the answer body.",
            "arguments": [
                { "name": "question", "description": "Question to ask.", "required": true }
            ]
        }),
    );
    m
}

/// T M9.7: build the `prompts/get` *response body* for `prompt_name`,
/// given the (already-validated) `arguments`. The match arms encode
/// the per-prompt result content + `_meta.format` hint for each
/// known fixture prompt; the catchall echo branch handles synthetic
/// prompts added at runtime by `mcp_test/add_prompt_prompt`.
///
/// Kept as a free function rather than inlined because the prompts/get
/// handler arm is already dense; pulling the body building out keeps
/// the validation flow above readable. Returns the *full JSON-RPC
/// response* (not just `result`) so the catchall and the named
/// branches share one shape.
#[allow(
    clippy::too_many_lines,
    reason = "declarative match arms; one per fixture prompt + catchall reads better as one block than fragmented per-format helpers"
)]
fn build_prompt_response(
    idv: &serde_json::Value,
    prompt_name: &str,
    arguments: &serde_json::Value,
) -> serde_json::Value {
    match prompt_name {
        "code_review" => {
            // Echo args back through the messages so M9.4 round-trip
            // tests can verify they arrived intact. No `_meta.format`
            // hint — the M9.7 package treats this as text-format.
            let language = arguments
                .get("language")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let source = arguments
                .get("source")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": idv,
                "result": {
                    "description": format!("Review this {language} code"),
                    "messages": [
                        {
                            "role": "user",
                            "content": {
                                "type": "text",
                                "text": format!("Please review this {language} code:\n{source}")
                            }
                        }
                    ]
                }
            })
        }
        "simple" => serde_json::json!({
            "jsonrpc": "2.0",
            "id": idv,
            "result": {
                "description": "A trivial prompt with no arguments",
                "messages": [
                    {
                        "role": "user",
                        "content": { "type": "text", "text": "no-args prompt body" }
                    }
                ]
            }
        }),
        "code_demo" => serde_json::json!({
            "jsonrpc": "2.0",
            "id": idv,
            "result": {
                "description": "A Rust code sample.",
                "_meta": { "format": "code", "language": "rust" },
                "messages": [
                    {
                        "role": "user",
                        "content": {
                            "type": "text",
                            "text": "fn main() {\n    let x: i32 = 42;\n    println!(\"hello, world!\");\n}\n"
                        }
                    }
                ]
            }
        }),
        "markdown_demo" => serde_json::json!({
            "jsonrpc": "2.0",
            "id": idv,
            "result": {
                "description": "A structured markdown document.",
                "_meta": { "format": "markdown" },
                "messages": [
                    {
                        "role": "user",
                        "content": {
                            "type": "text",
                            "text": "# Heading One\n\nA paragraph with **emphasis**.\n\n## Heading Two\n\n- bullet alpha\n- bullet beta\n\n```rust\nfn sample() {}\n```\n"
                        }
                    }
                ]
            }
        }),
        "multi_message" => serde_json::json!({
            "jsonrpc": "2.0",
            "id": idv,
            "result": {
                "description": "Three-message conversation.",
                "messages": [
                    {
                        "role": "system",
                        "content": { "type": "text", "text": "system instructions" }
                    },
                    {
                        "role": "user",
                        "content": { "type": "text", "text": "user question" }
                    },
                    {
                        "role": "assistant",
                        "content": { "type": "text", "text": "assistant answer" }
                    }
                ]
            }
        }),
        "unknown_format" => serde_json::json!({
            "jsonrpc": "2.0",
            "id": idv,
            "result": {
                "description": "Unrecognized format hint.",
                "_meta": { "format": "this-is-not-a-recognized-format" },
                "messages": [
                    {
                        "role": "user",
                        "content": { "type": "text", "text": "body for an unrecognized format" }
                    }
                ]
            }
        }),
        "mixed_content" => serde_json::json!({
            "jsonrpc": "2.0",
            "id": idv,
            "result": {
                "description": "Mixed content (text + image).",
                "messages": [
                    {
                        "role": "user",
                        "content": { "type": "text", "text": "preamble line" }
                    },
                    {
                        "role": "user",
                        "content": {
                            "type": "image",
                            "data": "AAECAw==",
                            "mimeType": "image/png"
                        }
                    }
                ]
            }
        }),
        "code_unknown_lang" => serde_json::json!({
            "jsonrpc": "2.0",
            "id": idv,
            "result": {
                "description": "Code-format prompt with an unregistered language.",
                "_meta": { "format": "code", "language": "klingon" },
                "messages": [
                    {
                        "role": "user",
                        "content": {
                            "type": "text",
                            "text": "QaH! Daq SoH yIqaw ghu' batlh."
                        }
                    }
                ]
            }
        }),
        "markdown_inline" => serde_json::json!({
            "jsonrpc": "2.0",
            "id": idv,
            "result": {
                "description": "Markdown with inline emphasis the block grammar can't reach.",
                "_meta": { "format": "markdown" },
                "messages": [
                    {
                        "role": "user",
                        "content": {
                            "type": "text",
                            "text": "# Title\n\nA paragraph with **bold**, _emphasis_, and a [link](https://example.invalid/).\n\n- list with `inline code`\n- and another item\n"
                        }
                    }
                ]
            }
        }),
        // T M9.8: AI-assistance prompts. Each echoes the args back
        // through the response body so the package's tests can verify
        // the context made it across the wire intact, and tags the
        // result with the format hint the package's renderer should
        // honor (`code` for review_function, `markdown` for the others).
        "review_function" => {
            let language = arguments
                .get("language")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let file_path = arguments
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let source = arguments
                .get("source")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": idv,
                "result": {
                    "description": format!("AI review of {file_path}"),
                    "_meta": { "format": "code", "language": language },
                    "messages": [
                        {
                            "role": "user",
                            "content": {
                                "type": "text",
                                "text": format!("// review of {file_path}\n{source}")
                            }
                        }
                    ]
                }
            })
        }
        "review_project" => {
            // The structured `files` arg arrives as a JSON array of
            // `{path, content}` objects. Build a markdown summary
            // listing each file's path, with the contents under it.
            use std::fmt::Write as _;
            let files = arguments.get("files").and_then(|v| v.as_array());
            let mut body = String::from("# Project review\n\n");
            if let Some(arr) = files {
                for entry in arr {
                    let path = entry.get("path").and_then(|v| v.as_str()).unwrap_or("?");
                    let content = entry.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    let _ = write!(body, "## {path}\n\n```\n{content}\n```\n\n");
                }
            }
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": idv,
                "result": {
                    "description": "AI review of project files",
                    "_meta": { "format": "markdown" },
                    "messages": [
                        {
                            "role": "user",
                            "content": { "type": "text", "text": body }
                        }
                    ]
                }
            })
        }
        "ask_freeform" => {
            let question = arguments
                .get("question")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": idv,
                "result": {
                    "description": "AI freeform answer",
                    "_meta": { "format": "markdown" },
                    "messages": [
                        {
                            "role": "user",
                            "content": {
                                "type": "text",
                                "text": format!("# Answer\n\nYou asked: {question}\n")
                            }
                        }
                    ]
                }
            })
        }
        // Catchall for synthetic prompts added at runtime via
        // `mcp_test/add_prompt_prompt`. Echoes the prompt name back
        // so reconciliation tests can verify the right prompt got
        // invoked after the registration.
        other => serde_json::json!({
            "jsonrpc": "2.0",
            "id": idv,
            "result": {
                "description": format!("Synthetic prompt: {other}"),
                "messages": [
                    {
                        "role": "user",
                        "content": {
                            "type": "text",
                            "text": format!("synthetic prompt body: {other}")
                        }
                    }
                ]
            }
        }),
    }
}

/// T M9.6: build the initial advertised tool list. Each entry is the
/// `tools/list` response shape per the MCP spec. Stored in a
/// `BTreeMap<String, serde_json::Value>` keyed by tool name so the
/// `mcp_test/{add,remove,change_schema}_tool` mutators can edit
/// individual entries deterministically.
#[allow(
    clippy::too_many_lines,
    reason = "declarative tool table; one entry per advertised tool reads better as one block than fragmented helpers"
)]
fn initial_tool_list() -> std::collections::BTreeMap<String, serde_json::Value> {
    let mut m = std::collections::BTreeMap::new();
    m.insert(
        "echo".to_owned(),
        serde_json::json!({
            "name": "echo",
            "description": "Echo input text back, prefixed with <echo>:.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "Text to echo." }
                },
                "required": ["text"]
            }
        }),
    );
    m.insert(
        "fail".to_owned(),
        serde_json::json!({
            "name": "fail",
            "description": "Always returns isError: true (semantic-failure path).",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
    );
    m.insert(
        "nullary".to_owned(),
        serde_json::json!({
            "name": "nullary",
            "description": "Tool with no required arguments.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
    );
    m.insert(
        "mcp_test/greet".to_owned(),
        serde_json::json!({
            "name": "mcp_test/greet",
            "description": "Two-required-arg greeting tool.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name":     { "type": "string", "description": "Person to greet." },
                    "greeting": { "type": "string", "description": "Greeting phrase." }
                },
                "required": ["name", "greeting"]
            }
        }),
    );
    // T M9.6: tool list mutators. The `description` and `inputSchema`
    // shape these tools advertise is what `tools/call` ends up
    // executing — keep their argument names in sync with the
    // tools/call match arms below.
    m.insert(
        "mcp_test/add_tool".to_owned(),
        serde_json::json!({
            "name": "mcp_test/add_tool",
            "description": "Append a synthetic tool and emit notifications/tools/list_changed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the new tool." }
                },
                "required": ["name"]
            }
        }),
    );
    m.insert(
        "mcp_test/remove_tool".to_owned(),
        serde_json::json!({
            "name": "mcp_test/remove_tool",
            "description": "Drop a tool and emit notifications/tools/list_changed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the tool to drop." }
                },
                "required": ["name"]
            }
        }),
    );
    m.insert(
        "mcp_test/change_tool_schema".to_owned(),
        serde_json::json!({
            "name": "mcp_test/change_tool_schema",
            "description": "Replace a tool's inputSchema.required and emit list_changed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name":     { "type": "string", "description": "Tool to mutate." },
                    "required": { "type": "string", "description": "Comma-separated new required-arg names." }
                },
                "required": ["name", "required"]
            }
        }),
    );
    // T M9.7: prompt-list mutators paralleling the M9.6 tool-list
    // mutators. Each emits notifications/prompts/list_changed so the
    // M9.7 reconciliation test can drive deterministic events.
    m.insert(
        "mcp_test/add_prompt".to_owned(),
        serde_json::json!({
            "name": "mcp_test/add_prompt",
            "description": "Append a synthetic prompt and emit notifications/prompts/list_changed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Prompt name to add." }
                },
                "required": ["name"]
            }
        }),
    );
    m.insert(
        "mcp_test/remove_prompt".to_owned(),
        serde_json::json!({
            "name": "mcp_test/remove_prompt",
            "description": "Drop a prompt and emit notifications/prompts/list_changed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Prompt name to remove." }
                },
                "required": ["name"]
            }
        }),
    );
    m.insert(
        "mcp_test/change_prompt".to_owned(),
        serde_json::json!({
            "name": "mcp_test/change_prompt",
            "description": "Replace a prompt's arguments[].required set and emit list_changed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name":     { "type": "string", "description": "Prompt to mutate." },
                    "required": { "type": "string", "description": "Comma-separated new required-arg names." }
                },
                "required": ["name", "required"]
            }
        }),
    );
    // Existing M9.3 / M9.5 tools that already had `tools/call`
    // handlers — advertise them too so M9.6 reconciliation tests can
    // see the full set.
    m.insert(
        "multipart_fail".to_owned(),
        serde_json::json!({
            "name": "multipart_fail",
            "description": "Multipart isError content: text + image + text.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
    );
    m.insert(
        "mcp_test/crash".to_owned(),
        serde_json::json!({
            "name": "mcp_test/crash",
            "description": "Exit non-zero before responding (drives M9.5 stale-recovery test).",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }),
    );
    m.insert(
        "mcp_test/trigger_update".to_owned(),
        serde_json::json!({
            "name": "mcp_test/trigger_update",
            "description": "Mutate a resource's stored content and emit notifications/resources/updated.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "uri":      { "type": "string", "description": "URI to update." },
                    "new_text": { "type": "string", "description": "New content text." }
                },
                "required": ["uri", "new_text"]
            }
        }),
    );
    // T M9.6: typed-arg coercion test fixture. Each scalar non-string
    // type the package supports gets one tool that exercises it; the
    // tools/call handler echoes back the JSON type it received so the
    // acceptance test can assert the package coerced before sending.
    m.insert(
        "typed_int".to_owned(),
        serde_json::json!({
            "name": "typed_int",
            "description": "Required integer arg; echoes the JSON type received.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "n": { "type": "integer", "description": "An integer." }
                },
                "required": ["n"]
            }
        }),
    );
    m.insert(
        "typed_number".to_owned(),
        serde_json::json!({
            "name": "typed_number",
            "description": "Required number arg; echoes the JSON type received.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "x": { "type": "number", "description": "A number." }
                },
                "required": ["x"]
            }
        }),
    );
    m.insert(
        "typed_bool".to_owned(),
        serde_json::json!({
            "name": "typed_bool",
            "description": "Required boolean arg; echoes the JSON type received.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "b": { "type": "boolean", "description": "A boolean." }
                },
                "required": ["b"]
            }
        }),
    );
    m
}

#[allow(
    clippy::too_many_lines,
    reason = "linear MCP-method dispatch; splitting a test helper this much fragments the read"
)]
fn main() {
    let mode = std::env::var("PMACS_FAKE_MCP_MODE").unwrap_or_default();
    if mode == "garbage" {
        let mut stdout = io::stdout().lock();
        let _ = stdout.write_all(b"NotAValidJsonLine\n");
        let _ = stdout.flush();
        return;
    }
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    let mut crashed_after_init = false;
    // Per-process counter incremented on every `resources/read`. The
    // value lands in the response text so M9.2 tests can detect
    // cache hit (same value on repeat) vs refetch (incremented value
    // after invalidation).
    let mut read_resource_counter: u64 = 0;
    // Counts non-handshake requests to drive the
    // `crash_after_first_request` mode deterministically.
    let mut request_counter: u64 = 0;
    // T M9.5: per-uri content store. Keyed by canonical URI.
    // Pre-populated with three synthetic resources (text, markdown,
    // directory). The `mcp_test/trigger_update` tool mutates this.
    let mut resource_store: std::collections::HashMap<String, (String, String)> =
        std::collections::HashMap::new();
    resource_store.insert(
        "mcp://text/doc.txt".to_owned(),
        ("text/plain".to_owned(), "initial doc body".to_owned()),
    );
    resource_store.insert(
        "mcp://text/readme.md".to_owned(),
        (
            "text/markdown".to_owned(),
            "# Readme\n\nInitial markdown body.".to_owned(),
        ),
    );
    // Directory: the value's content is a JSON-encoded array of
    // child URIs. Mimetype `application/vnd.pmacs.mcp.directory+json`
    // signals to the package layer that this is a directory listing;
    // the value is the JSON array of child URI strings.
    resource_store.insert(
        "mcp://dir/".to_owned(),
        (
            "application/vnd.pmacs.mcp.directory+json".to_owned(),
            r#"["mcp://text/doc.txt","mcp://text/readme.md"]"#.to_owned(),
        ),
    );
    // Table: query-result-shaped resource. The pmacs-specific
    // mimeType `application/vnd.pmacs.mcp.table+json` tells the
    // package's view module to render as a column-aligned table.
    // T M9.5 Pass-2 finding 1.
    // T M9.5 Pass-3 finding 2: ages are numeric (bareword), names
    // are quoted strings. Mixed string/number cells exercise the
    // table parser's row-tokenizer; a regression to the earlier
    // two-pass design (strings xor numbers) would lose the age
    // column entirely.
    resource_store.insert(
        "mcp://table/users.tbl".to_owned(),
        (
            "application/vnd.pmacs.mcp.table+json".to_owned(),
            r#"{"columns":["name","age"],"rows":[["alice",30],["bob",25],["carol",42]]}"#
                .to_owned(),
        ),
    );
    // T M9.5: subscription registry — uris that the client has
    // asked to be notified about on update.
    let mut resource_subscribers: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    // T M9.6: live tool list. Pre-populated from `initial_tool_list`
    // and mutated by the `mcp_test/{add,remove,change_schema}_tool`
    // tools so the M9.6 reconciliation test can drive deterministic
    // notifications/tools/list_changed events.
    let mut tools_list: std::collections::BTreeMap<String, serde_json::Value> = initial_tool_list();
    // T M9.7: live prompt list. Same shape and mutation pattern as
    // `tools_list`; mutated by `mcp_test/{add,remove,change_prompt}_prompt`.
    let mut prompts_list: std::collections::BTreeMap<String, serde_json::Value> =
        initial_prompt_list();
    for line_result in stdin.lock().lines() {
        let line = match line_result {
            Ok(l) => l,
            Err(e) => {
                eprintln!("stdin read error: {e}");
                return;
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let msg: serde_json::Value = match serde_json::from_str(&line) {
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
                if mode == "init_error" {
                    let resp = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": idv,
                        "error": {
                            "code": -32603,
                            "message": "synthetic initialize failure"
                        }
                    });
                    write_line(&mut stdout, &resp);
                    continue;
                }
                let echoed_pv = if mode == "bad_version" {
                    "1999-01-01"
                } else {
                    // Echo the client's protocolVersion when it's a
                    // value we know how to negotiate; otherwise fall
                    // back to the latest revision pmacs targets.
                    params
                        .get("protocolVersion")
                        .and_then(|v| v.as_str())
                        .unwrap_or("2025-11-25")
                };
                let resp = if mode == "missing_caps" {
                    // Pass-3 finding 2: server omits the required
                    // `capabilities` field. The client must reject
                    // this rather than silently defaulting.
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": idv,
                        "result": {
                            "protocolVersion": echoed_pv,
                            "serverInfo": { "name": "pmacs-fake-mcp", "version": "0.1.0" }
                        }
                    })
                } else {
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": idv,
                        "result": {
                            "protocolVersion": echoed_pv,
                            "capabilities": {
                                "resources": { "subscribe": true, "listChanged": false },
                                "tools": { "listChanged": true },
                                "prompts": { "listChanged": true }
                            },
                            "serverInfo": { "name": "pmacs-fake-mcp", "version": "0.1.0" }
                        }
                    })
                };
                write_line(&mut stdout, &resp);
                if mode == "crash" {
                    crashed_after_init = true;
                }
                if mode == "clean_exit_after_init" {
                    return;
                }
            }
            ("notifications/initialized" | "initialized", _) => {}
            ("notifications/cancelled", _) => {
                // T M9.3: record receipt of the MCP cancellation
                // notification by writing a sentinel file. Tests
                // that drove an `invoke_tool` cancellation poll for
                // the file as proof the server actually got it.
                if let Some(dir) = std::env::var_os("PMACS_FAKE_MCP_CANCEL_DIR")
                    && let Some(req_id) = params.get("requestId")
                {
                    // requestId is whatever the client sent —
                    // typically a u64, but per the spec it can
                    // be any JSON value. Use its string form.
                    let id_str = match req_id {
                        serde_json::Value::Number(n) => n.to_string(),
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    let path = std::path::PathBuf::from(&dir).join(format!("cancelled-{id_str}"));
                    let _ = std::fs::write(&path, b"");
                }
            }
            ("prompts/get", Some(idv)) => {
                request_counter += 1;
                let prompt_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let arguments = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);

                // Wire-shape recording: write the full params for
                // tests that need to verify `arguments` shape.
                if let Some(dir) = std::env::var_os("PMACS_FAKE_MCP_PROMPT_RECORD_DIR") {
                    let path = std::path::PathBuf::from(&dir)
                        .join(format!("prompt-{request_counter}.json"));
                    if let Ok(bytes) = serde_json::to_vec(&params) {
                        let _ = std::fs::write(&path, &bytes);
                    }
                }

                // T M9.7: look up the prompt in the live BTreeMap.
                // Required-args validation reads `arguments[].required`
                // from the live entry; the response body is built per-
                // prompt below with a catchall branch for synthetic
                // prompts added by `mcp_test/add_prompt_prompt`.
                let entry = prompts_list.get(prompt_name).cloned();
                let resp = match entry {
                    None => serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": idv,
                        "error": {
                            "code": -32602,
                            "message": format!("unknown prompt: {prompt_name}")
                        }
                    }),
                    Some(prompt_entry) => {
                        // Walk arguments[] and find the first required
                        // arg that's missing/empty. Mirrors the M9.4
                        // validation but driven from the live entry.
                        let required_args: Vec<String> = prompt_entry
                            .get("arguments")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter(|a| {
                                        a.get("required")
                                            .and_then(serde_json::Value::as_bool)
                                            .unwrap_or(false)
                                    })
                                    .filter_map(|a| {
                                        a.get("name").and_then(|n| n.as_str()).map(str::to_owned)
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        // Required-arg presence check. M9.4 only ever passed
                        // string values, so the original gate was
                        // `as_str().is_none_or(str::is_empty)`. M9.8's
                        // `review_project` prompt takes a structured
                        // `files: [{path, content}, ...]` array argument
                        // (Q5 reshape — explicit JSON beats separator
                        // encoding). The relaxed check is "key present and
                        // not null, with the empty-string case still
                        // rejected" — preserves M9.4's missing-arg
                        // behaviour while admitting non-string args.
                        let missing =
                            required_args
                                .iter()
                                .find(|arg| match arguments.get(arg.as_str()) {
                                    None | Some(serde_json::Value::Null) => true,
                                    Some(serde_json::Value::String(s)) if s.is_empty() => true,
                                    _ => false,
                                });
                        if let Some(missing_arg) = missing {
                            serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": idv,
                                "error": {
                                    "code": -32602,
                                    "message": format!("missing required argument: {missing_arg}")
                                }
                            })
                        } else {
                            build_prompt_response(&idv, prompt_name, &arguments)
                        }
                    }
                };
                write_line(&mut stdout, &resp);
            }
            ("prompts/list", Some(idv)) => {
                // T M9.7: dump the live advertised set. Matches the
                // M9.6 `tools/list` shape — a BTreeMap iteration is
                // already sorted, so test assertions can assume a
                // stable ordering.
                request_counter += 1;
                let arr: Vec<serde_json::Value> = prompts_list.values().cloned().collect();
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": idv,
                    "result": { "prompts": arr }
                });
                write_line(&mut stdout, &resp);
            }
            ("tools/list", Some(idv)) => {
                request_counter += 1;
                let arr: Vec<serde_json::Value> = tools_list.values().cloned().collect();
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": idv,
                    "result": { "tools": arr }
                });
                write_line(&mut stdout, &resp);
            }
            ("tools/call", Some(idv)) => {
                request_counter += 1;
                if mode == "slow_tools_call" {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
                let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let arguments = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let resp = match tool_name {
                    "echo" => {
                        let input = arguments.get("text").and_then(|v| v.as_str()).unwrap_or("");
                        serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": idv,
                            "result": {
                                "content": [{
                                    "type": "text",
                                    "text": format!("<echo>: {input}")
                                }],
                                "isError": false
                            }
                        })
                    }
                    "nullary" => serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": idv,
                        "result": {
                            "content": [{ "type": "text", "text": "no-arg tool ran" }],
                            "isError": false
                        }
                    }),
                    // T M9.6 typed-arg coercion: each typed_* tool
                    // echoes back the JSON shape the manager actually
                    // delivered (number / boolean / string) so the
                    // acceptance test can assert the package coerced
                    // the minibuffer string before sending. Reading
                    // .as_i64()/.as_f64()/.as_bool() returns None when
                    // the arg arrived as a string.
                    "typed_int" => {
                        let raw = arguments
                            .get("n")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null);
                        let kind = if raw.as_i64().is_some() {
                            "integer"
                        } else if raw.as_f64().is_some() {
                            "number"
                        } else if raw.is_string() {
                            "string"
                        } else {
                            "other"
                        };
                        serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": idv,
                            "result": {
                                "content": [{
                                    "type": "text",
                                    "text": format!("typed_int: kind={kind} value={raw}")
                                }],
                                "isError": false
                            }
                        })
                    }
                    "typed_number" => {
                        let raw = arguments
                            .get("x")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null);
                        let kind = if raw.as_f64().is_some() {
                            "number"
                        } else if raw.is_string() {
                            "string"
                        } else {
                            "other"
                        };
                        serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": idv,
                            "result": {
                                "content": [{
                                    "type": "text",
                                    "text": format!("typed_number: kind={kind} value={raw}")
                                }],
                                "isError": false
                            }
                        })
                    }
                    "typed_bool" => {
                        let raw = arguments
                            .get("b")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null);
                        let kind = if raw.as_bool().is_some() {
                            "boolean"
                        } else if raw.is_string() {
                            "string"
                        } else {
                            "other"
                        };
                        serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": idv,
                            "result": {
                                "content": [{
                                    "type": "text",
                                    "text": format!("typed_bool: kind={kind} value={raw}")
                                }],
                                "isError": false
                            }
                        })
                    }
                    "mcp_test/greet" => {
                        let person = arguments.get("name").and_then(|v| v.as_str()).unwrap_or("");
                        let phrase = arguments
                            .get("greeting")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": idv,
                            "result": {
                                "content": [{
                                    "type": "text",
                                    "text": format!("Hello, {person}! {phrase}")
                                }],
                                "isError": false
                            }
                        })
                    }
                    "mcp_test/add_tool" => {
                        // T M9.6: append a synthetic tool and emit
                        // notifications/tools/list_changed. Used by
                        // the reconciliation test.
                        let new_name = arguments
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_owned();
                        if new_name.is_empty() {
                            serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": idv,
                                "error": { "code": -32602, "message": "missing required name" }
                            })
                        } else {
                            tools_list.insert(
                                new_name.clone(),
                                serde_json::json!({
                                    "name": new_name,
                                    "description": "Synthetic tool added at runtime.",
                                    "inputSchema": {
                                        "type": "object",
                                        "properties": {},
                                        "required": []
                                    }
                                }),
                            );
                            let ok_resp = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": idv,
                                "result": {
                                    "content": [{
                                        "type": "text",
                                        "text": format!("added: {new_name}")
                                    }],
                                    "isError": false
                                }
                            });
                            write_line(&mut stdout, &ok_resp);
                            let notif = serde_json::json!({
                                "jsonrpc": "2.0",
                                "method": "notifications/tools/list_changed",
                                "params": {}
                            });
                            write_line(&mut stdout, &notif);
                            continue;
                        }
                    }
                    "mcp_test/remove_tool" => {
                        let drop_name = arguments
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_owned();
                        tools_list.remove(&drop_name);
                        let ok_resp = serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": idv,
                            "result": {
                                "content": [{
                                    "type": "text",
                                    "text": format!("removed: {drop_name}")
                                }],
                                "isError": false
                            }
                        });
                        write_line(&mut stdout, &ok_resp);
                        let notif = serde_json::json!({
                            "jsonrpc": "2.0",
                            "method": "notifications/tools/list_changed",
                            "params": {}
                        });
                        write_line(&mut stdout, &notif);
                        continue;
                    }
                    "mcp_test/change_tool_schema" => {
                        // Replace the named tool's `inputSchema.required`
                        // with a fresh comma-separated list. Used by the
                        // schema-change reconciliation test — proves the
                        // package re-registers when the schema mutates,
                        // not just when the name set changes.
                        let target = arguments
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_owned();
                        let required = arguments
                            .get("required")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_owned();
                        if target.is_empty() {
                            serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": idv,
                                "error": { "code": -32602, "message": "missing required name" }
                            })
                        } else if let Some(entry) = tools_list.get_mut(&target) {
                            let new_required: Vec<serde_json::Value> = required
                                .split(',')
                                .filter(|s| !s.is_empty())
                                .map(|s| serde_json::Value::String(s.to_owned()))
                                .collect();
                            let mut new_props = serde_json::Map::new();
                            for arg in &new_required {
                                if let Some(arg_name) = arg.as_str() {
                                    new_props.insert(
                                        arg_name.to_owned(),
                                        serde_json::json!({
                                            "type": "string",
                                            "description": format!("(synthetic) {arg_name}")
                                        }),
                                    );
                                }
                            }
                            entry["inputSchema"] = serde_json::json!({
                                "type": "object",
                                "properties": new_props,
                                "required": new_required
                            });
                            let ok_resp = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": idv,
                                "result": {
                                    "content": [{
                                        "type": "text",
                                        "text": format!("schema updated: {target}")
                                    }],
                                    "isError": false
                                }
                            });
                            write_line(&mut stdout, &ok_resp);
                            let notif = serde_json::json!({
                                "jsonrpc": "2.0",
                                "method": "notifications/tools/list_changed",
                                "params": {}
                            });
                            write_line(&mut stdout, &notif);
                            continue;
                        } else {
                            serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": idv,
                                "error": {
                                    "code": -32602,
                                    "message": format!("unknown tool: {target}")
                                }
                            })
                        }
                    }
                    "mcp_test/add_prompt" => {
                        // T M9.7: append a synthetic prompt and emit
                        // notifications/prompts/list_changed.
                        let new_name = arguments
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_owned();
                        if new_name.is_empty() {
                            serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": idv,
                                "error": { "code": -32602, "message": "missing required name" }
                            })
                        } else {
                            prompts_list.insert(
                                new_name.clone(),
                                serde_json::json!({
                                    "name": new_name,
                                    "description": "Synthetic prompt added at runtime.",
                                    "arguments": []
                                }),
                            );
                            let ok_resp = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": idv,
                                "result": {
                                    "content": [{
                                        "type": "text",
                                        "text": format!("added prompt: {new_name}")
                                    }],
                                    "isError": false
                                }
                            });
                            write_line(&mut stdout, &ok_resp);
                            let notif = serde_json::json!({
                                "jsonrpc": "2.0",
                                "method": "notifications/prompts/list_changed",
                                "params": {}
                            });
                            write_line(&mut stdout, &notif);
                            continue;
                        }
                    }
                    "mcp_test/remove_prompt" => {
                        let drop_name = arguments
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_owned();
                        prompts_list.remove(&drop_name);
                        let ok_resp = serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": idv,
                            "result": {
                                "content": [{
                                    "type": "text",
                                    "text": format!("removed prompt: {drop_name}")
                                }],
                                "isError": false
                            }
                        });
                        write_line(&mut stdout, &ok_resp);
                        let notif = serde_json::json!({
                            "jsonrpc": "2.0",
                            "method": "notifications/prompts/list_changed",
                            "params": {}
                        });
                        write_line(&mut stdout, &notif);
                        continue;
                    }
                    "mcp_test/change_prompt" => {
                        // Replace the named prompt's `arguments[].required`
                        // set. Used by the schema-change reconciliation
                        // test — same shape as `change_tool_schema` but
                        // for prompts.
                        let target = arguments
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_owned();
                        let required = arguments
                            .get("required")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_owned();
                        if target.is_empty() {
                            serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": idv,
                                "error": { "code": -32602, "message": "missing required name" }
                            })
                        } else if let Some(entry) = prompts_list.get_mut(&target) {
                            let new_args: Vec<serde_json::Value> = required
                                .split(',')
                                .filter(|s| !s.is_empty())
                                .map(|s| {
                                    serde_json::json!({
                                        "name": s,
                                        "description": format!("(synthetic) {s}"),
                                        "required": true
                                    })
                                })
                                .collect();
                            entry["arguments"] = serde_json::Value::Array(new_args);
                            let ok_resp = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": idv,
                                "result": {
                                    "content": [{
                                        "type": "text",
                                        "text": format!("prompt schema updated: {target}")
                                    }],
                                    "isError": false
                                }
                            });
                            write_line(&mut stdout, &ok_resp);
                            let notif = serde_json::json!({
                                "jsonrpc": "2.0",
                                "method": "notifications/prompts/list_changed",
                                "params": {}
                            });
                            write_line(&mut stdout, &notif);
                            continue;
                        } else {
                            serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": idv,
                                "error": {
                                    "code": -32602,
                                    "message": format!("unknown prompt: {target}")
                                }
                            })
                        }
                    }
                    "fail" => serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": idv,
                        "result": {
                            "content": [{
                                "type": "text",
                                "text": "synthetic tool failure"
                            }],
                            "isError": true
                        }
                    }),
                    "multipart_fail" => serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": idv,
                        "result": {
                            "content": [
                                { "type": "text", "text": "Failed: " },
                                { "type": "image", "data": "...", "mimeType": "image/png" },
                                { "type": "text", "text": "see attached" }
                            ],
                            "isError": true
                        }
                    }),
                    "mcp_test/crash" => {
                        // T M9.5 Pass-2 finding 3: deterministic
                        // crash trigger for the stale-recovery test.
                        // Exits before responding so the in-flight
                        // tool call settles cancelled and the manager
                        // observes the process death; OnCrash policy
                        // then drives the restart that the recovery
                        // path tests.
                        std::process::exit(78);
                    }
                    "mcp_test/trigger_update" => {
                        // T M9.5: deterministic resource-update trigger.
                        // Updates the per-uri store and emits a
                        // `notifications/resources/updated` notification
                        // so subscribers re-fetch.
                        let uri = arguments.get("uri").and_then(|v| v.as_str()).unwrap_or("");
                        let new_text = arguments
                            .get("new_text")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if uri.is_empty() {
                            serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": idv,
                                "error": { "code": -32602, "message": "missing required uri" }
                            })
                        } else {
                            // Update the store. Preserve mimeType if
                            // the uri exists; default to text/plain
                            // for new uris.
                            let mime = resource_store
                                .get(uri)
                                .map_or_else(|| "text/plain".to_owned(), |(m, _)| m.clone());
                            resource_store.insert(uri.to_owned(), (mime, new_text.to_owned()));
                            // Reply success first.
                            let ok_resp = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": idv,
                                "result": {
                                    "content": [{
                                        "type": "text",
                                        "text": format!("updated: {uri}")
                                    }],
                                    "isError": false
                                }
                            });
                            write_line(&mut stdout, &ok_resp);
                            // Then emit the update notification (only
                            // if the uri is subscribed).
                            if resource_subscribers.contains(uri) {
                                let notif = serde_json::json!({
                                    "jsonrpc": "2.0",
                                    "method": "notifications/resources/updated",
                                    "params": { "uri": uri }
                                });
                                write_line(&mut stdout, &notif);
                            }
                            // Skip the trailing write_line below.
                            continue;
                        }
                    }
                    other => serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": idv,
                        "error": {
                            "code": -32602,
                            "message": format!("unknown tool: {other}")
                        }
                    }),
                };
                write_line(&mut stdout, &resp);
            }
            ("shutdown" | "exit", _) if mode == "crash_on_protocol_shutdown" => {
                // Pass-3 finding 1: MCP stdio has no protocol-level
                // shutdown messages. A pmacs that still sends them
                // is non-compliant; the test fixture surfaces that
                // by exiting non-zero.
                std::process::exit(99);
            }
            ("ping", Some(idv)) => {
                request_counter += 1;
                if mode == "crash_after_first_request" && request_counter >= 1 {
                    std::process::exit(77);
                }
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": idv,
                    "result": {}
                });
                write_line(&mut stdout, &resp);
            }
            ("resources/read", Some(idv)) => {
                request_counter += 1;
                if mode == "crash_after_first_request" && request_counter >= 1 {
                    // Exit before responding — the in-flight request
                    // never settles via a happy path. The cache
                    // machinery should observe the process death
                    // and surface the failure to every attached
                    // awaiter.
                    std::process::exit(77);
                }
                if mode == "slow_resources_read" {
                    // Pass-2 (M9.2) finding 2: widen the in-flight
                    // window so the per-sibling cancellation test
                    // can cancel one awaiter before the response
                    // arrives.
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
                read_resource_counter += 1;
                let uri = params
                    .get("uri")
                    .and_then(|v| v.as_str())
                    .unwrap_or("<no-uri>");
                // T M9.5 Pass-2 finding 2: special URI that always
                // errors. Used to verify the open() registry-leak
                // fix — the package must clean up its bookkeeping
                // when the initial fetch fails.
                if uri == "mcp://error/test" {
                    let resp = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": idv,
                        "error": {
                            "code": -32602,
                            "message": "synthetic resource read failure"
                        }
                    });
                    write_line(&mut stdout, &resp);
                    continue;
                }
                // T M9.5: serve from the per-uri store if known;
                // otherwise fall back to the M9.2 synthetic-counter
                // shape (so existing M9.2 tests still pass).
                let resp = if let Some((mime, text)) = resource_store.get(uri) {
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": idv,
                        "result": {
                            "contents": [{
                                "uri": uri,
                                "mimeType": mime,
                                "text": text
                            }]
                        }
                    })
                } else {
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": idv,
                        "result": {
                            "contents": [{
                                "uri": uri,
                                "mimeType": "text/plain",
                                "text": format!("synthetic-{read_resource_counter}-for-{uri}")
                            }]
                        }
                    })
                };
                write_line(&mut stdout, &resp);
            }
            ("resources/subscribe", Some(idv)) => {
                request_counter += 1;
                let uri = params.get("uri").and_then(|v| v.as_str()).unwrap_or("");
                if uri.is_empty() {
                    let resp = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": idv,
                        "error": { "code": -32602, "message": "missing required uri" }
                    });
                    write_line(&mut stdout, &resp);
                } else {
                    resource_subscribers.insert(uri.to_owned());
                    let resp = serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": idv,
                        "result": {}
                    });
                    write_line(&mut stdout, &resp);
                }
            }
            ("resources/unsubscribe", Some(idv)) => {
                request_counter += 1;
                let uri = params.get("uri").and_then(|v| v.as_str()).unwrap_or("");
                resource_subscribers.remove(uri);
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": idv,
                    "result": {}
                });
                write_line(&mut stdout, &resp);
            }
            ("shutdown", Some(idv)) => {
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": idv,
                    "result": serde_json::Value::Null
                });
                write_line(&mut stdout, &resp);
            }
            ("exit", _) => return,
            (_, Some(idv)) => {
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": idv,
                    "result": { "echo": params, "method": method }
                });
                write_line(&mut stdout, &resp);
            }
            _ => {}
        }
        if crashed_after_init {
            std::process::exit(7);
        }
    }
    if mode == "ignore_eof_sleep" {
        loop {
            std::thread::sleep(std::time::Duration::from_mins(1));
        }
    }
}

fn write_line<W: Write>(w: &mut W, body: &serde_json::Value) {
    let bytes = serde_json::to_vec(body).expect("json serialize");
    let _ = w.write_all(&bytes);
    let _ = w.write_all(b"\n");
    let _ = w.flush();
}
