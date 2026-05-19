// locations.rs --- T M4.5: Location-shaped navigation requests.

//! Shared store for the navigation requests whose response shape is
//! identical to `textDocument/definition`
//! (`Location | Location[] | LocationLink[] | null`): `references`,
//! `declaration`, `typeDefinition`, `implementation`. Parsing is
//! reused verbatim from [`crate::definition::DefinitionResponse`];
//! this module only adds a `kind` discriminator so the four request
//! types don't collide on `(server, uri)` the way they would in the
//! existing single-purpose definition store.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::definition::DefinitionResponse;

/// Which Location-shaped request a stored response answers.
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum LocationKind {
    /// `textDocument/references`.
    References,
    /// `textDocument/declaration`.
    Declaration,
    /// `textDocument/typeDefinition`.
    TypeDefinition,
    /// `textDocument/implementation`.
    Implementation,
}

impl LocationKind {
    /// Stable lowercase label — the `pmacs.lsp.*` Lua surface name,
    /// and the supersede-/store-key discriminator.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            LocationKind::References => "references",
            LocationKind::Declaration => "declaration",
            LocationKind::TypeDefinition => "type_definition",
            LocationKind::Implementation => "implementation",
        }
    }

    /// The LSP request method name.
    #[must_use]
    pub fn method(self) -> &'static str {
        match self {
            LocationKind::References => "textDocument/references",
            LocationKind::Declaration => "textDocument/declaration",
            LocationKind::TypeDefinition => "textDocument/typeDefinition",
            LocationKind::Implementation => "textDocument/implementation",
        }
    }
}

/// Key into [`LocationsStore`].
#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct LocationsKey {
    /// Decimal LSP server id.
    pub server: String,
    /// Document URI.
    pub uri: String,
    /// Which request this entry answers.
    pub kind: LocationKind,
}

impl LocationsKey {
    /// Construct a key.
    #[must_use]
    pub fn new(server: impl Into<String>, uri: impl Into<String>, kind: LocationKind) -> Self {
        Self {
            server: server.into(),
            uri: uri.into(),
            kind,
        }
    }
}

/// Per-`(server, uri, kind)` Location-list state. The value type is
/// [`DefinitionResponse`] — same shape, same parser.
#[derive(Default)]
pub struct LocationsStore {
    by_key: HashMap<LocationsKey, DefinitionResponse>,
}

impl LocationsStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the response at `key`.
    pub fn set(&mut self, key: LocationsKey, response: DefinitionResponse) {
        self.by_key.insert(key, response);
    }

    /// Drop the entry at `key`.
    pub fn clear(&mut self, key: &LocationsKey) {
        self.by_key.remove(key);
    }

    /// Look up the entry at `key`.
    #[must_use]
    pub fn get(&self, key: &LocationsKey) -> Option<&DefinitionResponse> {
        self.by_key.get(key)
    }
}

/// Cheaply-cloneable shared handle.
pub type SharedLocationsStore = Arc<Mutex<LocationsStore>>;

/// Build a fresh shared store.
#[must_use]
pub fn make_shared_store() -> SharedLocationsStore {
    Arc::new(Mutex::new(LocationsStore::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definition::DefinitionLocation;

    #[test]
    fn kind_labels_and_methods_are_distinct() {
        let kinds = [
            LocationKind::References,
            LocationKind::Declaration,
            LocationKind::TypeDefinition,
            LocationKind::Implementation,
        ];
        let mut labels: Vec<&str> = kinds.iter().map(|k| k.label()).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), 4, "labels must be unique");
        for k in kinds {
            assert!(k.method().starts_with("textDocument/"));
        }
    }

    #[test]
    fn kind_keys_do_not_collide() {
        let mut s = LocationsStore::new();
        let mk = |kind| LocationsKey::new("1", "file:///a", kind);
        let one = |line| DefinitionResponse {
            locations: vec![DefinitionLocation {
                uri: "file:///t".into(),
                line,
                col: 0,
            }],
        };
        s.set(mk(LocationKind::References), one(1));
        s.set(mk(LocationKind::Implementation), one(2));
        // Same (server, uri) but different kind ⇒ independent entries.
        assert_eq!(
            s.get(&mk(LocationKind::References)).unwrap().locations[0].line,
            1
        );
        assert_eq!(
            s.get(&mk(LocationKind::Implementation)).unwrap().locations[0].line,
            2
        );
        s.clear(&mk(LocationKind::References));
        assert!(s.get(&mk(LocationKind::References)).is_none());
        assert!(s.get(&mk(LocationKind::Implementation)).is_some());
    }
}
