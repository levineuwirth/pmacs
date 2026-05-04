// packages/manifest.rs --- pmacs.toml schema, parser, and validator.

//! Manifest schema and parser (T M7.1, spec §sec:packages-future).
//!
//! A package is a versioned, addressable unit of Lua code with declared
//! metadata, dependencies, and a single entry point. Every package
//! ships a [`pmacs.toml`] at its root, deserialized into a
//! [`PackageManifest`] at install time (load time only re-stats it).
//!
//! v1.0 fields:
//! - `name` ([`PackageName`]) --- lowercase, hyphen-separated, with
//!   an optional `user/pkg-name` namespace.
//! - `version` ([`semver::Version`]) --- semantic version, parsed and
//!   rejected at parse time, not at install time.
//! - `summary` ([`String`]) --- one-line description.
//! - `pmacs_required` ([`semver::VersionReq`]) --- the pmacs version
//!   range this package supports.
//! - `dependencies` (`Vec<`[`DependencySpec`]`>`) --- packages this
//!   one needs, by address plus version constraint.
//! - `conflicts` (`Vec<`[`DependencySpec`]`>`) --- packages this one
//!   refuses to coexist with (e.g., two REPL implementations).
//! - `entry` ([`PathBuf`]) --- the Lua module file `require` returns.
//! - `exports` (`Vec<String>`) --- public Lua module names; other
//!   code must use only these.
//!
//! `dependencies` and `conflicts` default to empty if omitted; every
//! other field is required. Unknown fields are accepted (forward
//! compatibility): a v1.0 binary reading a v1.1 manifest with a new
//! optional field still loads it.
//!
//! ## TOML shape
//!
//! ```toml
//! name = "pmacs-magit"
//! version = "1.2.3"
//! summary = "Git porcelain for pmacs."
//! pmacs_required = ">= 1.0.0, < 2.0.0"
//! entry = "init.lua"
//! exports = ["magit", "magit.commit"]
//!
//! [[dependencies]]
//! address = "github:user/pmacs-async-utils"
//! version = "^0.4.0"
//!
//! [[conflicts]]
//! address = "github:other/pmacs-vc"
//! version = "*"
//! ```

use std::path::PathBuf;

use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// PackageName
// ---------------------------------------------------------------------------

/// Validated package name. Lowercase letters, digits, and hyphens; an
/// optional `namespace/name` form for forge-style ownership prefixes.
///
/// Construct via [`PackageName::new`]; deserialization runs the same
/// validator and surfaces a parse-time error on invalid names.
#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize)]
pub struct PackageName(String);

impl PackageName {
    /// Validate and wrap a candidate name.
    ///
    /// Rules: each segment (the part before `/` and the part after)
    /// must start with a lowercase ASCII letter and contain only
    /// `[a-z0-9-]`. At most one `/` separates a namespace from a name.
    pub fn new(s: impl Into<String>) -> Result<Self, ManifestError> {
        let s = s.into();
        let (head, tail) = match s.split_once('/') {
            None => (s.as_str(), ""),
            Some((h, t)) => (h, t),
        };
        if s.matches('/').count() > 1 {
            return Err(ManifestError::InvalidName {
                value: s.clone(),
                reason: "at most one `/` separator".into(),
            });
        }
        Self::validate_segment(head, &s)?;
        if !tail.is_empty() {
            Self::validate_segment(tail, &s)?;
        } else if s.contains('/') {
            return Err(ManifestError::InvalidName {
                value: s.clone(),
                reason: "namespace separator with empty tail".into(),
            });
        }
        Ok(Self(s))
    }

    fn validate_segment(seg: &str, full: &str) -> Result<(), ManifestError> {
        if seg.is_empty() {
            return Err(ManifestError::InvalidName {
                value: full.to_string(),
                reason: "empty segment".into(),
            });
        }
        let bytes = seg.as_bytes();
        if !bytes[0].is_ascii_lowercase() {
            return Err(ManifestError::InvalidName {
                value: full.to_string(),
                reason: format!("segment `{seg}` must start with a-z"),
            });
        }
        for &b in bytes {
            if !(b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
                return Err(ManifestError::InvalidName {
                    value: full.to_string(),
                    reason: format!("segment `{seg}` may only contain a-z, 0-9, and `-`"),
                });
            }
        }
        Ok(())
    }

    /// Borrow the inner string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for PackageName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::new(s).map_err(serde::de::Error::custom)
    }
}

// ---------------------------------------------------------------------------
// DependencySpec
// ---------------------------------------------------------------------------

/// A package address plus the version constraint that gates resolution.
///
/// The `address` is whatever syntax the package author wrote in their
/// manifest (`github:user/repo`, `git:URL`, etc.); validation that the
/// address parses lives in T M7.2's address fetcher, not here. T M7.1
/// stores the raw string verbatim so the M7.2 parser can deliver
/// scheme-localized errors at install time.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct DependencySpec {
    /// Package address (e.g., `"github:user/pmacs-magit"`).
    pub address: String,
    /// Version constraint (e.g., `"^1.0.0"`).
    pub version: VersionReq,
}

// ---------------------------------------------------------------------------
// PackageManifest
// ---------------------------------------------------------------------------

/// In-memory representation of a parsed `pmacs.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageManifest {
    /// Unique identifier per [`PackageName`] rules.
    pub name: PackageName,
    /// Package version (semver).
    pub version: Version,
    /// One-line description.
    pub summary: String,
    /// Compatible pmacs version range.
    pub pmacs_required: VersionReq,
    /// Dependencies. Empty if omitted from TOML.
    #[serde(default)]
    pub dependencies: Vec<DependencySpec>,
    /// Conflicts. Empty if omitted from TOML.
    #[serde(default)]
    pub conflicts: Vec<DependencySpec>,
    /// Lua module path the package's `require` returns. Relative to
    /// the package root.
    pub entry: PathBuf,
    /// Public Lua module names exported to other packages.
    pub exports: Vec<String>,
}

impl PackageManifest {
    /// Parse a TOML manifest from a string.
    ///
    /// Validation happens during deserialization: missing required
    /// fields produce errors naming the field; invalid semver in
    /// `version` or `pmacs_required` is rejected at parse time.
    pub fn from_toml(s: &str) -> Result<Self, ManifestError> {
        toml::from_str(s).map_err(ManifestError::from)
    }

    /// Serialize to canonical TOML form.
    pub fn to_toml(&self) -> Result<String, ManifestError> {
        toml::to_string(self).map_err(ManifestError::from)
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by manifest parsing, validation, and serialization.
#[derive(Debug, Error)]
pub enum ManifestError {
    /// A package name failed [`PackageName::new`]'s rules.
    #[error("invalid package name `{value}`: {reason}")]
    InvalidName {
        /// The offending string.
        value: String,
        /// Human-readable explanation (which segment, which character).
        reason: String,
    },
    /// TOML deserialization or per-field validation failed. The inner
    /// error includes a span (line, column) plus the field name from
    /// serde for missing-field errors and the semver crate's own
    /// message for invalid `version` / `pmacs_required`.
    #[error("manifest parse error: {0}")]
    Parse(#[from] toml::de::Error),
    /// TOML serialization failed (e.g., a non-UTF-8 path).
    #[error("manifest serialize error: {0}")]
    Serialize(#[from] toml::ser::Error),
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn sample_manifest() -> PackageManifest {
        PackageManifest {
            name: PackageName::new("pmacs-magit").unwrap(),
            version: Version::new(1, 2, 3),
            summary: "Git porcelain for pmacs.".into(),
            pmacs_required: VersionReq::parse(">=1.0.0, <2.0.0").unwrap(),
            dependencies: vec![DependencySpec {
                address: "github:user/pmacs-async-utils".into(),
                version: VersionReq::parse("^0.4.0").unwrap(),
            }],
            conflicts: vec![DependencySpec {
                address: "github:other/pmacs-vc".into(),
                version: VersionReq::parse("*").unwrap(),
            }],
            entry: PathBuf::from("init.lua"),
            exports: vec!["magit".into(), "magit.commit".into()],
        }
    }

    // -- Round-trip ----------------------------------------------------------

    #[test]
    fn from_toml_round_trips_a_valid_manifest() {
        let m = sample_manifest();
        let s = m.to_toml().unwrap();
        let parsed = PackageManifest::from_toml(&s).unwrap();
        assert_eq!(parsed, m);
    }

    #[test]
    fn from_toml_accepts_minimal_manifest_with_defaults() {
        let s = r#"
            name = "minimal"
            version = "0.1.0"
            summary = "minimal package"
            pmacs_required = ">=1.0.0"
            entry = "init.lua"
            exports = []
        "#;
        let m = PackageManifest::from_toml(s).unwrap();
        assert_eq!(m.name.as_str(), "minimal");
        assert!(m.dependencies.is_empty());
        assert!(m.conflicts.is_empty());
    }

    // -- Missing required fields name the field -----------------------------

    #[test]
    fn missing_name_field_error_names_name() {
        let s = r#"
            version = "0.1.0"
            summary = "x"
            pmacs_required = ">=1.0.0"
            entry = "init.lua"
            exports = []
        "#;
        let err = PackageManifest::from_toml(s).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("name"),
            "error should name the missing field, got: {msg}"
        );
    }

    #[test]
    fn missing_version_field_error_names_version() {
        let s = r#"
            name = "ok"
            summary = "x"
            pmacs_required = ">=1.0.0"
            entry = "init.lua"
            exports = []
        "#;
        let err = PackageManifest::from_toml(s).unwrap_err();
        assert!(err.to_string().contains("version"));
    }

    #[test]
    fn missing_summary_field_error_names_summary() {
        let s = r#"
            name = "ok"
            version = "0.1.0"
            pmacs_required = ">=1.0.0"
            entry = "init.lua"
            exports = []
        "#;
        let err = PackageManifest::from_toml(s).unwrap_err();
        assert!(err.to_string().contains("summary"));
    }

    #[test]
    fn missing_pmacs_required_field_error_names_pmacs_required() {
        let s = r#"
            name = "ok"
            version = "0.1.0"
            summary = "x"
            entry = "init.lua"
            exports = []
        "#;
        let err = PackageManifest::from_toml(s).unwrap_err();
        assert!(err.to_string().contains("pmacs_required"));
    }

    #[test]
    fn missing_entry_field_error_names_entry() {
        let s = r#"
            name = "ok"
            version = "0.1.0"
            summary = "x"
            pmacs_required = ">=1.0.0"
            exports = []
        "#;
        let err = PackageManifest::from_toml(s).unwrap_err();
        assert!(err.to_string().contains("entry"));
    }

    #[test]
    fn missing_exports_field_error_names_exports() {
        let s = r#"
            name = "ok"
            version = "0.1.0"
            summary = "x"
            pmacs_required = ">=1.0.0"
            entry = "init.lua"
        "#;
        let err = PackageManifest::from_toml(s).unwrap_err();
        assert!(err.to_string().contains("exports"));
    }

    // -- Invalid semver rejected at parse time ------------------------------

    #[test]
    fn invalid_version_string_rejected_at_parse_time() {
        let s = r#"
            name = "ok"
            version = "not-a-semver"
            summary = "x"
            pmacs_required = ">=1.0.0"
            entry = "init.lua"
            exports = []
        "#;
        let err = PackageManifest::from_toml(s).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("version") || msg.contains("not-a-semver"),
            "expected version parse error, got: {msg}"
        );
    }

    #[test]
    fn invalid_pmacs_required_rejected_at_parse_time() {
        let s = r#"
            name = "ok"
            version = "0.1.0"
            summary = "x"
            pmacs_required = "garbage-version-req"
            entry = "init.lua"
            exports = []
        "#;
        let err = PackageManifest::from_toml(s).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("pmacs_required") || msg.contains("garbage-version-req"),
            "expected pmacs_required parse error, got: {msg}"
        );
    }

    #[test]
    fn invalid_dependency_version_rejected_at_parse_time() {
        let s = r#"
            name = "ok"
            version = "0.1.0"
            summary = "x"
            pmacs_required = ">=1.0.0"
            entry = "init.lua"
            exports = []
            [[dependencies]]
            address = "github:foo/bar"
            version = "not-a-req"
        "#;
        let err = PackageManifest::from_toml(s).unwrap_err();
        assert!(err.to_string().contains("dependencies") || err.to_string().contains("not-a-req"));
    }

    // -- PackageName validator coverage -------------------------------------

    #[test]
    fn package_name_accepts_simple_lowercase() {
        assert!(PackageName::new("pmacs-magit").is_ok());
        assert!(PackageName::new("a").is_ok());
        assert!(PackageName::new("a1").is_ok());
        assert!(PackageName::new("foo-bar-baz").is_ok());
    }

    #[test]
    fn package_name_accepts_namespace_form() {
        assert!(PackageName::new("user/pkg").is_ok());
        assert!(PackageName::new("user-name/pkg-name").is_ok());
    }

    #[test]
    fn package_name_rejects_uppercase() {
        let err = PackageName::new("Pmacs-magit").unwrap_err();
        assert!(matches!(err, ManifestError::InvalidName { .. }));
    }

    #[test]
    fn package_name_rejects_leading_digit() {
        assert!(PackageName::new("1pkg").is_err());
    }

    #[test]
    fn package_name_rejects_leading_hyphen() {
        assert!(PackageName::new("-pkg").is_err());
    }

    #[test]
    fn package_name_rejects_underscore() {
        assert!(PackageName::new("pkg_name").is_err());
    }

    #[test]
    fn package_name_rejects_empty() {
        assert!(PackageName::new("").is_err());
    }

    #[test]
    fn package_name_rejects_double_namespace() {
        assert!(PackageName::new("a/b/c").is_err());
    }

    #[test]
    fn package_name_rejects_trailing_slash() {
        assert!(PackageName::new("user/").is_err());
    }

    #[test]
    fn package_name_rejects_leading_slash() {
        assert!(PackageName::new("/pkg").is_err());
    }

    #[test]
    fn package_name_deserializes_with_validation() {
        let s = r#"
            name = "BAD-CAPS"
            version = "0.1.0"
            summary = "x"
            pmacs_required = ">=1.0.0"
            entry = "init.lua"
            exports = []
        "#;
        let err = PackageManifest::from_toml(s).unwrap_err();
        assert!(err.to_string().contains("BAD-CAPS") || err.to_string().contains("name"));
    }

    // -- Optional dependencies / conflicts respected ------------------------

    #[test]
    fn dependencies_default_to_empty_when_omitted() {
        let s = r#"
            name = "ok"
            version = "0.1.0"
            summary = "x"
            pmacs_required = ">=1.0.0"
            entry = "init.lua"
            exports = ["main"]
        "#;
        let m = PackageManifest::from_toml(s).unwrap();
        assert!(m.dependencies.is_empty());
        assert!(m.conflicts.is_empty());
    }

    // -- Property test: arbitrary valid manifest round-trips ----------------

    fn name_segment_strategy() -> impl Strategy<Value = String> {
        (
            prop::char::range('a', 'z'),
            prop::collection::vec(
                prop_oneof![
                    prop::char::range('a', 'z'),
                    prop::char::range('0', '9'),
                    Just('-'),
                ],
                0..8,
            ),
        )
            .prop_map(|(head, rest)| {
                let mut s = String::new();
                s.push(head);
                s.extend(rest);
                s
            })
    }

    fn package_name_strategy() -> impl Strategy<Value = PackageName> {
        prop_oneof![
            name_segment_strategy().prop_filter_map("must validate", |s| PackageName::new(s).ok()),
            (name_segment_strategy(), name_segment_strategy())
                .prop_filter_map("namespaced must validate", |(a, b)| {
                    PackageName::new(format!("{a}/{b}")).ok()
                }),
        ]
    }

    fn version_strategy() -> impl Strategy<Value = Version> {
        (0u8..16, 0u8..16, 0u8..16).prop_map(|(a, b, c)| Version::new(a.into(), b.into(), c.into()))
    }

    fn version_req_strategy() -> impl Strategy<Value = VersionReq> {
        // Stick to forms semver round-trips losslessly: an exact pin
        // formatted as `=major.minor.patch`. Caret/tilde/range-style
        // requirements have canonicalization quirks (e.g., `*` <->
        // `>=0.0.0`) that round-trip but compare unequal; the property
        // test asserts equality, so we stay in the always-equal subset.
        version_strategy().prop_map(|v| VersionReq::parse(&format!("={v}")).unwrap())
    }

    fn dep_spec_strategy() -> impl Strategy<Value = DependencySpec> {
        (
            prop::string::string_regex("[a-z][a-z0-9-]{0,15}").unwrap(),
            version_req_strategy(),
        )
            .prop_map(|(addr, ver)| DependencySpec {
                address: format!("github:user/{addr}"),
                version: ver,
            })
    }

    fn manifest_strategy() -> impl Strategy<Value = PackageManifest> {
        (
            package_name_strategy(),
            version_strategy(),
            // Summary: printable ASCII without quote/backslash hazards.
            prop::string::string_regex("[a-zA-Z0-9 .,!?]{0,40}").unwrap(),
            version_req_strategy(),
            prop::collection::vec(dep_spec_strategy(), 0..3),
            prop::collection::vec(dep_spec_strategy(), 0..3),
            prop::string::string_regex("[a-z][a-z0-9_/.]{0,16}\\.lua").unwrap(),
            prop::collection::vec(
                prop::string::string_regex("[a-z][a-z0-9_.]{0,16}").unwrap(),
                0..4,
            ),
        )
            .prop_map(
                |(name, version, summary, req, deps, conf, entry, exports)| PackageManifest {
                    name,
                    version,
                    summary,
                    pmacs_required: req,
                    dependencies: deps,
                    conflicts: conf,
                    entry: PathBuf::from(entry),
                    exports,
                },
            )
    }

    proptest! {
        #[test]
        fn arbitrary_valid_manifest_survives_round_trip(m in manifest_strategy()) {
            let s = m.to_toml().expect("serialize");
            let parsed = PackageManifest::from_toml(&s).expect("parse round-tripped output");
            prop_assert_eq!(parsed, m);
        }
    }
}
