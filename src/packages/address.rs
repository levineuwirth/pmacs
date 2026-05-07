// packages/address.rs --- Address parsing for v1.0 package addresses.

//! Address parsing (T M7.2, spec §sec:packages-future).
//!
//! v1.0 ships four address forms:
//!
//! - `github:owner/repo` --- sugar that expands to
//!   `https://github.com/owner/repo.git`. The `.git` suffix is
//!   tolerated; `github:owner/repo.git` is accepted and treated as
//!   equivalent.
//! - `gitlab:owner/repo` --- the same sugar against `gitlab.com`. A
//!   self-hosted GitLab instance is reachable via the `git:` form.
//! - `git:<URL>` --- the prefix is stripped and whatever remains is
//!   passed to `git clone` as-is. This intentionally accepts anything
//!   `git clone` accepts: full URLs (`https://`, `ssh://`, `file://`,
//!   `git://`), SSH shorthand (`git@host:path`), local paths. Validation
//!   that the URL actually resolves happens at clone time, not parse
//!   time --- delegating the URL-form question to git's existing
//!   documentation rather than maintaining our own parser.
//! - Raw URLs starting with `https://` or `git://` --- accepted directly
//!   without a prefix. The natural form `https://example.com/repo.git`
//!   parses without forcing the user to type a redundant `https:` or
//!   `git:` prefix.
//!
//! ## Forge aliases (deferred)
//!
//! `codeberg:` and `forgejo:` were considered for v1.0 and deferred
//! to a post-v1.0 patch release driven by user demand. The
//! extension path is the same one match arm `gitlab:` takes below;
//! see T M7.2 box in `pmacs-tasks.tex`. Inputs starting with these
//! prefixes return [`AddressError::DeferredAlias`], whose message
//! names the alias and points at the `git:URL` fallback.
//!
//! ## Authentication
//!
//! v1.0 delegates authentication to the user's git configuration: if
//! the system has a credential helper for HTTPS or an SSH agent for
//! SSH URLs, private repos work transparently. The address parser
//! does not handle credentials; it only produces the URL string that
//! `git clone` will eventually receive.

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Address
// ---------------------------------------------------------------------------

/// A parsed package address.
///
/// Three variants in v1.0: a special-cased GitHub form (the most
/// common), a special-cased GitLab.com form (the second most
/// common), and an opaque URL form (everything else).
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum Address {
    /// `github:owner/repo` sugar.
    Github {
        /// Repository owner (user or organization).
        owner: String,
        /// Repository name. Tolerates an optional `.git` suffix at parse
        /// time but stores the bare name.
        repo: String,
    },
    /// `gitlab:owner/repo` sugar against `gitlab.com`. Self-hosted
    /// GitLab instances must use the `git:` form with a full URL.
    /// v1.0 admits a single owner segment (user or top-level group);
    /// nested subgroups (`gitlab:group/subgroup/repo`) are out of
    /// scope for the sugar and need the explicit
    /// `git:https://gitlab.com/group/subgroup/repo.git` form.
    Gitlab {
        /// Repository owner (user or top-level group).
        owner: String,
        /// Repository name (with optional `.git` suffix tolerated).
        repo: String,
    },
    /// Any clone-cloneable URL or shorthand. Stored as-is; passed to
    /// `git clone` verbatim.
    Url(String),
}

impl Address {
    /// Parse an address string per the v1.0 syntax.
    pub fn parse(s: &str) -> Result<Self, AddressError> {
        if s.is_empty() {
            return Err(AddressError::Empty);
        }

        // Deferred forge aliases must be detected before generic prefix
        // handling so the error message points at the trim decision.
        for alias in DEFERRED_ALIASES {
            if s.starts_with(alias) {
                return Err(AddressError::DeferredAlias {
                    alias: (*alias).to_string(),
                    input: s.to_string(),
                });
            }
        }

        // 1. Raw URLs without a prefix --- accept directly. This branch
        //    must come before the `git:` prefix handler: `git://x`
        //    starts with `git:` and would otherwise be miscaptured as
        //    a `git:` prefix with body `//x`.
        if s.starts_with("https://") || s.starts_with("git://") {
            return Ok(Address::Url(s.to_string()));
        }

        // 2. github:owner/repo (with optional .git suffix).
        if let Some(rest) = s.strip_prefix("github:") {
            return parse_github(rest, s);
        }

        // 2a. gitlab:owner/repo (with optional .git suffix). Same
        // shape as the github sugar; the only difference is the
        // expanded clone URL host. Self-hosted GitLab instances are
        // not addressable via this prefix --- they must use the
        // generic `git:https://gitlab.example/...` form.
        if let Some(rest) = s.strip_prefix("gitlab:") {
            return parse_gitlab(rest, s);
        }

        // 3. git:<anything> --- pass-through. Whatever follows is fed to
        //    `git clone` as-is. Accepts SSH shorthand, file URLs, and
        //    arbitrary clone targets. Validation that the target is
        //    reachable happens at fetch time, not at parse time.
        if let Some(rest) = s.strip_prefix("git:") {
            if rest.is_empty() {
                return Err(AddressError::EmptyGitTarget {
                    input: s.to_string(),
                });
            }
            return Ok(Address::Url(rest.to_string()));
        }

        // 4. https:<rest> --- redundant verbose form, kept for symmetry
        //    with git:URL. The natural `https://...` form is already
        //    handled by branch 1; this branch covers the user who
        //    writes `https:https://...` by reflex. Anything else after
        //    the `https:` prefix that isn't a recognizable HTTPS body
        //    is rejected.
        if let Some(rest) = s.strip_prefix("https:") {
            if let Some(inner) = rest.strip_prefix("https://") {
                let _ = inner;
                return Ok(Address::Url(rest.to_string()));
            }
            return Err(AddressError::MalformedHttps {
                input: s.to_string(),
            });
        }

        Err(AddressError::UnknownScheme {
            input: s.to_string(),
        })
    }

    /// The clone URL this address resolves to. Pass to `git clone`.
    #[must_use]
    pub fn to_git_url(&self) -> String {
        match self {
            Self::Github { owner, repo } => {
                format!("https://github.com/{owner}/{repo}.git")
            }
            Self::Gitlab { owner, repo } => {
                format!("https://gitlab.com/{owner}/{repo}.git")
            }
            Self::Url(u) => u.clone(),
        }
    }
}

const DEFERRED_ALIASES: &[&str] = &["codeberg:", "forgejo:"];

fn parse_github(rest: &str, original: &str) -> Result<Address, AddressError> {
    let (owner, repo) = parse_forge_pair(
        rest,
        original,
        AddressError::InvalidGithub {
            input: original.to_string(),
        },
    )?;
    Ok(Address::Github { owner, repo })
}

fn parse_gitlab(rest: &str, original: &str) -> Result<Address, AddressError> {
    let (owner, repo) = parse_forge_pair(
        rest,
        original,
        AddressError::InvalidGitlab {
            input: original.to_string(),
        },
    )?;
    Ok(Address::Gitlab { owner, repo })
}

/// Shared parser for the `<forge>:<owner>/<repo>` sugar. Tolerates a
/// trailing `.git` (users type it by habit) and rejects obviously
/// malformed inputs (missing slash, extra segment, suspicious
/// characters). Conservative `[A-Za-z0-9_.-]` character class
/// covers every realistic case; wider sets can be admitted later
/// if a real package surfaces a rejection.
fn parse_forge_pair(
    rest: &str,
    _original: &str,
    err: AddressError,
) -> Result<(String, String), AddressError> {
    let body = rest.strip_suffix(".git").unwrap_or(rest);
    let mut parts = body.split('/');
    let owner = parts.next().unwrap_or("");
    let repo = parts.next().unwrap_or("");
    if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
        return Err(err);
    }
    for seg in [owner, repo] {
        if !seg
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
        {
            return Err(err);
        }
    }
    Ok((owner.to_string(), repo.to_string()))
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors produced by [`Address::parse`].
///
/// Every variant names the offending input. Forge-alias rejections
/// also point at the `git:URL` fallback so the user knows what to
/// type instead.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum AddressError {
    /// Input was empty.
    #[error("empty package address")]
    Empty,
    /// `github:owner/repo` form was malformed (missing slash, extra
    /// segment, invalid characters).
    #[error("invalid github address `{input}`: expected `github:owner/repo`")]
    InvalidGithub {
        /// The offending input.
        input: String,
    },
    /// `gitlab:owner/repo` form was malformed.
    #[error("invalid gitlab address `{input}`: expected `gitlab:owner/repo`")]
    InvalidGitlab {
        /// The offending input.
        input: String,
    },
    /// `git:` prefix was followed by an empty body.
    #[error("empty git target in `{input}`: expected `git:<URL>`")]
    EmptyGitTarget {
        /// The offending input.
        input: String,
    },
    /// `https:` prefix did not introduce a recognizable HTTPS URL.
    #[error("malformed https address `{input}`: expected `https://...`")]
    MalformedHttps {
        /// The offending input.
        input: String,
    },
    /// Address used a forge-alias prefix that v1.0 deferred
    /// (`codeberg:`, `forgejo:`). The message points at the
    /// `git:URL` fallback so the user knows what to type instead.
    #[error(
        "address scheme `{alias}` is deferred for v1.0; \
         use `git:<full-URL>` instead (e.g. `git:https://gitlab.com/owner/repo.git`). \
         Offending input: `{input}`"
    )]
    DeferredAlias {
        /// The deferred alias prefix (e.g. `"codeberg:"`).
        alias: String,
        /// The full offending input.
        input: String,
    },
    /// Address did not match any v1.0 scheme.
    #[error(
        "unknown address scheme in `{input}`; \
         expected `github:owner/repo`, `gitlab:owner/repo`, \
         `git:<URL>`, `https://...`, or `git://...`"
    )]
    UnknownScheme {
        /// The offending input.
        input: String,
    },
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- github sugar --------------------------------------------------------

    #[test]
    fn github_simple_form_parses() {
        let a = Address::parse("github:rust-lang/rust").unwrap();
        assert_eq!(
            a,
            Address::Github {
                owner: "rust-lang".into(),
                repo: "rust".into(),
            }
        );
    }

    #[test]
    fn github_dot_git_suffix_tolerated() {
        let a = Address::parse("github:owner/repo.git").unwrap();
        assert_eq!(
            a,
            Address::Github {
                owner: "owner".into(),
                repo: "repo".into(),
            }
        );
    }

    #[test]
    fn github_to_git_url_canonicalizes() {
        let a = Address::parse("github:foo/bar").unwrap();
        assert_eq!(a.to_git_url(), "https://github.com/foo/bar.git");
    }

    #[test]
    fn github_to_git_url_canonicalizes_after_dot_git_strip() {
        let a = Address::parse("github:foo/bar.git").unwrap();
        // The `.git` survives in the canonical URL even though the
        // parsed `repo` is `bar`.
        assert_eq!(a.to_git_url(), "https://github.com/foo/bar.git");
    }

    #[test]
    fn github_rejects_missing_repo() {
        let err = Address::parse("github:owner").unwrap_err();
        assert!(matches!(err, AddressError::InvalidGithub { .. }));
    }

    #[test]
    fn github_rejects_extra_segment() {
        let err = Address::parse("github:owner/repo/extra").unwrap_err();
        assert!(matches!(err, AddressError::InvalidGithub { .. }));
    }

    #[test]
    fn github_rejects_empty_owner() {
        let err = Address::parse("github:/repo").unwrap_err();
        assert!(matches!(err, AddressError::InvalidGithub { .. }));
    }

    #[test]
    fn github_rejects_empty_repo() {
        let err = Address::parse("github:owner/").unwrap_err();
        assert!(matches!(err, AddressError::InvalidGithub { .. }));
    }

    #[test]
    fn github_rejects_invalid_characters() {
        let err = Address::parse("github:owner/repo with spaces").unwrap_err();
        assert!(matches!(err, AddressError::InvalidGithub { .. }));
    }

    // -- git: prefix --------------------------------------------------------

    #[test]
    fn git_prefix_with_https_url() {
        let a = Address::parse("git:https://example.com/owner/repo.git").unwrap();
        assert_eq!(a, Address::Url("https://example.com/owner/repo.git".into()));
        assert_eq!(a.to_git_url(), "https://example.com/owner/repo.git");
    }

    #[test]
    fn git_prefix_with_ssh_url() {
        let a = Address::parse("git:ssh://git@example.com/owner/repo.git").unwrap();
        assert_eq!(
            a,
            Address::Url("ssh://git@example.com/owner/repo.git".into())
        );
    }

    #[test]
    fn git_prefix_with_ssh_shorthand() {
        // SSH shorthand isn't a URL but `git clone` accepts it. We pass
        // it through verbatim.
        let a = Address::parse("git:git@github.com:owner/repo.git").unwrap();
        assert_eq!(a, Address::Url("git@github.com:owner/repo.git".into()));
    }

    #[test]
    fn git_prefix_with_file_url() {
        let a = Address::parse("git:file:///tmp/test-repo").unwrap();
        assert_eq!(a, Address::Url("file:///tmp/test-repo".into()));
    }

    #[test]
    fn git_prefix_empty_is_rejected() {
        let err = Address::parse("git:").unwrap_err();
        assert!(matches!(err, AddressError::EmptyGitTarget { .. }));
    }

    // -- raw URL forms ------------------------------------------------------

    #[test]
    fn raw_https_url_accepted() {
        let a = Address::parse("https://example.com/owner/repo.git").unwrap();
        assert_eq!(a, Address::Url("https://example.com/owner/repo.git".into()));
    }

    #[test]
    fn raw_git_protocol_url_accepted() {
        let a = Address::parse("git://example.com/owner/repo").unwrap();
        assert_eq!(a, Address::Url("git://example.com/owner/repo".into()));
    }

    #[test]
    fn https_verbose_redundant_form_accepted() {
        let a = Address::parse("https:https://example.com/owner/repo").unwrap();
        // Verbose form: the inner `https://...` is what we keep.
        assert_eq!(a, Address::Url("https://example.com/owner/repo".into()));
    }

    #[test]
    fn https_prefix_without_url_body_rejected() {
        let err = Address::parse("https:example.com/repo").unwrap_err();
        assert!(matches!(err, AddressError::MalformedHttps { .. }));
    }

    // -- gitlab sugar -------------------------------------------------------

    #[test]
    fn gitlab_simple_form_parses() {
        let a = Address::parse("gitlab:user/repo").unwrap();
        assert_eq!(
            a,
            Address::Gitlab {
                owner: "user".into(),
                repo: "repo".into(),
            }
        );
    }

    #[test]
    fn gitlab_to_git_url_canonicalizes_to_gitlab_com() {
        let a = Address::parse("gitlab:user/repo").unwrap();
        assert_eq!(a.to_git_url(), "https://gitlab.com/user/repo.git");
    }

    #[test]
    fn gitlab_dot_git_suffix_tolerated() {
        let a = Address::parse("gitlab:user/repo.git").unwrap();
        assert_eq!(
            a,
            Address::Gitlab {
                owner: "user".into(),
                repo: "repo".into(),
            }
        );
        assert_eq!(a.to_git_url(), "https://gitlab.com/user/repo.git");
    }

    #[test]
    fn gitlab_rejects_missing_slash() {
        let err = Address::parse("gitlab:user").unwrap_err();
        assert!(matches!(err, AddressError::InvalidGitlab { .. }));
    }

    #[test]
    fn gitlab_rejects_extra_segment() {
        // Subgroups (`group/subgroup/repo`) aren't supported by the
        // sugar; users with subgroups go through `git:https://...`.
        let err = Address::parse("gitlab:group/sub/repo").unwrap_err();
        assert!(matches!(err, AddressError::InvalidGitlab { .. }));
    }

    // -- Forge aliases rejected with helpful pointer ------------------------

    #[test]
    fn codeberg_alias_rejected_with_pointer_to_git_fallback() {
        let err = Address::parse("codeberg:owner/repo").unwrap_err();
        let msg = err.to_string();
        assert!(matches!(err, AddressError::DeferredAlias { .. }));
        assert!(msg.contains("codeberg:"));
        assert!(msg.contains("git:"));
    }

    #[test]
    fn forgejo_alias_rejected_with_pointer_to_git_fallback() {
        let err = Address::parse("forgejo:host/owner/repo").unwrap_err();
        let msg = err.to_string();
        assert!(matches!(err, AddressError::DeferredAlias { .. }));
        assert!(msg.contains("forgejo:"));
        assert!(msg.contains("git:"));
    }

    // -- Catch-alls ---------------------------------------------------------

    #[test]
    fn empty_input_rejected() {
        let err = Address::parse("").unwrap_err();
        assert!(matches!(err, AddressError::Empty));
    }

    #[test]
    fn unknown_scheme_rejected_with_help() {
        let err = Address::parse("ftp://example.com/repo").unwrap_err();
        let msg = err.to_string();
        assert!(matches!(err, AddressError::UnknownScheme { .. }));
        assert!(
            msg.contains("github:"),
            "error should list known schemes: {msg}"
        );
        assert!(msg.contains("git:"));
    }

    #[test]
    fn http_unsupported_falls_to_unknown_scheme() {
        // Plain http:// is not in v1.0's set --- users should use https.
        let err = Address::parse("http://example.com/repo").unwrap_err();
        assert!(matches!(err, AddressError::UnknownScheme { .. }));
    }

    #[test]
    fn bare_word_rejected() {
        let err = Address::parse("just-a-word").unwrap_err();
        assert!(matches!(err, AddressError::UnknownScheme { .. }));
    }
}
