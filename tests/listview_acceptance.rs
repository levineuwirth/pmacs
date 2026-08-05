//! List-panel acceptance (Arc 1b phase 1) --- the `pmacs.listview`
//! module end-to-end through `dispatch_key`: open/navigate/visit,
//! `q` restore, the Q#P3 read-only intercept, the Q#P6 round-trip
//! gate (`dispatch_idle` false while a panel is focused), and
//! refresh. The references panel itself needs a live LSP and is
//! validated manually / via the m4 harness; these tests drive the
//! substrate hermetically.
//!
//! Framing: docs/lsp-panels-framing.md.
//!
//! Generated-buffer immutability Stage 1
//! (docs/generated-buffer-immutability-framing.md §6) adds the
//! criteria below `refresh_reruns_the_source_and_reseats`: the undo
//! paths the Q#P3 intercept never guarded, the Q#GB13 ownership rule,
//! and the Q#GB18 identity routing that ownership makes load-bearing.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use pmacs::buffer::BufferId;
use pmacs::editor::EditorState;
use pmacs::protocol::{CellSize, FrontendId};

/// Bottom-panel Stage 3: a listview now opens into the PANEL by default,
/// and a panel is derived-hidden while the frontend's frame geometry is
/// unknown — so focus would fall back to the document window and every
/// panel assertion here would read the wrong buffer.
///
/// `bottom_panel_stage1_acceptance` has always declared geometry for the
/// same reason: a grid frontend's real frame size IS its declaration,
/// and every test that does not render must state it before any input.
/// This suite never needed to while listview defaulted to the current
/// window. It does now.
fn editor() -> EditorState {
    let s = EditorState::new_with_roots(&crate::iso::roots());
    s.sync_frame_geometry(FrontendId::LOCAL, CellSize::new(24, 80));
    s
}

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: mods,
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    }
}

fn press(s: &mut EditorState, code: KeyCode) {
    s.dispatch_key(FrontendId::LOCAL, key(code, KeyModifiers::NONE));
}

fn ctrl(s: &mut EditorState, c: char) {
    s.dispatch_key(
        FrontendId::LOCAL,
        key(KeyCode::Char(c), KeyModifiers::CONTROL),
    );
}

fn alt(s: &mut EditorState, c: char) {
    s.dispatch_key(FrontendId::LOCAL, key(KeyCode::Char(c), KeyModifiers::ALT));
}

fn type_str(s: &mut EditorState, text: &str) {
    for ch in text.chars() {
        s.dispatch_key(
            FrontendId::LOCAL,
            key(KeyCode::Char(ch), KeyModifiers::NONE),
        );
    }
}

/// `M-x <name> RET` through the **real** minibuffer, not
/// `pmacs.command.invoke`: `buffer.undo` is reachable that way on every
/// buffer in the tree and no buffer-local rebinding can remove it, which
/// is the whole reason the intercept idiom did not close this hole.
fn m_x(s: &mut EditorState, name: &str) {
    alt(s, 'x');
    type_str(s, name);
    press(s, KeyCode::Enter);
}

fn exec(s: &EditorState, src: &str) {
    s.lua_host.lua().load(src.to_string()).exec().unwrap();
}

fn eval<T: mlua::FromLuaMulti>(s: &EditorState, src: &str) -> T {
    s.lua_host.lua().load(src.to_string()).eval().unwrap()
}

fn status(s: &EditorState) -> String {
    s.core.borrow().status.clone()
}

fn buffer_names(s: &EditorState) -> Vec<String> {
    eval(
        s,
        "local out = {}\n\
         for _, id in ipairs(pmacs.buffer.list()) do\n\
           out[#out + 1] = pmacs.describe.buffer(id).name\n\
         end\n\
         return out",
    )
}

fn active_name(s: &EditorState) -> String {
    eval(
        s,
        "return pmacs.describe.buffer(pmacs.window.buffer()).name",
    )
}

fn active_text(s: &EditorState) -> String {
    eval(
        s,
        "local b = pmacs.window.buffer()\nreturn b:slice(0, b:len())",
    )
}

fn id_of(s: &EditorState, name: &str) -> BufferId {
    let core = s.core.borrow();
    let reg = core.registry.borrow();
    reg.find_by_name(name)
        .unwrap_or_else(|| panic!("no buffer named {name:?} in {:?}", buffer_names(s)))
}

/// Q#GB14: the rope lock is not observable from Lua --- `describe.buffer`
/// carries `name`, `length`, `modified`, `view_count` and nothing else ---
/// so every "is it locked" assertion goes through Rust.
fn is_read_only(s: &EditorState, id: BufferId) -> bool {
    let core = s.core.borrow();
    let reg = core.registry.borrow();
    reg.get(id).expect("buffer in registry").is_read_only()
}

fn set_read_only(s: &EditorState, id: BufferId, value: bool) {
    let core = s.core.borrow();
    let mut reg = core.registry.borrow_mut();
    reg.get_mut(id)
        .expect("buffer in registry")
        .set_read_only(value);
}

/// Render the active window's text view into a cell grid. Criterion 7 is
/// pinned by PAINTING, because that is where a rope/window disagreement
/// bites: the rope is right and the screen is not.
fn paint_active_window(s: &EditorState, rows: u32, cols: u32) -> Vec<pmacs::cell::Cell> {
    use pmacs::cell::{Cell, CellGrid, CellSize};
    use pmacs::view::{View, Viewport};
    use pmacs::window::Rect;

    let mut core = s.core.borrow_mut();
    let active = core.active_window_id();
    let registry = core.registry.clone();
    let win = core.windows.get_mut(&active).expect("active window");
    let rect = Rect::new(0, 0, rows, cols);
    let mut backing = vec![Cell::default(); (rows * cols) as usize];
    let reg = registry.borrow();
    let buf = reg.get(win.buffer_id).expect("buffer in registry");
    let viewport = Viewport {
        buffer_start: 0,
        buffer_end: buf.len(),
        cell_origin: rect.origin,
        cell_size: CellSize::new(rows, cols),
        gutter_w: 0,
        folds: None,
    };
    let mut grid = CellGrid {
        cells: &mut backing,
        stride: cols,
        size: CellSize::new(rows, cols),
    };
    win.text_view.render(buf, viewport, &mut grid);
    backing
}

fn grid_row(cells: &[pmacs::cell::Cell], row: u32, cols: u32) -> String {
    use pmacs::cell::Glyph;
    (0..cols)
        .map(|c| match cells[(row * cols + c) as usize].glyph {
            Glyph::Char(ch) => ch,
            _ => ' ',
        })
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// Open a three-row test panel whose visits record into `_G.VISITED`.
fn open_test_panel(s: &mut EditorState) {
    s.lua_host
        .lua()
        .load(
            r#"
            _G.VISITED = nil
            pmacs.listview.open {
              name = "*test-panel*",
              header = "3 items   RET visit  q quit",
              rows = {
                { text = "alpha", item = "A" },
                { text = "beta",  item = "B" },
                { text = "gamma", item = "C" },
              },
              on_visit = function(item) _G.VISITED = item end,
              on_refresh = function()
                return { { text = "delta", item = "D" } }
              end,
            }
            "#,
        )
        .exec()
        .expect("open test panel");
}

/// Open the fixture panel with an explicit `display = "current"`, for
/// the tests whose subject requires it to sit in a document window.
fn open_test_panel_in_document(s: &mut EditorState) {
    s.lua_host
        .lua()
        .load(
            r#"
            _G.VISITED = nil
            pmacs.listview.open {
              name = "*test-panel*",
              header = "3 items   RET visit  q quit",
              display = "current",
              rows = {
                { text = "alpha", item = "A" },
                { text = "beta",  item = "B" },
                { text = "gamma", item = "C" },
              },
              on_visit = function(item) _G.VISITED = item end,
            }
            "#,
        )
        .exec()
        .expect("open test panel in a document window");
}

/// `(active buffer name, buffer text, cursor line, visited)` probed
/// through the Lua surface.
fn probe(s: &EditorState) -> (String, String, i64, Option<String>) {
    s.lua_host
        .lua()
        .load(
            r"
            local b = pmacs.window.buffer()
            local d = pmacs.describe.buffer(b)
            return d.name, b:slice(0, b:len()), pmacs.editor.cursor_line(), _G.VISITED
            ",
        )
        .eval()
        .expect("probe panel state")
}

#[test]
fn open_seats_cursor_and_ret_visits_the_row() {
    let mut s = editor();
    open_test_panel(&mut s);
    let (name, text, line, _) = probe(&s);
    assert_eq!(name, "*test-panel*");
    assert!(text.starts_with("3 items"), "header renders first");
    assert_eq!(line, 1, "the cursor opens on the first data row");

    press(&mut s, KeyCode::Char('n')); // buffer-local: cursor.down
    press(&mut s, KeyCode::Enter);
    let (_, _, _, visited) = probe(&s);
    assert_eq!(visited.as_deref(), Some("B"), "RET visits the second row");
}

#[test]
fn header_row_is_not_visitable() {
    let mut s = editor();
    open_test_panel(&mut s);
    press(&mut s, KeyCode::Char('p')); // up onto the header
    press(&mut s, KeyCode::Enter);
    let (_, _, _, visited) = probe(&s);
    assert_eq!(visited, None, "the header maps to no item");
}

#[test]
fn q_restores_the_previous_buffer() {
    let mut s = editor();
    open_test_panel(&mut s);
    press(&mut s, KeyCode::Char('q'));
    let (name, _, _, _) = probe(&s);
    assert_eq!(name, "*scratch*", "q returns to the buffer we came from");
}

#[test]
fn panel_rejects_typing() {
    let mut s = editor();
    open_test_panel(&mut s);
    let (_, before, _, _) = probe(&s);
    press(&mut s, KeyCode::Char('z')); // unbound printable → self-insert → intercept rejects
    let (_, after, _, _) = probe(&s);
    assert_eq!(before, after, "the read-only intercept rejects self-insert");
}

#[test]
fn dispatch_idle_is_false_while_a_panel_is_focused() {
    // Q#P6: while the panel is the active buffer, semantic frontends
    // must round-trip every key (RET = visit, not an optimistic \n).
    let mut s = editor();
    assert!(s.dispatch_idle(), "scratch buffer: idle");
    open_test_panel(&mut s);
    assert!(!s.dispatch_idle(), "panel focused: keys must round-trip");
    press(&mut s, KeyCode::Char('q'));
    assert!(s.dispatch_idle(), "restored buffer: idle again");
}

#[test]
fn refresh_reruns_the_source_and_reseats() {
    let mut s = editor();
    open_test_panel(&mut s);
    press(&mut s, KeyCode::Char('g'));
    let (_, text, line, _) = probe(&s);
    assert!(text.contains("delta"), "g re-renders from on_refresh");
    assert!(!text.contains("alpha"), "old rows are gone");
    assert_eq!(line, 1, "cursor re-seats on a data row after refresh");
    press(&mut s, KeyCode::Enter);
    let (_, _, _, visited) = probe(&s);
    assert_eq!(visited.as_deref(), Some("D"), "the refreshed row visits");
}

// ---------------------------------------------------------------------------
// Generated-buffer immutability, Stage 1
// (docs/generated-buffer-immutability-framing.md §6, Stage 1)
// ---------------------------------------------------------------------------

/// The exact bytes `open_test_panel` renders. Asserted by value, not by
/// `is_empty()`: "the panel is not empty" is the assertion shape the
/// framing's §0.1 shows passing with the bug live on other families.
const PANEL_TEXT: &str = "3 items   RET visit  q quit\nalpha\nbeta\ngamma";

/// Criterion 1 [`main`] --- `C-/` cannot empty a listview panel.
///
/// Driven by a real chord through `dispatch_key`, because listview
/// rebinds **no** undo chord (`grep -n 'C-/\|C-_\|C-x u\|undo'
/// builtin/runtime/listview.lua` is empty), so this is the whole
/// distance from a keystroke to an empty panel.
///
/// *Bite:* measured on the pre-image --- `"H\nrow-one\nrow-two"` -> `""`.
/// `scripts/bite githubsucks/main builtin/runtime/listview.lua` falsifies
/// it: the panel's own render pushes a poppable undo entry, and
/// `Buffer::undo` reaches the rope through `ensure_writable` without ever
/// consulting the intercept chain.
#[test]
fn s1_1_the_undo_chord_cannot_empty_a_listview_panel() {
    let mut s = editor();
    open_test_panel(&mut s);
    assert_eq!(active_text(&s), PANEL_TEXT, "precondition: rendered");

    ctrl(&mut s, '/');

    assert_eq!(
        active_text(&s),
        PANEL_TEXT,
        "C-/ must leave the panel's content intact"
    );
}

/// Criterion 2 [`main`] --- `M-x buffer.undo` cannot empty a listview
/// panel, driven through the **real** minibuffer.
///
/// Separate from criterion 1 on purpose: a fix that only rebound the
/// chords would pass 1 and fail this. `compile.lua`'s own comment already
/// concedes the point ("command/menu undo stays dispatchable").
///
/// *Bite:* same empty result on the pre-image.
#[test]
fn s1_2_m_x_buffer_undo_cannot_empty_a_listview_panel() {
    let mut s = editor();
    open_test_panel(&mut s);

    m_x(&mut s, "buffer.undo");

    assert_eq!(
        active_name(&s),
        "*test-panel*",
        "the minibuffer round trip must land back in the panel"
    );
    assert_eq!(
        active_text(&s),
        PANEL_TEXT,
        "M-x buffer.undo must leave the panel's content intact"
    );
}

/// Criterion 4 [fix-shape] --- the owner's own refresh still works after
/// the lock, and the buffer is still locked afterwards.
///
/// *Bite:* a naive `set_read_only(true)` at panel creation passes
/// criteria 1-3 and fails here, because it refuses the refresh the panel
/// exists for. That is the failure mode `Buffer::set_generated_contents`
/// exists to prevent, and it is why the **pairing** is the primitive.
/// The assertion is on the content `g` produced, not on the call not
/// raising.
#[test]
fn s1_4_the_owners_refresh_still_works_after_the_lock() {
    let mut s = editor();
    open_test_panel(&mut s);
    let panel = id_of(&s, "*test-panel*");
    assert!(
        is_read_only(&s, panel),
        "precondition: the first render locked the rope"
    );

    press(&mut s, KeyCode::Char('g'));

    let text = active_text(&s);
    assert!(text.contains("delta"), "g must re-render: {text:?}");
    assert!(
        !text.contains("alpha"),
        "and replace the old rows: {text:?}"
    );
    assert!(
        is_read_only(&s, panel),
        "and the panel must still be locked afterwards"
    );
}

/// Stage 1 criterion 5 [`main`, and also fix-shape] — the rope lock
/// refuses an ordinary edit first, and the named intercept survives
/// behind it.
///
/// Both halves are required. The first asserts the exact
/// `BufferError::ReadOnly` rendering and byte identity. The second lifts
/// the lock Rust-side and distinguishes the intercept by its
/// `intercept rejected the edit` message. Deleting `add_intercept`
/// therefore passes the rope half and fails the lifted half.
#[test]
fn s1_5_the_rope_lock_and_named_intercept_refuse_in_order() {
    let mut s = editor();
    open_test_panel(&mut s);
    let panel = id_of(&s, "*test-panel*");
    let before = active_text(&s);

    press(&mut s, KeyCode::Char('z'));
    assert_eq!(
        status(&s),
        format!("insert failed: buffer `*test-panel*` (id {panel:?}) is read-only"),
        "with the lock on, the rope must provide the exact refusal"
    );
    assert_eq!(
        active_text(&s),
        before,
        "the rope refusal leaves every byte unchanged"
    );

    set_read_only(&s, panel, false);
    press(&mut s, KeyCode::Char('z'));
    set_read_only(&s, panel, true);

    assert_eq!(
        active_text(&s),
        PANEL_TEXT,
        "the intercept must refuse the edit even with the rope writable"
    );
    let st = status(&s);
    assert!(
        st.starts_with("insert failed: intercept rejected the edit:")
            && st.contains("listview.lua")
            && st.contains("*test-panel* is read-only"),
        "the lifted path must carry the named intercept refusal; got {st:?}"
    );
    assert!(
        !st.contains(&format!(
            "buffer `*test-panel*` (id {panel:?}) is read-only"
        )),
        "the lifted path must not masquerade as the rope refusal: {st:?}"
    );
}

/// Criterion 6 [fix-shape] --- `set_round_trip_input` survives adoption,
/// asserted so that only the round-trip mark can make it pass.
///
/// `dispatch_idle_for` (`src/editor.rs:1126-1155`) returns `false` for
/// **six** independent reasons, so `!dispatch_idle_for(..)` alone is
/// satisfied by any of them. All three halves are required:
///
/// * **(a)** the document-window premise (`!window.is_side()`), so a
///   fixture that later displays the panel in a side window fails loudly
///   rather than passing vacuously --- `tests/dired_acceptance.rs:975`'s
///   shape;
/// * **(b)** the gate itself while the panel is focused;
/// * **(c)** the positive control --- switching the same window to a
///   plain buffer must flip the gate back to `true`. A stuck minibuffer,
///   a pending chord, an open menu or a live search would keep it `false`
///   across the switch, so (c) failing is the signal that (b) passed for
///   the wrong reason.
///
/// *Bite:* delete the `set_round_trip_input` call in `listview.lua` and
/// criteria 1-5 all still pass; only (b) fails. A daemon-side rope
/// refusal does nothing for a replica's own mirror, which is why this is
/// pinned through `dispatch_idle_for` rather than through `read_only`.
#[test]
fn s1_6_round_trip_input_survives_the_adoption() {
    let mut s = editor();
    // Stage 3: an EXPLICIT opt-out, because this fixture genuinely needs
    // the document window. `dispatch_idle` goes false for TWO reasons —
    // a round-trip buffer (this test's subject) and a focused panel
    // (`dispatch_idle_is_false_while_a_panel_is_focused`, a different
    // test). Letting the panel default apply here would satisfy the gate
    // for the wrong reason and the test would pass while proving
    // nothing. The premise assertion below is what keeps that honest.
    open_test_panel_in_document(&mut s);

    // (a) the premise.
    {
        let core = s.core.borrow();
        let active = core.active_window_id();
        assert!(
            !core.windows.get(&active).expect("live window").is_side(),
            "fixture premise: the panel is in a document window here"
        );
    }
    // (b) the gate.
    assert!(
        !s.dispatch_idle_for(FrontendId::LOCAL),
        "a round-trip buffer must turn optimistic apply OFF"
    );
    // (c) the positive control.
    exec(
        &s,
        "pmacs.window.switch_buffer(pmacs.buffer.create('*plain*'))",
    );
    assert!(
        s.dispatch_idle_for(FrontendId::LOCAL),
        "and back ON for a plain buffer --- otherwise (b) passed for one \
         of the other five reasons"
    );
}

/// **Stage 1 criterion 7 [mutation]**, the listview half: *a refresh
/// reaches the window, not just the rope --- pinned by painting a
/// shrinking render (many rows -> one) and asserting row 1 is empty, for
/// each adopter.* Bite: *delete the `notify_buffer_edit_to_windows` call
/// in the `set_generated_contents` binding
/// (`src/lua_bindings/mod.rs:3092`).*
///
/// The criterion says **for each adopter**, so both halves exist; the
/// dired half is `dired_acceptance::dired_a_shrinking_repaint_reaches_the_window`.
///
/// `listview.refresh` re-seats through the already-notified `TextView`;
/// it deliberately does not rebuild the view by switching to the buffer
/// it already shows. Deleting the notification fan-out therefore leaves
/// the old line index live and this paint assertion bites.
#[test]
fn s1_7_a_shrinking_refresh_reaches_the_window() {
    let mut s = editor();
    open_test_panel(&mut s);
    let painted = paint_active_window(&s, 6, 24);
    assert_eq!(
        grid_row(&painted, 1, 24),
        "alpha",
        "precondition: three data rows paint"
    );
    assert_eq!(grid_row(&painted, 3, 24), "gamma");

    // `g` re-renders from three rows to one.
    press(&mut s, KeyCode::Char('g'));

    let painted = paint_active_window(&s, 6, 24);
    assert_eq!(
        grid_row(&painted, 1, 24),
        "delta",
        "the refreshed row must paint"
    );
    assert_eq!(
        grid_row(&painted, 2, 24),
        "",
        "and nothing of the rows it replaced"
    );
}

/// Criterion 9 [`main`] --- a foreign buffer that happens to share the
/// panel's name is never adopted (Q#GB13).
///
/// Both halves, because the second is what fails if adoption is merely
/// made "safe" by skipping the render: the user's bytes survive **and**
/// an ordinary edit to the user's buffer still lands. Adoption installed
/// an erroring intercept whose handle it discarded, so the pre-image left
/// the clobbered buffer permanently un-editable --- and this arc removes
/// the `M-x buffer.undo` that was the only way back.
///
/// *Bite:* measured on the pre-image --- `"my precious notes"` ->
/// `"H\nr1"`, one buffer where there should be two, and the user's buffer
/// left un-editable. `scripts/bite githubsucks/main
/// builtin/runtime/listview.lua` falsifies it.
#[test]
fn s1_9_a_foreign_buffer_with_the_panels_name_is_never_adopted() {
    let mut s = editor();
    exec(
        &s,
        "FOREIGN = pmacs.buffer.create('*test-panel*')\n\
         FOREIGN:insert(0, 'my precious notes')",
    );

    open_test_panel(&mut s);

    let foreign: String = eval(&s, "return FOREIGN:slice(0, FOREIGN:len())");
    assert_eq!(
        foreign, "my precious notes",
        "the user's bytes must survive the panel opening"
    );
    let landed: String = eval(
        &s,
        "FOREIGN:insert(0, 'still mine: ')\n\
         return FOREIGN:slice(0, FOREIGN:len())",
    );
    assert_eq!(
        landed, "still mine: my precious notes",
        "and an ordinary edit to it must still land"
    );
    assert_eq!(
        active_name(&s),
        "*test-panel*<2>",
        "the panel opens under a disambiguated name"
    );
    let names = buffer_names(&s);
    assert!(
        names.iter().any(|n| n == "*test-panel*") && names.iter().any(|n| n == "*test-panel*<2>"),
        "two buffers, not one: {names:?}"
    );
}

/// Criterion 10 [fix-shape] --- exhausting the disambiguation limit
/// raises rather than falling back to adoption, matching
/// `dired.lua:493-503` and `terminal.lua:309-315`.
///
/// *Bite:* an implementation that adopts once `<99>` is taken passes
/// criterion 9 and fails here --- and it fails in the worst direction,
/// because the buffer it would adopt is by construction one a user
/// created.
#[test]
fn s1_10_the_disambiguation_limit_raises_rather_than_adopting() {
    let s = editor();
    exec(
        &s,
        "MINE = pmacs.buffer.create('*test-panel*')\n\
         MINE:insert(0, 'mine')\n\
         for i = 2, 99 do pmacs.buffer.create(string.format('*test-panel*<%d>', i)) end",
    );

    let (ok, err): (bool, String) = eval(
        &s,
        "local ok, err = pcall(pmacs.listview.open, { name = '*test-panel*', rows = {} })\n\
         return ok, tostring(err)",
    );

    assert!(!ok, "the open must raise, not adopt");
    assert!(
        err.contains("no free variant remains"),
        "and say why; got {err:?}"
    );
    let mine: String = eval(&s, "return MINE:slice(0, MINE:len())");
    assert_eq!(mine, "mine", "and touch nothing");
}

/// Stage 1 criterion 11 [`main`] — a **disambiguated** panel still answers
/// `RET`, `g` and `q` (Q#GB18). The framing labels it `[main]` and names
/// its bite as *Q#GB13 landed without Q#GB18*.
///
/// On `main` the test first fails at the disambiguation premise because
/// ownership is absent. The framing's narrower pre-image is also pinned:
/// keep disambiguation but restore a name-keyed `panel_for_buffer`, and
/// the test reaches the consumer checks and fails at `g`.
///
/// Disambiguation alone leaves the old lookup reading
/// `panels["*test-panel*<2>"]` for a record stored under
/// `"*test-panel*"`, so all three commands return early. Every one of
/// them fails **silently**, so the assertion is on the content each
/// command produced, never on "it did not raise".
#[test]
fn s1_11_a_disambiguated_panel_still_answers_ret_g_and_q() {
    let mut s = editor();
    exec(
        &s,
        "FOREIGN = pmacs.buffer.create('*test-panel*')\n\
         ORIGIN = pmacs.buffer.create('*origin*')\n\
         pmacs.window.switch_buffer(ORIGIN)",
    );
    open_test_panel(&mut s);
    assert_eq!(active_name(&s), "*test-panel*<2>", "premise: disambiguated");

    // g --- re-render from the data source.
    press(&mut s, KeyCode::Char('g'));
    let text = active_text(&s);
    assert!(text.contains("delta"), "g must re-render: {text:?}");

    // RET --- fire on_visit for the row under the cursor.
    press(&mut s, KeyCode::Enter);
    let visited: Option<String> = eval(&s, "return _G.VISITED");
    assert_eq!(
        visited.as_deref(),
        Some("D"),
        "RET must visit the refreshed row"
    );

    // q --- restore the buffer the panel was opened from.
    press(&mut s, KeyCode::Char('q'));
    assert_eq!(
        active_name(&s),
        "*origin*",
        "q must restore the previous buffer"
    );
}

/// Stage 1 criterion 12 [`main`] — the `q`-target capture is not inverted
/// (Q#GB18), which needs its own criterion because it fails **open**
/// rather than closed. The framing labels it `[main]`.
///
/// On `main` the ownership premise fails first. With disambiguation kept
/// and only `panel_for_buffer` restored to name-keyed lookup, the test
/// reaches and fails the `q`-target assertion the criterion exists for.
///
/// `listview.open`'s guard reads "capture the current buffer as the `q`
/// target, but never another panel (chained panels would trap `q` in a
/// loop)". When the lookup cannot recognise a disambiguated panel it
/// returns nil, the guard reads "not a panel", and the panel is captured
/// as the next panel's `q` target --- producing exactly the loop the
/// guard exists to prevent. Criterion 11 passes with that bug live,
/// because each command works in isolation; only the two-panel sequence
/// shows it.
///
/// *Bite:* restore the name-keyed `panels[d.name]` lookup while keeping
/// the disambiguation and `q` lands back in `*test-panel*<2>`.
/// Bottom-panel Stage 3 — the SIDE-WINDOW half of `q`, complementary to
/// `s1_12`'s buffer-level `p.prev` rule.
///
/// The parent framing's criterion 20 requires listview `q` to route
/// through `window.quit`, with `C → B → A` restoring each prior
/// presentation and the first panel deleting its wrapper. Before Stage 3
/// this was unreachable from listview's own entry point without an
/// explicit `display = "panel"` on every open; the default flip makes it
/// the ordinary path, so it gets an ordinary-path test.
///
/// The two mechanisms are complementary, not competing: **presentation
/// history chains in the side slot**, while **`p.prev` prevents
/// raw-switch and capability-fallback listview loops**. `s1_12` pins the
/// second by keeping its panels in document windows; this pins the
/// first.
/// Tree primitive — a panel with `depth` + `id` collapses and expands,
/// and BOTH the collapse state and the selection survive a re-render.
///
/// Acceptance 2 and 3. The re-render is what the primitive controls;
/// `g` refresh is deliberately out of scope because the anchor consumer
/// (the outline) has no `on_refresh` at all — see the framing's §1.5a.
#[test]
fn tr_1_collapse_hides_descendants_and_survives_re_render() {
    let mut s = editor();
    exec(
        &s,
        r#"pmacs.listview.open {
             name = "*tree*",
             header = "tree   TAB fold",
             rows = {
               { text = "root",   item = "root", depth = 0, id = "a" },
               { text = "  kid1", item = "kid1", depth = 1, id = "b" },
               { text = "  kid2", item = "kid2", depth = 1, id = "c" },
               { text = "tail",   item = "tail", depth = 0, id = "d" },
             },
           }"#,
    );
    let body = |s: &EditorState| active_text(s);
    assert!(
        body(&s).contains("kid1"),
        "children visible before collapse"
    );

    // Cursor opens on the first data row (root); TAB collapses it.
    press(&mut s, KeyCode::Tab);
    let collapsed = body(&s);
    assert!(
        !collapsed.contains("kid1"),
        "descendants hidden: {collapsed}"
    );
    assert!(
        !collapsed.contains("kid2"),
        "ALL descendants hidden: {collapsed}"
    );
    assert!(
        collapsed.contains("root") && collapsed.contains("tail"),
        "the node itself and its SIBLING survive — collapse hides \
         descendants, not the following run: {collapsed}"
    );

    // Selection is re-seated by ID, so the cursor is still on `root`.
    let on_root: String = eval(
        &s,
        "return pmacs.describe.buffer(pmacs.window.buffer()).name",
    );
    assert_eq!(on_root, "*tree*");

    press(&mut s, KeyCode::Tab);
    assert!(
        body(&s).contains("kid1") && body(&s).contains("kid2"),
        "TAB again expands"
    );
}

/// Selection survives a re-render that MOVES the selected node.
///
/// `tr_1` is not sufficient for this and was vacuous as a selection
/// test: it toggles the ROOT, which occupies line 1 before and after the
/// collapse, so the old line-based re-seating would pass it unchanged.
/// A selection test has to move the node.
///
/// Here `on_refresh` inserts a child ABOVE the selected sibling, so the
/// sibling's line shifts. Re-seating by line would land on the inserted
/// row; re-seating by id stays on the sibling.
#[test]
fn tr_4_selection_follows_the_node_when_rows_are_inserted_above_it() {
    let mut s = editor();
    exec(
        &s,
        r#"_G.EXTRA = false
           pmacs.listview.open {
             name = "*tree*", header = "tree",
             rows = {
               { text = "root",    item = "root",    depth = 0, id = "a" },
               { text = "  kid",   item = "kid",     depth = 1, id = "b" },
               { text = "sibling", item = "sibling", depth = 0, id = "z" },
             },
             on_refresh = function()
               if _G.EXTRA then
                 return {
                   { text = "root",     item = "root",  depth = 0, id = "a" },
                   { text = "  kid",    item = "kid",   depth = 1, id = "b" },
                   { text = "  kid2",   item = "kid2",  depth = 1, id = "c" },
                   { text = "sibling",  item = "sib",   depth = 0, id = "z" },
                 }
               end
               return {
                 { text = "root",    item = "root", depth = 0, id = "a" },
                 { text = "  kid",   item = "kid",  depth = 1, id = "b" },
                 { text = "sibling", item = "sib",  depth = 0, id = "z" },
               }
             end,
           }"#,
    );

    // Select `sibling` — data line 3.
    press(&mut s, KeyCode::Char('n'));
    press(&mut s, KeyCode::Char('n'));
    let line_before: i64 = eval(&s, "return pmacs.editor.cursor_line()");
    let text_at = |s: &EditorState| -> String {
        let body = active_text(s);
        let line: i64 = eval(s, "return pmacs.editor.cursor_line()");
        body.lines()
            .nth(usize::try_from(line).expect("line fits"))
            .unwrap_or_default()
            .to_string()
    };
    assert_eq!(text_at(&s), "sibling", "premise: sibling is selected");

    // Refresh inserts `kid2` ABOVE sibling, so its line moves.
    exec(&s, "_G.EXTRA = true");
    press(&mut s, KeyCode::Char('g'));

    let line_after: i64 = eval(&s, "return pmacs.editor.cursor_line()");
    // Substantive claim first, so a regression reports as what it is.
    // Under line-based re-seating the cursor stays on line 3, which now
    // holds the INSERTED row.
    assert_eq!(
        text_at(&s),
        "sibling",
        "selection follows the NODE, not the line"
    );
    // …and the fixture really did move it, so the assertion above is not
    // satisfied by the node happening to stay put (which is exactly how
    // `tr_1` is vacuous as a selection test).
    assert_ne!(
        line_before, line_after,
        "fixture: the insert must move the selected node"
    );
}

/// A leaf reports rather than silently doing nothing.
///
/// The outline's `g` is already a dead binding — bound, dispatched, no
/// feedback (framing §1.3a). This primitive must not add a second one.
#[test]
fn tr_2_toggling_a_leaf_reports_instead_of_silently_doing_nothing() {
    let mut s = editor();
    exec(
        &s,
        r#"pmacs.listview.open {
             name = "*tree*", header = "tree",
             rows = { { text = "leaf", item = "leaf", depth = 0, id = "only" } },
           }"#,
    );
    press(&mut s, KeyCode::Tab);
    assert!(
        status(&s).contains("no children"),
        "a leaf toggle says so; got: {}",
        status(&s)
    );
}

/// Rows WITHOUT `depth`/`id` behave exactly as before — the property
/// that keeps the three flat consumers unaffected (acceptance 5).
#[test]
fn tr_3_a_flat_panel_is_untouched_by_the_tree_extension() {
    let mut s = editor();
    exec(
        &s,
        r#"pmacs.listview.open {
             name = "*flat*", header = "flat",
             rows = { { text = "one", item = 1 }, { text = "two", item = 2 } },
           }"#,
    );
    let before = active_text(&s);
    let status_before = status(&s);
    press(&mut s, KeyCode::Tab);
    assert_eq!(
        active_text(&s),
        before,
        "TAB on a depthless panel changes nothing"
    );
    // Byte-identity of the BUFFER is not enough: TAB is bound for every
    // listview, so the tree command intercepts a key that previously
    // fell through to `buffer.tab` and the read-only intercept. A
    // listview-specific status here would be a behaviour change the
    // flat consumers never had, and invisible to a buffer comparison.
    assert!(
        !status(&s).contains("no node here") && !status(&s).contains("no children"),
        "a flat panel must not gain tree feedback; status was {:?} (was {:?})",
        status(&s),
        status_before
    );
    assert!(
        before.contains("one") && before.contains("two"),
        "both rows render: {before}"
    );
}

#[test]
fn s3_1_q_walks_the_side_presentation_chain_back_to_the_document() {
    let mut s = editor();
    exec(
        &s,
        "ORIGIN = pmacs.buffer.create('*origin*')\n\
              pmacs.window.switch_buffer(ORIGIN)",
    );
    assert_eq!(active_name(&s), "*origin*", "premise: a document window");

    for name in ["*panel-a*", "*panel-b*", "*panel-c*"] {
        exec(
            &s,
            &format!(
                "pmacs.listview.open {{ name = '{name}', header = 'H', \
                 rows = {{ {{ text = 'x', item = 'X' }} }} }}"
            ),
        );
        assert_eq!(active_name(&s), name, "each open takes the panel slot");
    }

    // C → B → A: each `q` restores the presentation the next one
    // replaced, rather than forgetting them or jumping straight out.
    press(&mut s, KeyCode::Char('q'));
    assert_eq!(
        active_name(&s),
        "*panel-b*",
        "q restores the replaced panel"
    );
    press(&mut s, KeyCode::Char('q'));
    assert_eq!(active_name(&s), "*panel-a*", "…and again, in order");

    // A → delete: the FIRST panel deletes its wrapper and focus lands
    // back in the document. This is what bounds the chain — a loop
    // between panels would never reach here.
    press(&mut s, KeyCode::Char('q'));
    assert_eq!(
        active_name(&s),
        "*origin*",
        "the last q deletes the wrapper and returns to the document"
    );
    assert!(
        s.core.borrow().windows.values().all(|w| !w.is_side()),
        "the side wrapper is collapsed, not left empty"
    );
}

#[test]
fn s1_12_the_q_target_capture_is_not_inverted_across_two_panels() {
    let mut s = editor();
    exec(
        &s,
        "FOREIGN = pmacs.buffer.create('*test-panel*')\n\
         ORIGIN = pmacs.buffer.create('*origin*')\n\
         pmacs.window.switch_buffer(ORIGIN)",
    );
    // Stage 3: BOTH opens are explicitly `display = "current"`, and that
    // is what keeps this test meaningful rather than what makes it pass.
    //
    // Its subject is the BUFFER-level `p.prev` skip rule and the Q#GB18
    // name-keyed identity guard — the `FOREIGN` buffer above shares the
    // panel's name, so the disambiguation to `*test-panel*<2>` is the
    // regression this pins. Under the panel default those two listviews
    // would share the one bottom slot and `q` would exercise the
    // SIDE-WINDOW restore chain instead (Q#BP2c criterion 20), which is
    // a different mechanism with its own test below. Keeping them in
    // document windows isolates the two, so a `p.prev` inversion stays
    // detectable rather than being masked by presentation history.
    open_test_panel_in_document(&mut s);
    assert_eq!(active_name(&s), "*test-panel*<2>", "premise: disambiguated");

    exec(
        &s,
        "pmacs.listview.open { name = '*other-panel*', header = 'O', \
         display = 'current', \
         rows = { { text = 'x', item = 'X' } } }",
    );
    assert_eq!(active_name(&s), "*other-panel*", "premise: second panel");

    press(&mut s, KeyCode::Char('q'));

    assert_ne!(
        active_name(&s),
        "*test-panel*<2>",
        "q must never return into another panel via p.prev --- the \
         raw-switch/capability-fallback loop this rule exists to prevent"
    );
    assert_eq!(
        active_name(&s),
        "*scratch*",
        "with no capturable previous buffer, q falls back to *scratch*"
    );
}

/// Criterion 14 [structural] --- rides **alongside** 1-13, never instead:
/// a structural comparison of two authorities does not catch a misrouted
/// consumer, which is why 11 and 12 assert through `dispatch_key`.
///
/// Three claims, each keyed to a decision: no `bypass_intercept` write
/// survives in either Stage 1 adopter (§1.1's arithmetic is the
/// reference, so the check is per non-comment line rather than a
/// substring sweep that the explanatory comments would trip);
/// `ensure_panel` contains no find-by-name adoption (Q#GB13); and every
/// `panels[` subscript is an append, so none can be keyed by a name
/// derived from `describe.buffer` (Q#GB18).
#[test]
fn s1_14_no_bypass_write_or_name_keyed_identity_remains() {
    const LISTVIEW: &str = include_str!("../builtin/runtime/listview.lua");
    const DIRED: &str = include_str!("../builtin/runtime/dired.lua");

    for (file, src) in [("listview.lua", LISTVIEW), ("dired.lua", DIRED)] {
        let writes: Vec<&str> = src
            .lines()
            .filter(|l| !l.trim_start().starts_with("--") && l.contains("bypass_intercept"))
            .collect();
        assert!(
            writes.is_empty(),
            "{file} must contain no bypass_intercept write; found {writes:?}"
        );
        assert!(
            src.contains("set_generated_contents"),
            "{file} must write through the authorized primitive"
        );
    }

    assert!(
        !LISTVIEW.contains("find_buffer_by_name(name) or pmacs.buffer.create"),
        "ensure_panel must not adopt a same-named foreign buffer"
    );
    let subscripts: Vec<&str> = LISTVIEW
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("--") && line.contains("panels["))
        .filter(|line| !line.starts_with("panels[#panels + 1]"))
        .collect();
    assert!(
        subscripts.is_empty(),
        "every `panels[` subscript must be an append; found {subscripts:?}"
    );
}

// Isolated bootstrap storage roots (see the module docs): an
// integration test is compiled without `cfg(test)`, so a raw
// `EditorState::new()` would read the developer's real `init.lua` and
// write into their real data root.
#[path = "common/iso.rs"]
mod iso;
