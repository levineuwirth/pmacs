//! Mode-system wiring acceptance over the real daemon process.
//!
//! One ordered scenario keeps the dispatch assertions observable: every
//! checkpoint is itself reached through a wire key event, and the final
//! statusline marker is published only after all Lua-side assertions pass.
//! Statusline assertions consume the daemon's real grid-render payloads.

use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use pmacs::cell::{Cell, CellSize, Glyph};
use pmacs::protocol::{
    AttachRequest, FrontendEvent, FrontendId, Hello, InstanceMessage, Key, KeyEvent, Modifiers,
    PROTOCOL_VERSION,
};
use pmacs::transport::{read_message, write_message};

mod common;
use common::daemon::{TestDaemon, build_default_caps};

struct Client {
    stream: UnixStream,
    frontend_id: FrontendId,
}

const ROWS: u32 = 30;
const COLS: u32 = 160;

struct Grid {
    cells: Vec<Cell>,
}

impl Grid {
    fn new() -> Self {
        Self {
            cells: vec![Cell::default(); (ROWS * COLS) as usize],
        }
    }

    fn apply(&mut self, spans: Vec<pmacs::cell::DiffSpan>) {
        for span in spans {
            let start = (span.start.row * COLS + span.start.col) as usize;
            for (offset, cell) in span.cells.into_iter().enumerate() {
                self.cells[start + offset] = cell;
            }
        }
    }

    fn text(&self) -> String {
        let mut text = String::with_capacity((ROWS * (COLS + 1)) as usize);
        for row in 0..ROWS {
            for column in 0..COLS {
                let cell = &self.cells[(row * COLS + column) as usize];
                let ch = match &cell.glyph {
                    Glyph::Char(ch) => *ch,
                    Glyph::Cluster(bytes) => std::str::from_utf8(bytes)
                        .ok()
                        .and_then(|value| value.chars().next())
                        .unwrap_or(' '),
                    Glyph::Continuation => ' ',
                };
                text.push(ch);
            }
            text.push('\n');
        }
        text
    }
}

fn attach(daemon: &TestDaemon) -> (Client, Grid) {
    let mut stream = daemon.connect();
    stream
        .set_read_timeout(Some(Duration::from_millis(100)))
        .expect("set daemon read timeout");
    let hello: Hello = read_message(&mut stream).expect("read daemon Hello");
    assert_eq!(hello.protocol_version, PROTOCOL_VERSION);
    write_message(
        &mut stream,
        &AttachRequest {
            protocol_version: PROTOCOL_VERSION,
            frontend_capabilities: build_default_caps(),
            initial_size: CellSize::new(ROWS, COLS),
        },
    )
    .expect("attach grid frontend");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut grid = Grid::new();
    loop {
        assert!(
            Instant::now() < deadline,
            "initial full-grid frame timed out"
        );
        if let Ok(InstanceMessage::CellDelta {
            spans,
            full_grid: true,
        }) = read_message::<InstanceMessage>(&mut stream)
        {
            grid.apply(spans);
            break;
        }
    }

    (
        Client {
            stream,
            frontend_id: hello.assigned_frontend_id,
        },
        grid,
    )
}

fn send_key(client: &mut Client, key: Key, mods: Modifiers) {
    write_message(
        &mut client.stream,
        &FrontendEvent::Key(KeyEvent {
            frontend_id: client.frontend_id,
            key,
            mods,
            timestamp_ns: 0,
        }),
    )
    .expect("send daemon key event");
}

fn send_ctrl_chord(client: &mut Client, second: char) {
    send_key(client, Key::Char('c'), Modifiers::CTRL);
    send_key(client, Key::Char(second), Modifiers::CTRL);
}

fn checkpoint(client: &mut Client, n: u8) {
    send_key(client, Key::F(n), Modifiers::NONE);
}

fn pump_grid_until(
    client: &mut Client,
    grid: &mut Grid,
    what: &str,
    predicate: impl Fn(&str) -> bool,
) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let text = grid.text();
        assert!(
            !text.contains("MSW_ERROR:"),
            "daemon-side Lua checkpoint failed:\n{text}"
        );
        if predicate(&text) {
            return text;
        }
        assert!(
            Instant::now() < deadline,
            "grid update timed out waiting for {what}; current grid:\n{text}"
        );
        if let Ok(InstanceMessage::CellDelta { spans, .. }) =
            read_message::<InstanceMessage>(&mut client.stream)
        {
            grid.apply(spans);
        }
    }
}

fn lua_string(path: &Path) -> String {
    format!("{:?}", path.to_string_lossy())
}

#[allow(
    clippy::too_many_lines,
    reason = "one ordered daemon session preserves dispatch state across all ten acceptance checks"
)]
#[test]
fn mode_system_wiring_is_observable_end_to_end() {
    let fixtures = tempfile::tempdir().expect("fixture tempdir");
    let rust = fixtures.path().join("dispatch.rs");
    let python = fixtures.path().join("dispatch.py");
    let unknown = fixtures.path().join("dispatch.txt");
    let server = fixtures.path().join("dispatch.msw");
    std::fs::write(&rust, "// MSW_RUST_FIXTURE\nfn main() {}\n").unwrap();
    std::fs::write(&python, "# MSW_PYTHON_FIXTURE\nprint('ok')\n").unwrap();
    std::fs::write(&unknown, "MSW_UNKNOWN_FIXTURE\n").unwrap();
    std::fs::write(&server, "MSW_SERVER_ONLY_FIXTURE\n").unwrap();

    let init_template = r#"
-- Ordinary fixtures must never inherit the built-in real-server registry.
pmacs.lsp.config = {}
pmacs.lsp.filetypes.msw = "serveronly"

local RUST_PATH = __RUST_PATH__
local PYTHON_PATH = __PYTHON_PATH__
local UNKNOWN_PATH = __UNKNOWN_PATH__
local SERVER_PATH = __SERVER_PATH__
local S = { rust_hits = 0 }
_G.MSW_STATE = S
_G.MSW_RESULT = false
_G.MSW_ERROR = nil

local function eq(actual, expected, label)
  assert(actual == expected,
    label .. ": expected " .. tostring(expected) .. ", got " .. tostring(actual))
end

local function current_modes(expected)
  local modes = pmacs.editor.active_modes()
  if expected == nil then
    eq(#modes, 0, "active mode count")
  else
    eq(#modes, 1, "active mode count")
    eq(modes[1], expected, "active mode")
  end
end

local function command(name, body)
  pmacs.command.define {
    name = name,
    description = "mode-system wiring acceptance command " .. name,
    fn = function()
      local ok, err = pcall(body)
      if not ok then
        _G.MSW_ERROR = ("MSW_ERROR:" .. tostring(err)):sub(1, 200)
      end
    end,
  }
end

local function global_key(sequence, name)
  pmacs.keymap.bind { scope = "global", sequence = sequence, command = name }
end

command("test.rust-only", function()
  S.rust_hits = S.rust_hits + 1
end)
command("test.priority-buffer", function() S.priority_hit = "buffer" end)
command("test.priority-mode", function() S.priority_hit = "mode" end)
command("test.priority-global", function() S.priority_hit = "global" end)
command("test.parity-mode", function() S.parity_hit = "mode" end)
command("test.parity-global", function() S.parity_hit = "global" end)

pmacs.keymap.bind {
  scope = "mode", mode = "rust", sequence = "C-c C-c", command = "test.rust-only",
}
pmacs.keymap.bind {
  scope = "mode", mode = "rust", sequence = "C-c C-p", command = "test.priority-mode",
}
pmacs.keymap.bind {
  scope = "global", sequence = "C-c C-p", command = "test.priority-global",
}
pmacs.keymap.bind {
  scope = "mode", mode = "rust", sequence = "C-c C-d", command = "test.parity-mode",
}
pmacs.keymap.bind {
  scope = "global", sequence = "C-c C-d", command = "test.parity-global",
}

pmacs.statusline.register {
  name = "mode-system-result",
  side = "right",
  priority = 999,
  face = "ui.modeline",
  fn = function()
    if _G.MSW_ERROR then return _G.MSW_ERROR end
    if _G.MSW_RESULT then return "MODE-SYSTEM-WIRING-PASS" end
    return nil
  end,
}

command("test.step-1", function()
  local provider
  for _, candidate in ipairs(pmacs.statusline.providers()) do
    if candidate.name == "mode" then provider = candidate end
  end
  assert(provider ~= nil, "built-in mode provider is registered")
  eq(provider.side, "left", "mode provider side")
  eq(provider.priority, 0, "mode provider priority")
  eq(provider.face, "ui.modeline", "mode provider face")
  eq(provider.enabled, true, "mode provider enabled")

  S.rust = pmacs.buffer.find_or_open(RUST_PATH)
  eq(pmacs.buffer.major_mode(S.rust), "rust", "rust initialization")
  current_modes("rust")

  S.python = pmacs.buffer.find_or_open(PYTHON_PATH)
  eq(pmacs.buffer.major_mode(S.python), "python", "python initialization")
  current_modes("python")

  S.unknown = pmacs.buffer.find_or_open(UNKNOWN_PATH)
  eq(pmacs.buffer.major_mode(S.unknown), nil, "unknown initialization")
  current_modes(nil)

  S.server = pmacs.buffer.find_or_open(SERVER_PATH)
  eq(pmacs.buffer.major_mode(S.server), "serveronly", "server-only initialization")
  current_modes("serveronly")
  eq(pmacs.parse._has_view(S.server), false, "server-only parse view")

  pmacs.keymap.bind {
    scope = "buffer", buffer = S.rust,
    sequence = "C-c C-p", command = "test.priority-buffer",
  }

  -- Leave a Rust-active/Python-passive split for real statusline evaluation.
  pmacs.window.switch_buffer(S.rust)
  pmacs.window.split_vertical()
  pmacs.window.focus_next()
  pmacs.window.switch_buffer(S.python)
  pmacs.window.focus_next()
  eq(tostring(pmacs.window.buffer()), tostring(S.rust), "focused split buffer")
  eq(pmacs.buffer.major_mode(S.rust), "rust", "rust survived switching")
  eq(pmacs.buffer.major_mode(S.python), "python", "python survived switching")
end)

command("test.step-2", function()
  eq(S.rust_hits, 1, "Rust mode dispatch")
  pmacs.window.switch_buffer(S.python)
  current_modes("python")
end)

command("test.step-3", function()
  eq(S.rust_hits, 1, "Python must not dispatch Rust binding")
  pmacs.window.switch_buffer(S.unknown)
  eq(pmacs.buffer.major_mode(S.unknown), nil, "unknown stays mode-less")
  current_modes(nil)
end)

command("test.step-4", function()
  eq(S.rust_hits, 1, "mode-less buffer must not dispatch Rust binding")
  pmacs.window.switch_buffer(S.rust)
  S.priority_hit = nil
end)

command("test.step-5", function()
  eq(S.priority_hit, "buffer", "buffer scope precedence")
  pmacs.keymap.unbind {
    scope = "buffer", buffer = S.rust, sequence = "C-c C-p",
  }
  S.priority_hit = nil
end)

command("test.step-6", function()
  eq(S.priority_hit, "mode", "mode scope precedence")
  pmacs.window.switch_buffer(S.python)
  S.priority_hit = nil
end)

command("test.step-7", function()
  eq(S.priority_hit, "global", "global scope fallback")
  pmacs.window.switch_buffer(S.rust)
  S.parity_hit = nil
end)

command("test.step-8", function()
  eq(S.parity_hit, "mode", "parity binding dispatch")

  local described = pmacs.describe.key("C-c C-d")
  assert(described ~= nil, "describe.key returns the mode binding")
  eq(described.command, "test.parity-mode", "describe.key command")
  eq(described.scope, "mode:rust", "describe.key scope")

  local help_id = pmacs.help.show_key("C-c C-d")
  assert(help_id ~= nil, "help.show_key returns a help buffer")
  local body = help_id:slice(0, help_id:len())
  assert(body:find("Runs: %[command: test%.parity%-mode%]"), body)
  assert(body:find("Scope: mode:rust", 1, true), body)

  help_id = pmacs.help.show_command("test.parity-mode")
  body = help_id:slice(0, help_id:len())
  local link_start = body:find("%[key: C%-c C%-d @mode:rust%]")
  assert(link_start ~= nil, "mode command help link carries @mode:rust: " .. body)
  pmacs.window.switch_buffer(help_id)
  local followed = pmacs.help.follow_link(link_start + 6)
  assert(followed ~= nil, "mode key link follows while *help* is active")
  local followed_body = followed:slice(0, followed:len())
  assert(followed_body:find("Runs: %[command: test%.parity%-mode%]"), followed_body)
  assert(followed_body:find("Scope: mode:rust", 1, true), followed_body)

  pmacs.window.switch_buffer(S.rust)
  pmacs.buffer.set_major_mode(S.rust, "markdown")
  pmacs.window.switch_buffer(S.python)
  pmacs.window.switch_buffer(S.rust)
  eq(pmacs.buffer.major_mode(S.rust), "markdown", "explicit override survives switches")
  current_modes("markdown")
end)

command("test.step-9", function()
  eq(pmacs.buffer.major_mode(S.rust), "markdown", "override remains live")
  pmacs.buffer.set_major_mode(S.rust, nil)
  pmacs.window.switch_buffer(S.python)
  pmacs.window.switch_buffer(S.rust)
  eq(pmacs.buffer.major_mode(S.rust), nil, "explicit clear survives switches")
  current_modes(nil)
  S.clear_baseline = S.rust_hits
end)

command("test.step-10", function()
  eq(S.rust_hits, S.clear_baseline, "cleared Rust mode must not dispatch")

  pmacs.window.switch_buffer(S.server)
  eq(pmacs.buffer.major_mode(S.server), "serveronly", "server-only mode survives")
  current_modes("serveronly")
  eq(pmacs.parse._has_view(S.server), false, "server-only language stays parser-free")

  pmacs.window.switch_buffer(S.unknown)
  eq(pmacs.buffer.major_mode(S.unknown), nil, "unknown mode remains nil")
  current_modes(nil)
  _G.MSW_RESULT = true
end)

for n = 1, 10 do
  global_key("<f" .. n .. ">", "test.step-" .. n)
end
"#;

    let init = init_template
        .replace("__RUST_PATH__", &lua_string(&rust))
        .replace("__PYTHON_PATH__", &lua_string(&python))
        .replace("__UNKNOWN_PATH__", &lua_string(&unknown))
        .replace("__SERVER_PATH__", &lua_string(&server));

    let daemon = TestDaemon::spawn_with_config(&init);
    let (mut client, mut grid) = attach(&daemon);

    // Initialize through real after-load hooks and leave Rust focused with
    // Python in the passive split. The daemon's ordinary grid painter must
    // render each window from its own buffer context.
    checkpoint(&mut client, 1);
    let split_text = pump_grid_until(
        &mut client,
        &mut grid,
        "Rust-active/Python-passive mode lines",
        |text| {
            text.contains("dispatch.rs")
                && text.contains("dispatch.py")
                && text.contains("(rust)")
                && text.contains("(python)")
        },
    );
    let split_modeline = split_text
        .lines()
        .find(|line| line.contains("dispatch.rs") && line.contains("dispatch.py"))
        .expect("both split mode lines occupy the split's modeline row");
    assert!(split_modeline.contains("(rust)"), "{split_modeline}");
    assert!(split_modeline.contains("(python)"), "{split_modeline}");

    // 1-2: the exact mode binding fires in Rust, not Python or no-mode text.
    send_ctrl_chord(&mut client, 'c');
    checkpoint(&mut client, 2);
    send_ctrl_chord(&mut client, 'c');
    checkpoint(&mut client, 3);
    send_ctrl_chord(&mut client, 'c');

    // The active unknown-language pane omits its mode segment while the
    // passive Python pane retains its own. This distinguishes empty output
    // from a provider accidentally reading the focused/other buffer.
    let unknown_text = pump_grid_until(&mut client, &mut grid, "mode-less active buffer", |text| {
        text.contains("dispatch.txt") && text.contains("dispatch.py")
    });
    let unknown_modeline = unknown_text
        .lines()
        .find(|line| line.contains("dispatch.txt") && line.contains("dispatch.py"))
        .expect("unknown and passive Python mode lines share a row");
    let passive_start = unknown_modeline
        .find("dispatch.py")
        .expect("passive Python buffer name");
    assert!(
        !unknown_modeline[..passive_start].contains('('),
        "unknown-language mode line must omit a mode segment: {unknown_modeline}"
    );
    assert!(
        unknown_modeline[passive_start..].contains("(python)"),
        "passive Python mode line keeps its own mode: {unknown_modeline}"
    );

    // 5: buffer-local, then mode, then global, all driven through dispatch.
    checkpoint(&mut client, 4);
    send_ctrl_chord(&mut client, 'p');
    checkpoint(&mut client, 5);
    send_ctrl_chord(&mut client, 'p');
    checkpoint(&mut client, 6);
    send_ctrl_chord(&mut client, 'p');

    // 10: first prove the mode binding dispatched, then compare describe,
    // show-key, and followed @mode link rendering against that result.
    checkpoint(&mut client, 7);
    send_ctrl_chord(&mut client, 'd');
    checkpoint(&mut client, 8);

    // 7-8: override and clear each survive switch-away/back; the cleared mode
    // no longer dispatches the detected-language binding.
    checkpoint(&mut client, 9);
    send_ctrl_chord(&mut client, 'c');
    checkpoint(&mut client, 10);

    // The marker is painted only if every Lua assertion, including
    // active_modes and server-only parse gating, completed successfully.
    pump_grid_until(
        &mut client,
        &mut grid,
        "mode-system success marker",
        |text| text.contains("MODE-SYSTEM-WIRING-PASS"),
    );
}
