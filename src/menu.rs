// menu.rs --- Menu registry: the items that populate the right-click context menu.

//! Context-menu items.
//!
//! The right-click context menu (Q#CM2) is populated from a Lua
//! registry that mirrors [`crate::command::CommandRegistry`]. Each
//! [`MenuItem`] names a [`crate::command::Command`] to invoke and
//! carries optional visibility controls: a coarse `context` tag
//! (sugar) and/or a full `predicate` (the escape hatch), evaluated
//! against a context table when the menu opens (Q#CM3). Items are
//! grouped and ordered for layout; separators fall between groups.
//!
//! # Storage and lookup
//!
//! [`MenuRegistry`] owns the live items in insertion order. Unlike
//! commands, items are not uniquely *named* --- two contexts may each
//! contribute a "Copy". An optional `id` allows targeted removal or
//! in-place replacement, so a user config can hide or override a
//! builtin item (`pmacs.menu.remove("edit.copy")`, or re-`item` with
//! the same `id`) without disturbing the rest of the menu.
//!
//! # Threading
//!
//! Single-threaded, behind `Rc<RefCell<...>>` next to the command and
//! keymap registries.

use mlua::Function;
use thiserror::Error;

use crate::command::SourceLocation;

// ---------------------------------------------------------------------------
// Contexts
// ---------------------------------------------------------------------------

/// Coarse context tags that are sugar for a visibility predicate
/// (Q#CM3). Validated at registration so a typo (`"selecton"`) is a
/// hard error rather than a silently invisible item --- the same
/// typo-paranoia as the command spec's unknown-field check (R50).
///
/// The tag → predicate mapping is owned by the menu builder in the
/// core (it has the live context table); the registry only validates
/// the vocabulary.
pub const KNOWN_CONTEXTS: &[&str] = &["always", "selection", "symbol", "diagnostic"];

// ---------------------------------------------------------------------------
// MenuItem + errors
// ---------------------------------------------------------------------------

/// A single context-menu entry.
///
/// Cloning is cheap: `String`s clone trivially and `mlua::Function` is
/// reference-counted internally.
#[derive(Clone)]
pub struct MenuItem {
    /// Optional stable identifier. Enables targeted [`MenuRegistry::remove`]
    /// and in-place override (re-adding with the same id replaces).
    pub id: Option<String>,
    /// Human-readable label shown in the menu row.
    pub label: String,
    /// Name of the [`crate::command::Command`] this item invokes.
    /// Resolved at invoke time (like a keymap binding), not at
    /// registration --- the command may be defined later.
    pub command: String,
    /// Coarse visibility tag (one of [`KNOWN_CONTEXTS`]). Sugar for a
    /// predicate; ignored when `predicate` is set.
    pub context: Option<String>,
    /// Full visibility predicate: `fn(context_table) -> bool`. The
    /// escape hatch for items whose availability the coarse tags can't
    /// express. Takes precedence over `context`.
    pub predicate: Option<Function>,
    /// Layout group. Items sharing a group render together; a separator
    /// falls between distinct groups (group first-appearance order).
    pub group: String,
    /// Sort key within a group (ascending). Ties break on insertion order.
    pub order: i64,
    /// Where the item was defined (Lua `file:line`).
    pub source: SourceLocation,
}

/// Errors raised by the menu registry.
#[derive(Debug, Error)]
pub enum MenuError {
    /// `item` was called with no label or an all-whitespace one.
    #[error("menu item label must be non-empty")]
    EmptyLabel,

    /// `item` was called without a `command` to invoke.
    #[error("menu item \"{label}\" requires a non-empty `command`")]
    EmptyCommand {
        /// The offending item's label.
        label: String,
    },

    /// The `context` tag is not one of [`KNOWN_CONTEXTS`].
    #[error(
        "menu item \"{label}\" has unknown context `{context}`; supported: always, selection, symbol, diagnostic"
    )]
    UnknownContext {
        /// The offending item's label.
        label: String,
        /// The offending context tag.
        context: String,
    },

    /// The spec table contained a key the registry doesn't know about.
    #[error(
        "unknown field `{field}` in menu item spec; supported: id, label, command, context, predicate, group, order"
    )]
    UnknownField {
        /// The offending key.
        field: String,
    },
}

// ---------------------------------------------------------------------------
// MenuRegistry
// ---------------------------------------------------------------------------

/// Ordered registry of context-menu items.
///
/// Insert via [`Self::add`] (validates, replacing in place on a
/// matching `id`). Read the live set via [`Self::items`]; the menu
/// builder sorts and groups at open time. Remove by id via
/// [`Self::remove`]; reset via [`Self::clear`].
#[derive(Default)]
pub struct MenuRegistry {
    items: Vec<MenuItem>,
}

impl MenuRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert `item`, validating its metadata. If the item carries an
    /// `id` already present, the existing entry is replaced in place
    /// (preserving its slot) so re-running config and user overrides are
    /// idempotent; otherwise the item is appended.
    pub fn add(&mut self, item: MenuItem) -> Result<(), MenuError> {
        if item.label.trim().is_empty() {
            return Err(MenuError::EmptyLabel);
        }
        if item.command.trim().is_empty() {
            return Err(MenuError::EmptyCommand { label: item.label });
        }
        if let Some(context) = &item.context
            && !KNOWN_CONTEXTS.contains(&context.as_str())
        {
            return Err(MenuError::UnknownContext {
                label: item.label,
                context: context.clone(),
            });
        }
        if let Some(id) = item.id.clone()
            && let Some(slot) = self
                .items
                .iter_mut()
                .find(|it| it.id.as_deref() == Some(&id))
        {
            *slot = item;
            return Ok(());
        }
        self.items.push(item);
        Ok(())
    }

    /// The live items, in insertion order.
    #[must_use]
    pub fn items(&self) -> &[MenuItem] {
        &self.items
    }

    /// Remove every item whose `id` equals `id`. Returns `true` if any
    /// item was removed.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|it| it.id.as_deref() != Some(id));
        self.items.len() != before
    }

    /// Drop every item.
    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Number of registered items.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// True iff no items are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Open-menu runtime state (Q#CM1) + TUI surface
// ---------------------------------------------------------------------------

use std::sync::{Arc, Mutex};

use crate::buffer::Buffer;
use crate::cell::{CellCoord, CellGrid, Color, Glyph, Style};
use crate::view::{View, Viewport};

/// One rendered row of an open menu: either a selectable command or a
/// non-selectable group divider. The resolved list is built in Lua
/// (`pmacs.menu.build`, which evaluates predicates / context tags and
/// groups items) and stored here; navigation skips `Separator` rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuRow {
    /// A divider between groups.
    Separator,
    /// A selectable entry that invokes `command` when chosen.
    Item {
        /// Display label.
        label: String,
        /// Name of the command invoked on selection.
        command: String,
    },
}

impl MenuRow {
    const fn is_item(&self) -> bool {
        matches!(self, MenuRow::Item { .. })
    }
}

/// The live state of an open context menu (Q#CM1). Frontend-agnostic:
/// the TUI [`MenuView`] overlay and (later) the GPU producer both render
/// from this. `active` is an index into `rows` that always points at an
/// [`MenuRow::Item`]. `anchor` is the absolute cell the menu opens at
/// (the click point) — the TUI render origin; the GPU positions in
/// pixels locally and ignores it.
pub struct MenuState {
    /// Rows top-to-bottom (items + separators).
    pub rows: Vec<MenuRow>,
    /// Highlighted row index (always an `Item`).
    pub active: usize,
    /// Absolute `(row, col)` cell the popup renders from.
    pub anchor: (u32, u32),
    /// Popup width in cells (max label + padding, clamped).
    pub width: u32,
}

/// Min / max popup width in cells.
const MENU_MIN_WIDTH: u32 = 8;
const MENU_MAX_WIDTH: u32 = 48;

impl MenuState {
    /// Build from a resolved row list. Returns `None` when no row is an
    /// `Item` (an empty menu never opens). Width is the widest label
    /// plus padding, clamped to [`MENU_MIN_WIDTH`]..[`MENU_MAX_WIDTH`].
    #[must_use]
    pub fn new(rows: Vec<MenuRow>, anchor: (u32, u32)) -> Option<Self> {
        let active = rows.iter().position(MenuRow::is_item)?;
        let widest = rows
            .iter()
            .filter_map(|r| match r {
                MenuRow::Item { label, .. } => Some(label.chars().count() as u32),
                MenuRow::Separator => None,
            })
            .max()
            .unwrap_or(0);
        let width = (widest + 2).clamp(MENU_MIN_WIDTH, MENU_MAX_WIDTH);
        Some(Self {
            rows,
            active,
            anchor,
            width,
        })
    }

    /// Move the highlight one `Item` row forward (`delta >= 0`) or back
    /// (`delta < 0`), wrapping and skipping separators. Only the sign of
    /// `delta` matters — callers step one item at a time.
    pub fn step(&mut self, delta: isize) {
        let n = self.rows.len();
        if n == 0 {
            return;
        }
        let forward = delta >= 0;
        for _ in 0..n {
            self.active = if forward {
                (self.active + 1) % n
            } else {
                (self.active + n - 1) % n
            };
            if self.rows[self.active].is_item() {
                return;
            }
        }
    }

    /// The active item's command name.
    #[must_use]
    pub fn active_command(&self) -> Option<&str> {
        match self.rows.get(self.active)? {
            MenuRow::Item { command, .. } => Some(command),
            MenuRow::Separator => None,
        }
    }

    /// Map an absolute cell to the row index it covers, but only when
    /// that row is a selectable `Item` (separators and cells outside the
    /// popup rectangle return `None`).
    #[must_use]
    pub fn hit(&self, row: u32, col: u32) -> Option<usize> {
        let (arow, acol) = self.anchor;
        if row < arow || col < acol {
            return None;
        }
        let ri = (row - arow) as usize;
        let ci = col - acol;
        if ri >= self.rows.len() || ci >= self.width {
            return None;
        }
        self.rows[ri].is_item().then_some(ri)
    }
}

/// Shared handle to the open menu (`None` when closed). Held by
/// [`crate::editor_core::EditorCore`] and read by [`MenuView`], mirroring
/// the search store's `Arc<Mutex>` bridge between core state and the
/// overlay that renders it.
pub type SharedMenu = Arc<Mutex<Option<MenuState>>>;

/// A fresh, closed shared menu.
#[must_use]
pub fn make_shared_menu() -> SharedMenu {
    Arc::new(Mutex::new(None))
}

/// Popup background (non-selected rows) — a dim fill so the menu reads
/// as a floating surface over the buffer text it occludes.
fn menu_style() -> Style {
    Style {
        fg: Color::Indexed(252),
        bg: Color::Indexed(236),
        ..Style::default()
    }
}

/// Highlighted-row style (the active item).
fn menu_selected_style() -> Style {
    Style {
        fg: Color::Indexed(231),
        bg: Color::Indexed(24),
        ..Style::default()
    }
}

/// TUI overlay that paints the open menu (Q#CM1). Persistent on the
/// active window once attached (deduped by [`View::kind`]); renders
/// nothing while the menu is closed, mirroring `SearchView`'s
/// self-suppressing model. Owns every cell inside the popup rectangle,
/// occluding the buffer text beneath.
pub struct MenuView {
    menu: SharedMenu,
}

impl MenuView {
    /// Build a view reading `menu`.
    #[must_use]
    pub fn new(menu: SharedMenu) -> Self {
        Self { menu }
    }
}

impl View for MenuView {
    fn kind(&self) -> &'static str {
        "context-menu"
    }

    fn render(&mut self, _buf: &Buffer, viewport: Viewport<'_>, cells: &mut CellGrid<'_>) {
        let guard = self.menu.lock().expect("menu mutex poisoned");
        let Some(menu) = guard.as_ref() else {
            return;
        };
        let top = viewport.cell_origin.row;
        let left = viewport.cell_origin.col;
        let bottom = top + viewport.cell_size.rows;
        let right = left + viewport.cell_size.cols;
        let (arow, acol) = menu.anchor;

        for (i, row) in menu.rows.iter().enumerate() {
            let r = arow + i as u32;
            if r < top || r >= bottom {
                continue; // clip rows that fall outside the window
            }
            let selected = i == menu.active;
            let row_style = if selected {
                menu_selected_style()
            } else {
                menu_style()
            };
            // Paint the full-width row background first.
            for c in 0..menu.width {
                let col = acol + c;
                if col < left || col >= right {
                    continue;
                }
                let cell = cells.at(CellCoord::new(r, col));
                cell.glyph = Glyph::Char(' ');
                cell.style = row_style;
                cell.attachment = None;
            }
            match row {
                MenuRow::Separator => {
                    for c in 0..menu.width {
                        let col = acol + c;
                        if col < left || col >= right {
                            continue;
                        }
                        cells.at(CellCoord::new(r, col)).glyph = Glyph::Char('─');
                    }
                }
                MenuRow::Item { label, .. } => {
                    // One cell of left padding; stop at the popup's right edge.
                    let row_right = (acol + menu.width).min(right);
                    for (col, ch) in (acol + 1..).zip(label.chars()) {
                        if col >= row_right {
                            break;
                        }
                        let cell = cells.at(CellCoord::new(r, col));
                        cell.glyph = Glyph::Char(ch);
                        cell.style = row_style;
                        cell.attachment = None;
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;

    fn item(id: Option<&str>, label: &str, command: &str) -> MenuItem {
        MenuItem {
            id: id.map(ToOwned::to_owned),
            label: label.to_owned(),
            command: command.to_owned(),
            context: None,
            predicate: None,
            group: String::new(),
            order: 0,
            source: SourceLocation::default(),
        }
    }

    #[test]
    fn add_then_items_round_trips() {
        let mut r = MenuRegistry::new();
        r.add(item(None, "Copy", "edit.copy")).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r.items()[0].label, "Copy");
        assert_eq!(r.items()[0].command, "edit.copy");
    }

    #[test]
    fn empty_label_is_rejected() {
        let mut r = MenuRegistry::new();
        assert!(matches!(
            r.add(item(None, "   ", "edit.copy")),
            Err(MenuError::EmptyLabel)
        ));
    }

    #[test]
    fn empty_command_is_rejected() {
        let mut r = MenuRegistry::new();
        match r.add(item(None, "Copy", "")) {
            Err(MenuError::EmptyCommand { label }) => assert_eq!(label, "Copy"),
            other => panic!("expected EmptyCommand, got {other:?}"),
        }
    }

    #[test]
    fn unknown_context_is_rejected() {
        let mut r = MenuRegistry::new();
        let mut it = item(None, "Copy", "edit.copy");
        it.context = Some("selecton".into());
        match r.add(it) {
            Err(MenuError::UnknownContext { label, context }) => {
                assert_eq!(label, "Copy");
                assert_eq!(context, "selecton");
            }
            other => panic!("expected UnknownContext, got {other:?}"),
        }
    }

    #[test]
    fn known_contexts_are_accepted() {
        let mut r = MenuRegistry::new();
        for cx in KNOWN_CONTEXTS {
            let mut it = item(None, "X", "x.cmd");
            it.context = Some((*cx).to_owned());
            r.add(it).unwrap();
        }
        assert_eq!(r.len(), KNOWN_CONTEXTS.len());
    }

    #[test]
    fn predicate_is_stored() {
        let lua = Lua::new();
        let mut r = MenuRegistry::new();
        let mut it = item(None, "Paste", "edit.paste");
        it.predicate = Some(lua.create_function(|_, ()| Ok(true)).unwrap());
        r.add(it).unwrap();
        assert!(r.items()[0].predicate.is_some());
    }

    #[test]
    fn matching_id_replaces_in_place() {
        let mut r = MenuRegistry::new();
        r.add(item(Some("a"), "First", "cmd.a")).unwrap();
        r.add(item(Some("b"), "Second", "cmd.b")).unwrap();
        // Override `a` in place: stays at slot 0 with the new label.
        r.add(item(Some("a"), "First!", "cmd.a")).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r.items()[0].label, "First!");
        assert_eq!(r.items()[1].label, "Second");
    }

    #[test]
    fn remove_by_id_drops_the_item() {
        let mut r = MenuRegistry::new();
        r.add(item(Some("a"), "A", "cmd.a")).unwrap();
        r.add(item(None, "B", "cmd.b")).unwrap();
        assert!(r.remove("a"));
        assert!(!r.remove("a")); // already gone
        assert_eq!(r.len(), 1);
        assert_eq!(r.items()[0].label, "B");
    }

    #[test]
    fn clear_empties_the_registry() {
        let mut r = MenuRegistry::new();
        r.add(item(None, "A", "cmd.a")).unwrap();
        r.add(item(None, "B", "cmd.b")).unwrap();
        r.clear();
        assert!(r.is_empty());
    }

    #[test]
    fn items_without_id_both_append() {
        let mut r = MenuRegistry::new();
        r.add(item(None, "A", "cmd.a")).unwrap();
        r.add(item(None, "A", "cmd.a")).unwrap();
        // No id → no dedup; both are kept.
        assert_eq!(r.len(), 2);
    }

    // ---- MenuState (open-menu runtime) -------------------------------------

    fn row_item(label: &str) -> MenuRow {
        MenuRow::Item {
            label: label.to_owned(),
            command: format!("cmd.{label}"),
        }
    }

    #[test]
    fn menu_state_new_requires_a_selectable_item() {
        assert!(MenuState::new(vec![], (0, 0)).is_none());
        assert!(MenuState::new(vec![MenuRow::Separator], (0, 0)).is_none());
        // active lands on the first item, skipping a leading separator.
        let m = MenuState::new(vec![MenuRow::Separator, row_item("A")], (2, 3)).unwrap();
        assert_eq!(m.active, 1);
        assert_eq!(m.anchor, (2, 3));
    }

    #[test]
    fn menu_state_step_skips_separators_and_wraps() {
        let rows = vec![row_item("A"), MenuRow::Separator, row_item("B")];
        let mut m = MenuState::new(rows, (0, 0)).unwrap();
        assert_eq!(m.active, 0);
        m.step(1);
        assert_eq!(m.active, 2); // jumps over the separator at row 1
        m.step(1);
        assert_eq!(m.active, 0); // wraps to the top
        m.step(-1);
        assert_eq!(m.active, 2); // wraps back, still skipping the separator
    }

    #[test]
    fn menu_state_hit_maps_cells_to_item_rows_only() {
        let rows = vec![row_item("A"), MenuRow::Separator, row_item("B")];
        let m = MenuState::new(rows, (5, 10)).unwrap();
        // Row 5 = item A (anywhere within the popup width).
        assert_eq!(m.hit(5, 10), Some(0));
        assert_eq!(m.hit(5, 10 + m.width - 1), Some(0));
        // Row 6 = separator → not selectable.
        assert_eq!(m.hit(6, 10), None);
        // Row 7 = item B.
        assert_eq!(m.hit(7, 12), Some(2));
        // Outside the popup rectangle in each direction.
        assert_eq!(m.hit(4, 10), None); // above
        assert_eq!(m.hit(8, 10), None); // below
        assert_eq!(m.hit(5, 9), None); // left
        assert_eq!(m.hit(5, 10 + m.width), None); // right
    }

    #[test]
    fn menu_state_active_command_reads_the_highlight() {
        let mut m = MenuState::new(vec![row_item("A"), row_item("B")], (0, 0)).unwrap();
        assert_eq!(m.active_command(), Some("cmd.A"));
        m.step(1);
        assert_eq!(m.active_command(), Some("cmd.B"));
    }
}
