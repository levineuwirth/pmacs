//! Typed-edit consumer chain acceptance (Arc 8 Stage 4a,
//! docs/lean4-mode-framing.md Q#LN10, criteria 46a–46e).
//!
//! The chain owns the single `buffer.after-edit` subscriber that reads
//! the one-shot typed-edit record (Q#AP9) and offers it to consumers in
//! priority order. These tests pin the chain's OWN behavior — take-once,
//! priority ordering, claim-stops-chain, throw containment, and the
//! Q#AP7 flush ordering it inherited from `pair.lua`.
//!
//! They deliberately do not re-test auto-pairing: criterion 46 requires
//! `tests/auto_pair_acceptance.rs` to pass byte-identical, and that
//! suite is the no-behavior-change pin. Pairing appears here only as
//! the chain's last consumer, which is how 46c observes that a claim
//! really stopped the chain.
//!
//! Dispatch-driven throughout: `dispatch_key` is the producer that arms
//! the record for a grid frontend.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::editor::EditorState;
use pmacs::lua_bindings::StateDir;
use pmacs::protocol::FrontendId;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

fn fresh_state_dir() -> PathBuf {
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "pmacs-typededit-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: mods,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn type_str(s: &mut EditorState, text: &str) {
    for ch in text.chars() {
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char(ch), KeyModifiers::NONE),
        );
    }
}

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

fn buffer_text(s: &EditorState) -> String {
    let b: mlua::String = eval(
        s,
        "local b = pmacs.window.buffer(); return b:slice(0, b:len())",
    );
    String::from_utf8_lossy(&b.as_bytes()).into_owned()
}

fn status(s: &EditorState) -> String {
    s.core.borrow().status.clone()
}

/// Fresh scratch-buffer editor, cursor at 0. Scratch pairing uses the
/// `default` set, so `(` pairs — which is what 46c reads.
fn editor_with(body: &str) -> EditorState {
    let s = EditorState::new();
    if !body.is_empty() {
        exec(&s, &format!("pmacs.window.buffer():insert(0, {body:?})"));
    }
    exec(&s, "pmacs.editor.goto_byte(0)");
    s
}

// ---------------------------------------------------------------------------
// 46a — one read for the whole fan-out
// ---------------------------------------------------------------------------

#[test]
fn chain_reads_the_record_once_and_hands_the_same_one_to_every_consumer() {
    let mut s = editor_with("");
    exec(
        &s,
        r#"
        _G.seen = {}
        local function spy(tag)
          return function(rec)
            -- Each consumer independently attempts its own take. Under
            -- the pre-chain design this is exactly what a second
            -- consumer would have done, and exactly what would have
            -- returned nil (or stolen the record from pairing).
            local own = pmacs.editor.take_typed_edit()
            _G.seen[#_G.seen + 1] = {
              tag = tag,
              char = rec and rec.char,
              post_cursor = rec and rec.post_cursor,
              clean = rec and rec.clean,
              own_take_was_nil = (own == nil),
            }
            return false
          end
        end
        pmacs.typed_edit.add_consumer { name = "spy-a", priority = 1, fn = spy("a") }
        pmacs.typed_edit.add_consumer { name = "spy-b", priority = 2, fn = spy("b") }
        "#,
    );

    type_str(&mut s, "x");

    let (n, a_char, b_char, a_pc, b_pc, a_clean, b_clean, a_nil, b_nil): (
        i64,
        String,
        String,
        i64,
        i64,
        bool,
        bool,
        bool,
        bool,
    ) = eval(
        &s,
        "
        local a, b = _G.seen[1], _G.seen[2]
        return #_G.seen, a.char, b.char, a.post_cursor, b.post_cursor,
               a.clean, b.clean, a.own_take_was_nil, b.own_take_was_nil
        ",
    );

    assert_eq!(n, 2, "both consumers ran for one typed character");
    // The same record, not two reads of a slot that only one could win.
    assert_eq!(a_char, "x");
    assert_eq!(b_char, "x", "the second consumer sees the record too");
    assert_eq!((a_pc, b_pc), (1, 1), "identical post_cursor");
    assert!(a_clean && b_clean, "identical clean verdict");
    // ...and the chain, not the consumers, did the taking.
    assert!(
        a_nil && b_nil,
        "a consumer's own take_typed_edit() observes nil — the chain \
         already consumed the one-shot slot (Q#AP9)"
    );
}

#[test]
fn consumers_run_when_the_fan_out_carries_no_record() {
    // The chain calls consumers with nil rather than skipping them.
    // Three tests in the auto-pairing suite depend on this (they assert
    // `_last_record == nil` after a record-less fan-out), so it is a
    // load-bearing decision and not an implementation detail.
    let s = editor_with("");
    exec(
        &s,
        r#"
        _G.calls, _G.nil_calls = 0, 0
        pmacs.typed_edit.add_consumer {
          name = "nil-spy", priority = 1,
          fn = function(rec)
            _G.calls = _G.calls + 1
            if rec == nil then _G.nil_calls = _G.nil_calls + 1 end
            return false
          end,
        }
        "#,
    );

    // A manual fan-out arms no record.
    exec(&s, "pmacs.hook.run(\"buffer.after-edit\")");

    let (calls, nil_calls): (i64, i64) = eval(&s, "return _G.calls, _G.nil_calls");
    assert_eq!(calls, 1, "the consumer ran");
    assert_eq!(nil_calls, 1, "and was handed nil, not skipped");
}

// ---------------------------------------------------------------------------
// 46b — priority order, not registration order
// ---------------------------------------------------------------------------

#[test]
fn consumers_run_in_priority_order_not_registration_order() {
    let mut s = editor_with("");
    // Registered HIGH priority first. If the chain honored registration
    // order (or `include_str!` order, which is the same failure dressed
    // differently), the observed order would be the registration order.
    exec(
        &s,
        r#"
        _G.order = {}
        local function mark(tag)
          return function() _G.order[#_G.order + 1] = tag; return false end
        end
        pmacs.typed_edit.add_consumer { name = "late",  priority = 30, fn = mark("late") }
        pmacs.typed_edit.add_consumer { name = "early", priority = 10, fn = mark("early") }
        pmacs.typed_edit.add_consumer { name = "mid",   priority = 20, fn = mark("mid") }
        "#,
    );

    type_str(&mut s, "x");

    let order: String = eval(&s, "return table.concat(_G.order, ',')");
    assert_eq!(
        order, "early,mid,late",
        "lowest priority runs first, regardless of when it registered"
    );
}

#[test]
fn equal_priorities_break_by_registration_order() {
    // The stated tiebreak. Lua's `table.sort` is not stable, so this
    // bites an implementation that sorts instead of inserting in place.
    let mut s = editor_with("");
    exec(
        &s,
        r#"
        _G.order = {}
        local function mark(tag)
          return function() _G.order[#_G.order + 1] = tag; return false end
        end
        pmacs.typed_edit.add_consumer { name = "first",  priority = 5, fn = mark("first") }
        pmacs.typed_edit.add_consumer { name = "second", priority = 5, fn = mark("second") }
        pmacs.typed_edit.add_consumer { name = "third",  priority = 5, fn = mark("third") }
        "#,
    );

    type_str(&mut s, "x");

    let order: String = eval(&s, "return table.concat(_G.order, ',')");
    assert_eq!(order, "first,second,third");
}

// ---------------------------------------------------------------------------
// 46c — a claim stops the chain
// ---------------------------------------------------------------------------

#[test]
fn a_claiming_consumer_stops_the_chain() {
    let mut s = editor_with("");
    exec(
        &s,
        r#"
        _G.later_ran = false
        pmacs.typed_edit.add_consumer {
          name = "claimer", priority = 1, fn = function() return true end,
        }
        pmacs.typed_edit.add_consumer {
          name = "later", priority = 2,
          fn = function() _G.later_ran = true; return false end,
        }
        "#,
    );

    type_str(&mut s, "(");

    let later_ran: bool = eval(&s, "return _G.later_ran");
    assert!(!later_ran, "a later consumer must not run after a claim");
    // Pairing is the chain's last consumer at priority 100, so the
    // claim is observable in the buffer: no closer was inserted. This
    // is the assertion that makes the criterion about behavior rather
    // than about a bookkeeping flag.
    assert_eq!(
        buffer_text(&s),
        "(",
        "auto-pairing never ran, so the opener stands alone"
    );
}

#[test]
fn a_non_claiming_consumer_does_not_stop_the_chain() {
    let mut s = editor_with("");
    exec(
        &s,
        r#"
        _G.later_ran = false
        pmacs.typed_edit.add_consumer {
          name = "passer", priority = 1, fn = function() return false end,
        }
        pmacs.typed_edit.add_consumer {
          name = "later", priority = 2,
          fn = function() _G.later_ran = true; return false end,
        }
        "#,
    );

    type_str(&mut s, "(");

    let later_ran: bool = eval(&s, "return _G.later_ran");
    assert!(later_ran, "a declining consumer passes the edit along");
    assert_eq!(
        buffer_text(&s),
        "()",
        "and pairing, still last in the chain, reacted normally"
    );
}

// ---------------------------------------------------------------------------
// 46d — a throwing consumer is contained
// ---------------------------------------------------------------------------

#[test]
fn a_throwing_consumer_is_contained_reported_and_does_not_stop_the_chain() {
    let mut s = editor_with("");
    exec(
        &s,
        r#"
        _G.later_ran = false
        pmacs.typed_edit.add_consumer {
          name = "boom", priority = 1,
          fn = function() error("consumer exploded") end,
        }
        pmacs.typed_edit.add_consumer {
          name = "later", priority = 2,
          fn = function() _G.later_ran = true; return false end,
        }
        "#,
    );

    // `buffer.after-edit` is all-must-succeed: an uncontained throw
    // would fail the fan-out for every other subscriber, including
    // lsp.lua's didChange flush.
    type_str(&mut s, "(");

    let later_ran: bool = eval(&s, "return _G.later_ran");
    assert!(later_ran, "a throwing consumer must not stop the chain");
    assert_eq!(
        buffer_text(&s),
        "()",
        "and pairing still ran — the fan-out survived the throw"
    );
    let st = status(&s);
    assert!(
        st.contains("boom") && st.contains("consumer exploded"),
        "the failure is reported by consumer name and message, got {st:?}"
    );
}

#[test]
fn add_consumer_rejects_malformed_registrations() {
    let s = editor_with("");
    for (src, want) in [
        (
            "pmacs.typed_edit.add_consumer(\"nope\")",
            "spec must be a table",
        ),
        (
            "pmacs.typed_edit.add_consumer{ priority = 1, fn = function() end }",
            "name must be a non-empty string",
        ),
        (
            "pmacs.typed_edit.add_consumer{ name = \"n\", fn = function() end }",
            "priority must be a number",
        ),
        (
            "pmacs.typed_edit.add_consumer{ name = \"n\", priority = 1 }",
            "fn must be a function",
        ),
    ] {
        let err = s
            .lua_host
            .lua()
            .load(src.to_string())
            .exec()
            .expect_err("malformed registration must throw");
        let msg = err.to_string();
        assert!(
            msg.contains(want),
            "expected {want:?} in the error for {src:?}, got {msg:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 46e — the Q#AP7 flush ordering the chain inherited
// ---------------------------------------------------------------------------

fn fake_lsp_path() -> String {
    env!("CARGO_BIN_EXE_pmacs_fake_lsp").to_owned()
}

fn pump_lua_flag(state: &mut EditorState, flag: &str, secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        state.tick_processes();
        state.tick_lsp();
        state.tick_async();
        let done: bool = state
            .lua_host
            .lua()
            .load(format!("return ({flag}) == true"))
            .eval()
            .unwrap_or(false);
        if done {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The `text` of every `textDocument/didChange` line in the sink, in
/// arrival order.
fn did_change_texts(sink: &std::path::Path) -> Vec<String> {
    let Ok(raw) = std::fs::read_to_string(sink) else {
        return Vec::new();
    };
    raw.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v.get("method").and_then(|m| m.as_str()) == Some("textDocument/didChange"))
        .filter_map(|v| v.get("text").and_then(|t| t.as_str()).map(str::to_owned))
        .collect()
}

#[test]
fn a_chain_consumers_edit_reaches_the_first_did_change() {
    // Q#AP7 generalized from pairing to the chain: lsp.lua's after-edit
    // callback flushes didChange SYNCHRONOUSLY on the signature-trigger
    // path, so every reaction to a typed character must already be in
    // the buffer when it runs. The auto-pairing suite pins this for
    // pairing; this pins it for the chain itself, which is what now
    // owns the registration position.
    //
    // Falsified by loading typed_edit.lua after lsp.lua in
    // `src/editor.rs`: the consumer's text would then arrive in the
    // SECOND didChange, or not at all.
    let dir = fresh_state_dir();
    let sink = dir.join("changes.jsonl");
    let sink_disp = sink.display().to_string();
    let fake = fake_lsp_path();

    let mut s = EditorState::new();
    s.lua_host.lua().remove_app_data::<StateDir>();
    s.lua_host.lua().set_app_data(StateDir(dir.clone()));
    exec(&s, "pmacs.lsp.config = {}");
    exec(
        &s,
        &format!(
            "pmacs.lsp.config.rust = {{
               command = '{fake}',
               env = {{
                 PMACS_FAKE_LSP_MODE = 'sighelp',
                 PMACS_FAKE_LSP_CHANGE_SINK = '{sink_disp}',
               }},
             }}"
        ),
    );

    // A consumer that appends a marker of its own, ahead of pairing.
    // It declines the claim so pairing still runs — the assertion is
    // about ordering against the flush, not about claiming.
    exec(
        &s,
        r#"
        pmacs.typed_edit.add_consumer {
          name = "marker", priority = 1,
          fn = function(rec)
            if not rec then return false end
            if rec.char ~= "(" then return false end
            local buf = pmacs.window.buffer()
            buf:insert(buf:len(), "Z")
            return false
          end,
        }
        "#,
    );

    let f = dir.join("a.rs");
    std::fs::write(&f, "\n").unwrap();
    let fd = f.display().to_string();
    exec(&s, &format!("pmacs.buffer.find_or_open({fd:?})"));
    exec(&s, "pmacs.editor.goto_byte(0)");
    let initialized = "(function() \
       for _,r in ipairs(pmacs.lsp.list()) do \
         if r.state and r.state.kind=='initialized' then return true end \
       end \
       return false \
     end)()";
    assert!(pump_lua_flag(&mut s, initialized, 5), "fake server init");

    type_str(&mut s, "(");
    assert_eq!(
        buffer_text(&s),
        "()\nZ",
        "both the chain consumer's marker and pairing's closer landed"
    );

    let deadline = Instant::now() + Duration::from_secs(5);
    let changes = loop {
        s.tick_processes();
        s.tick_lsp();
        s.tick_async();
        let c = did_change_texts(&sink);
        if !c.is_empty() {
            break c;
        }
        assert!(
            Instant::now() < deadline,
            "no didChange reached the fake server"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(
        changes[0], "()\nZ",
        "the FIRST didChange carries BOTH reactions — the chain ran \
         before lsp.lua's synchronous flush (Q#AP7)"
    );
}
