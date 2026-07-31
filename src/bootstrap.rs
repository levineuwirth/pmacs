// src/bootstrap.rs --- explicit bootstrap storage roots.

//! Where pmacs stores things at startup, as a value rather than as an
//! ambient property of the process environment.
//!
//! # Why this exists
//!
//! `EditorState::new` resolves two storage roots from the environment
//! before it returns:
//!
//! * the **data** root, which
//!   [`crate::builtin_packages::bundled_runtime_dir`] resolves from
//!   `XDG_DATA_HOME` (else `$HOME/.local/share`) and which
//!   [`crate::builtin_packages::materialize_all`] then **writes into**
//!   --- unconditionally, outside any `cfg` guard; and
//! * the **config** root, which [`crate::config::user_config_dir`]
//!   resolves from `XDG_CONFIG_HOME` (else `$HOME/.config`) and from
//!   which `init.lua` is read.
//!
//! An integration test in `tests/` links this crate as an ordinary
//! dependency, so it is compiled **without** `cfg(test)`: the
//! `#[cfg(not(test))]` guard around config loading is inactive for
//! every one of them. They read the developer's real `init.lua` and
//! write into the developer's real data root.
//!
//! They cannot fix that themselves. `std::env::set_var` has been
//! `unsafe` since Rust 2024 and this crate is `#![forbid(unsafe_code)]`
//! --- the same constraint that produced
//! [`crate::packages::installer::Installer::with_install_root_override`]
//! and [`crate::lua_bindings::PackageInstallOverride`]. So isolation has
//! to arrive as a **parameter**, which is what this type is.
//!
//! # Contract
//!
//! [`BootstrapRoots::ambient()`] is production: every root stays `None`
//! and every resolution goes to the environment exactly as before. A
//! root that is `Some` replaces the environment lookup for that root and
//! **only** that root.
//!
//! # Scope: storage roots only
//!
//! This type covers the four roots that decide where pmacs *stores*
//! things: config, data, state and cache. It deliberately does not
//! cover:
//!
//! * **`HOME`'s non-storage semantics.** `expand_tilde`
//!   ([`crate::editor_core`]) resolves a leading `~` for ordinary path
//!   entry, and `tests/find_file_acceptance.rs` consumes `HOME` on
//!   purpose to pin that expansion. Redirecting a storage root is the
//!   right fix for a storage root and the wrong fix for a
//!   path-expansion root.
//! * **`XDG_RUNTIME_DIR`**, which addresses sockets rather than stored
//!   data.

use std::path::{Path, PathBuf};

/// The bootstrap storage roots an [`crate::editor::EditorState`] is
/// constructed against.
///
/// Each field is the *base* directory --- the value `XDG_<X>_HOME`
/// would hold --- not the `pmacs/` subdirectory under it. `None` means
/// "resolve from the environment", which is what production does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BootstrapRoots {
    config: Option<PathBuf>,
    data: Option<PathBuf>,
    state: Option<PathBuf>,
    cache: Option<PathBuf>,
}

/// The version-keyed leaf `bundled_runtime_dir` materializes into.
fn bundled_leaf() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

impl BootstrapRoots {
    /// Production: every root resolves from the process environment.
    #[must_use]
    pub fn ambient() -> Self {
        Self::default()
    }

    /// Every storage root redirected under `base`, into sibling
    /// `config/`, `data/`, `state/` and `cache/` directories.
    ///
    /// The layout mirrors the XDG one so a caller that also spawns a
    /// child process can hand the same four paths to `XDG_CONFIG_HOME`,
    /// `XDG_DATA_HOME`, `XDG_STATE_HOME` and `XDG_CACHE_HOME` (plus
    /// `PMACS_STATE_HOME`, which outranks `XDG_STATE_HOME`) and get the
    /// same tree from an in-process and a spawned editor.
    #[must_use]
    pub fn isolated_under(base: &Path) -> Self {
        Self {
            config: Some(base.join("config")),
            data: Some(base.join("data")),
            state: Some(base.join("state")),
            cache: Some(base.join("cache")),
        }
    }

    /// Builder: redirect the config root (the `XDG_CONFIG_HOME` value).
    #[must_use]
    pub fn with_config_root(mut self, root: PathBuf) -> Self {
        self.config = Some(root);
        self
    }

    /// Builder: redirect the data root (the `XDG_DATA_HOME` value).
    #[must_use]
    pub fn with_data_root(mut self, root: PathBuf) -> Self {
        self.data = Some(root);
        self
    }

    /// Builder: redirect the state root (the `XDG_STATE_HOME` value).
    #[must_use]
    pub fn with_state_root(mut self, root: PathBuf) -> Self {
        self.state = Some(root);
        self
    }

    /// Builder: redirect the cache root (the `XDG_CACHE_HOME` value).
    #[must_use]
    pub fn with_cache_root(mut self, root: PathBuf) -> Self {
        self.cache = Some(root);
        self
    }

    /// True when nothing is redirected --- i.e. this is production's
    /// [`Self::ambient`].
    #[must_use]
    pub fn is_ambient(&self) -> bool {
        self.config.is_none() && self.data.is_none() && self.state.is_none() && self.cache.is_none()
    }

    /// The config *base*, if redirected.
    #[must_use]
    pub fn config_root(&self) -> Option<&Path> {
        self.config.as_deref()
    }

    /// The data *base*, if redirected.
    #[must_use]
    pub fn data_root(&self) -> Option<&Path> {
        self.data.as_deref()
    }

    /// The state *base*, if redirected.
    #[must_use]
    pub fn state_root(&self) -> Option<&Path> {
        self.state.as_deref()
    }

    /// The cache *base*, if redirected.
    #[must_use]
    pub fn cache_root(&self) -> Option<&Path> {
        self.cache.as_deref()
    }

    /// The directory `init.lua` is read from --- `<config>/pmacs`, the
    /// same shape [`crate::config::user_config_dir`] builds.
    #[must_use]
    pub fn config_dir(&self) -> Option<PathBuf> {
        self.config.as_ref().map(|p| p.join("pmacs"))
    }

    /// Where bundled packages are materialized ---
    /// `<data>/pmacs/builtin-packages/v<crate-version>`, the same shape
    /// [`crate::builtin_packages::bundled_runtime_dir`] builds.
    #[must_use]
    pub fn bundled_runtime_dir(&self) -> Option<PathBuf> {
        self.data.as_ref().map(|p| {
            p.join("pmacs")
                .join("builtin-packages")
                .join(bundled_leaf())
        })
    }

    /// The user-scope package install root --- `<data>/pmacs/packages`.
    #[must_use]
    pub fn package_install_root(&self) -> Option<PathBuf> {
        self.data.as_ref().map(|p| p.join("pmacs").join("packages"))
    }

    /// The package fetcher's bare-mirror cache --- `<cache>/pmacs/git`.
    #[must_use]
    pub fn package_cache_dir(&self) -> Option<PathBuf> {
        self.cache.as_ref().map(|p| p.join("pmacs").join("git"))
    }

    /// The editor state directory --- `<state>/pmacs`, the same shape
    /// [`crate::state::user_state_dir`] builds.
    #[must_use]
    pub fn state_dir(&self) -> Option<PathBuf> {
        self.state.as_ref().map(|p| p.join("pmacs"))
    }

    /// The minibuffer history directory --- `<state>/pmacs/history`.
    #[must_use]
    pub fn history_dir(&self) -> Option<PathBuf> {
        self.state_dir().map(|d| d.join("history"))
    }

    /// The environment a **child** `pmacs` process must be given so it
    /// resolves the same roots this value names.
    ///
    /// An in-process caller passes the value; a caller that spawns
    /// `pmacs --daemon` (or re-execs a test binary) cannot, and has to
    /// go through the environment instead. This is that translation, so
    /// the two paths cannot drift.
    ///
    /// **Five variables, not four.** `PMACS_STATE_HOME` outranks
    /// `XDG_STATE_HOME` ([`crate::state::user_state_dir`]), so a child
    /// given only the four XDG variables still resolves the *inherited*
    /// `PMACS_STATE_HOME` if the launching environment exports one --- a
    /// hole that is invisible on a machine that does not.
    ///
    /// A root left ambient emits no variable, so the child inherits it.
    #[must_use]
    pub fn child_env(&self) -> Vec<(&'static str, PathBuf)> {
        let mut out = Vec::with_capacity(5);
        if let Some(p) = &self.config {
            out.push(("XDG_CONFIG_HOME", p.clone()));
        }
        if let Some(p) = &self.data {
            out.push(("XDG_DATA_HOME", p.clone()));
        }
        if let Some(p) = &self.state {
            out.push(("XDG_STATE_HOME", p.clone()));
            out.push(("PMACS_STATE_HOME", p.clone()));
        }
        if let Some(p) = &self.cache {
            out.push(("XDG_CACHE_HOME", p.clone()));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ambient_redirects_nothing() {
        let roots = BootstrapRoots::ambient();
        assert!(roots.is_ambient());
        assert_eq!(roots.config_dir(), None);
        assert_eq!(roots.bundled_runtime_dir(), None);
        assert_eq!(roots.state_dir(), None);
        assert_eq!(roots.package_cache_dir(), None);
    }

    #[test]
    fn isolated_under_mirrors_the_xdg_layout() {
        let roots = BootstrapRoots::isolated_under(Path::new("/scratch"));
        assert!(!roots.is_ambient());
        assert_eq!(
            roots.config_dir().unwrap(),
            Path::new("/scratch/config/pmacs")
        );
        assert_eq!(
            roots.bundled_runtime_dir().unwrap(),
            Path::new("/scratch/data/pmacs/builtin-packages").join(bundled_leaf())
        );
        assert_eq!(
            roots.package_install_root().unwrap(),
            Path::new("/scratch/data/pmacs/packages")
        );
        assert_eq!(
            roots.package_cache_dir().unwrap(),
            Path::new("/scratch/cache/pmacs/git")
        );
        assert_eq!(
            roots.state_dir().unwrap(),
            Path::new("/scratch/state/pmacs")
        );
        assert_eq!(
            roots.history_dir().unwrap(),
            Path::new("/scratch/state/pmacs/history")
        );
    }

    /// A builder that sets one root leaves the other three ambient ---
    /// the "only that root" half of the contract.
    #[test]
    fn a_single_builder_leaves_the_other_roots_ambient() {
        let roots = BootstrapRoots::ambient().with_config_root(PathBuf::from("/only/config"));
        assert!(!roots.is_ambient());
        assert_eq!(roots.config_dir().unwrap(), Path::new("/only/config/pmacs"));
        assert_eq!(roots.bundled_runtime_dir(), None);
        assert_eq!(roots.state_dir(), None);
        assert_eq!(roots.package_cache_dir(), None);
    }

    /// The five-variable contract, asserted as content: naming only the
    /// four XDG variables leaves `PMACS_STATE_HOME` --- which outranks
    /// `XDG_STATE_HOME` --- pointing wherever the launching environment
    /// left it.
    #[test]
    fn child_env_names_all_five_storage_variables() {
        let roots = BootstrapRoots::isolated_under(Path::new("/scratch"));
        let env = roots.child_env();
        let names: Vec<&str> = env.iter().map(|(k, _)| *k).collect();
        assert_eq!(
            names,
            vec![
                "XDG_CONFIG_HOME",
                "XDG_DATA_HOME",
                "XDG_STATE_HOME",
                "PMACS_STATE_HOME",
                "XDG_CACHE_HOME",
            ]
        );
        let value = |name: &str| {
            env.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.clone())
                .unwrap()
        };
        assert_eq!(value("XDG_CONFIG_HOME"), Path::new("/scratch/config"));
        assert_eq!(value("XDG_DATA_HOME"), Path::new("/scratch/data"));
        assert_eq!(value("XDG_STATE_HOME"), Path::new("/scratch/state"));
        assert_eq!(value("PMACS_STATE_HOME"), Path::new("/scratch/state"));
        assert_eq!(value("XDG_CACHE_HOME"), Path::new("/scratch/cache"));
    }

    /// An ambient root emits no variable — the child inherits it. A
    /// blanket five-variable emission would silently redirect roots the
    /// caller deliberately left alone.
    #[test]
    fn child_env_emits_nothing_for_ambient_roots() {
        assert!(BootstrapRoots::ambient().child_env().is_empty());
        let only_config = BootstrapRoots::ambient().with_config_root(PathBuf::from("/only/config"));
        assert_eq!(
            only_config.child_env(),
            vec![("XDG_CONFIG_HOME", PathBuf::from("/only/config"))]
        );
    }

    /// `child_env` and the in-process resolvers must describe the same
    /// tree: the spawned daemon and the in-process editor are both
    /// supposed to land in the isolated roots, and a mismatch would let
    /// one of them escape while the other looked fine.
    #[test]
    fn child_env_agrees_with_the_in_process_resolvers() {
        let roots = BootstrapRoots::isolated_under(Path::new("/scratch"));
        let env = roots.child_env();
        let value = |name: &str| {
            env.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.clone())
                .unwrap()
        };
        assert_eq!(
            roots.config_dir().unwrap(),
            value("XDG_CONFIG_HOME").join("pmacs")
        );
        assert_eq!(
            roots.package_install_root().unwrap(),
            value("XDG_DATA_HOME").join("pmacs").join("packages")
        );
        assert_eq!(
            roots.state_dir().unwrap(),
            value("PMACS_STATE_HOME").join("pmacs")
        );
        assert_eq!(
            roots.package_cache_dir().unwrap(),
            value("XDG_CACHE_HOME").join("pmacs").join("git")
        );
    }

    /// The isolated bundled dir must agree with the ambient resolver's
    /// shape, version leaf included: a mismatch would make an isolated
    /// editor materialize somewhere production never looks.
    #[test]
    fn bundled_leaf_matches_the_ambient_resolver() {
        let roots = BootstrapRoots::isolated_under(Path::new("/scratch"));
        let isolated = roots.bundled_runtime_dir().unwrap();
        // `bundled_runtime_dir()` reads the live environment, so compare
        // only the tail that does not depend on it.
        let ambient = crate::builtin_packages::bundled_runtime_dir();
        let tail = |p: &Path| {
            p.components()
                .rev()
                .take(3)
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(tail(&isolated), tail(&ambient));
    }
}
