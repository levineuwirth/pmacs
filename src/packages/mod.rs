// packages/mod.rs --- Package model: manifests, addresses, resolver, lockfile.

//! Package model (spec §sec:packages-future, M7).
//!
//! v0.1 ships zero package machinery; loose Lua files in a config
//! directory are loaded directly. M7 adds the manifest format, address
//! parsing, the dependency resolver, the lockfile, and the loader that
//! wires all of it into `require`.
//!
//! This module assembles the M7 building blocks. T M7.1 lands the
//! manifest schema and parser ([`manifest`]); subsequent M7 tasks add
//! their own submodules under this same parent.

pub mod address;
pub mod fetcher;
pub mod installer;
pub mod manifest;

pub use address::{Address, AddressError};
pub use fetcher::{FetchError, Fetcher, RefSpec};
pub use installer::{InstallError, InstallScope, InstallSpec, InstalledPackage, Installer};
pub use manifest::{DependencySpec, ManifestError, PackageManifest, PackageName};
