// buffer_names.rs --- The names of path-backed buffers (D40).

//! The one naming function for buffers whose name derives from a path
//! (E10c, D40): a file buffer is named by its file's basename and a
//! directory listing by its directory's last component and a `/`
//! (`proj/`). Two derived names that collide each take the fewest
//! trailing directories that tell them apart, written forward
//! (`src/a.rs` beside `tests/a.rs`, `a/proj/` beside `b/proj/`), so a
//! survivor returns to its basename when the other goes.
//!
//! A name that does not derive from a path (an explicit one, `*scratch*`,
//! a terminal's) is never rewritten here: it is *reserved*, and a derived
//! name that would equal it takes more of its path instead. Only a name
//! no amount of path can tell apart (one path held twice, or an explicit
//! name equal to a whole path) takes a `<N>` suffix, the holders' order
//! deciding which.
//!
//! The rule is two functions, [`name_depth`] (which of a path's names it
//! is shown by) and [`name_holders`] (the names of the buffers holding
//! one path), which [`derive_names`] applies to a set at once and the
//! registry ([`crate::buffer_registry::BufferRegistry`]) applies one path
//! at a time: a path's name depends only on which of its names other
//! paths share and which are reserved, so an event renames the paths
//! whose names it shares and leaves the rest of the session alone (E10c
//! review 1's Medium 1).

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

/// A path split once into every name it can be shown by: its basename,
/// then each longer run of trailing components written forward with `/`
/// between, then the whole path from the root; a directory's each end in
/// `/`. The registry builds one when a buffer is named by a path and
/// keeps it until the path changes, so naming renders nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamePath {
    /// The path the names are of.
    path: PathBuf,
    /// True for a directory listing (dired), false for a file.
    directory: bool,
    /// `names[0]` the basename, `names[k]` the last `k + 1` components,
    /// the last of them the whole path.
    names: Vec<String>,
}

impl NamePath {
    /// Split `path`, a normalized buffer path, into its names; a
    /// `directory` is the one a listing shows (named `proj/`).
    #[must_use]
    pub fn new(path: &Path, directory: bool) -> Self {
        let parts = components(path);
        let names = (1..=parts.len() + 1)
            .map(|depth| render(&parts, depth, directory))
            .collect();
        Self {
            path: path.to_path_buf(),
            directory,
            names,
        }
    }

    /// True iff this is the split of `path` as a directory or a file.
    #[must_use]
    pub fn is_of(&self, path: &Path, directory: bool) -> bool {
        self.directory == directory && self.path == path
    }

    /// Every name this path can be shown by, shortest first.
    #[must_use]
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// The group this path's names fall in, its basename (`a.rs`,
    /// `proj/`): no name of a path outside the group can equal one of
    /// its own but by a `<N>` suffix ([`groups_of_name`]).
    #[must_use]
    pub fn group(&self) -> &str {
        &self.names[0]
    }

    /// The whole path as a name, which tells this path from every other.
    #[must_use]
    pub fn whole(&self) -> &str {
        self.names.last().expect("a path has at least one name")
    }
}

/// Which of `path`'s names it is shown by, as an index into
/// [`NamePath::names`]: the first that no other path has among its own
/// (`shared` false) and no reserved name holds (`reserved` false), or
/// the whole path when every one is shared or reserved.
///
/// This is where growing every colliding name a component at a time
/// arrives: two paths that share a name share every shorter one, so they
/// grow in step and part at the first name that tells them apart, and a
/// reserved name lengthens only the name equal to it. Computed this way
/// a path's name reads only its own names, so it changes only when one
/// of them becomes shared, unshared, reserved or free.
#[must_use]
pub fn name_depth(
    path: &NamePath,
    shared: impl Fn(&str) -> bool,
    reserved: impl Fn(&str) -> bool,
) -> usize {
    path.names
        .iter()
        .position(|name| !shared(name) && !reserved(name))
        .unwrap_or(path.names.len() - 1)
}

/// The names of the buffers holding one path whose name is `base`, in
/// their order, each holding the name in `holds` now. A buffer keeps the
/// name it holds while that is still one of the path's: the buffer
/// already holding `base` keeps it (or else the first takes it), unless a
/// reserved name holds it (`base_free` false), and a buffer holding one
/// of the path's `<N>` keeps that; every other takes the lowest `<N>`,
/// from 2, that `taken` does not refuse. So a save-as or a rename onto an
/// open file's path leaves every buffer showing that file its name and
/// gives the newcomer the next `<N>`, as Emacs's `write-file` does (E10c
/// review 1's Low 2, the owner's ruling of 2026-10-25). `taken` answers
/// for a name some buffer other than these holds. A path's name is never
/// another path's ([`name_depth`] gives each a name of its own), so
/// `base` needs no such check and a `<N>` meets only reserved names and
/// names of paths that end in a `<N>` of their own.
pub fn name_holders<'a>(
    base: &'a str,
    holds: &[&str],
    base_free: bool,
    taken: impl Fn(&str) -> bool,
) -> Vec<Cow<'a, str>> {
    let mut out: Vec<Option<Cow<'a, str>>> = vec![None; holds.len()];
    if base_free && !holds.is_empty() {
        let keeper = holds.iter().position(|&h| h == base).unwrap_or(0);
        out[keeper] = Some(Cow::Borrowed(base));
    }
    let mut used: HashSet<String> = HashSet::new();
    for (slot, &held) in out.iter_mut().zip(holds) {
        if slot.is_none() && is_suffixed(base, held) && !taken(held) && used.insert(held.to_owned())
        {
            *slot = Some(Cow::Owned(held.to_owned()));
        }
    }
    let mut n = 2usize;
    for slot in &mut out {
        while slot.is_none() {
            let candidate = format!("{base}<{n}>");
            n += 1;
            if !taken(&candidate) && used.insert(candidate.clone()) {
                *slot = Some(Cow::Owned(candidate));
            }
        }
    }
    out.into_iter()
        .map(|name| name.expect("every holder is named"))
        .collect()
}

/// True iff `name` is `base` with a `<N>` of the kind [`name_holders`]
/// gives, `N` from 2.
fn is_suffixed(base: &str, name: &str) -> bool {
    name.strip_prefix(base)
        .and_then(|rest| rest.strip_prefix('<'))
        .and_then(|rest| rest.strip_suffix('>'))
        .and_then(|digits| digits.parse::<usize>().ok())
        .is_some_and(|n| n >= 2 && name.len() == base.len() + n.to_string().len() + 2)
}

/// One buffer to name: the path its name derives from, and the name it
/// holds now.
#[derive(Clone, Copy, Debug)]
pub struct NameRequest<'a> {
    /// The buffer's path, split ([`NamePath::new`]).
    pub path: &'a NamePath,
    /// The name the buffer holds now, which it keeps where it shares its
    /// path with another ([`name_holders`]).
    pub holds: &'a str,
}

/// The names of `requests` at once, in order: each the fewest trailing
/// components of its path that no other request's path shares and no
/// `reserved` name holds, written forward with `/` between, a
/// directory's ending in `/` ([`name_depth`]), and the buffers sharing
/// one path told apart by `<N>` ([`name_holders`]). A path that runs out
/// of components is named by the whole of it, leading `/` included.
#[must_use]
pub fn derive_names<'a>(
    requests: &[NameRequest<'a>],
    reserved: impl Fn(&str) -> bool,
) -> Vec<Cow<'a, str>> {
    // One key per distinct path: a path held twice is one name and a
    // suffix, never a reason to grow.
    let mut keys: Vec<&'a NamePath> = Vec::new();
    let mut index: HashMap<&'a str, usize> = HashMap::new();
    let mut holders: Vec<Vec<usize>> = Vec::new();
    for (i, request) in requests.iter().enumerate() {
        let k = *index.entry(request.path.whole()).or_insert_with(|| {
            keys.push(request.path);
            holders.push(Vec::new());
            keys.len() - 1
        });
        holders[k].push(i);
    }
    let mut shared: HashMap<&str, usize> = HashMap::new();
    for path in &keys {
        for name in &path.names {
            *shared.entry(name).or_default() += 1;
        }
    }
    let bases: Vec<&'a str> = keys
        .iter()
        .map(|path| path.names[name_depth(path, |n| shared[n] > 1, &reserved)].as_str())
        .collect();
    let all_bases: HashSet<&str> = bases.iter().copied().collect();
    let mut out: Vec<Option<Cow<'a, str>>> = vec![None; requests.len()];
    for (k, members) in holders.iter().enumerate() {
        let holds: Vec<&str> = members.iter().map(|&i| requests[i].holds).collect();
        let names = name_holders(bases[k], &holds, !reserved(bases[k]), |n| {
            reserved(n) || all_bases.contains(n)
        });
        for (&i, name) in members.iter().zip(names) {
            out[i] = Some(name);
        }
    }
    out.into_iter()
        .map(|name| name.expect("every request is named"))
        .collect()
}

/// The groups `name` can meet: the group of its last component, and,
/// for a name ending in a `<N>` suffix, the group of the name before it
/// (`a.rs<2>` is a file of that name or a second `a.rs`).
pub fn groups_of_name(name: &str) -> impl Iterator<Item = &str> {
    std::iter::once(group_of(name)).chain(without_suffix(name).map(group_of))
}

/// The basename a name ends with, as a [`NamePath::group`] writes it.
fn group_of(name: &str) -> &str {
    let body = name.strip_suffix('/').unwrap_or(name);
    body.rfind('/').map_or(name, |slash| &name[slash + 1..])
}

/// `name` without its `<N>` suffix, if it ends with one: the name whose
/// `<N>` it may be.
#[must_use]
pub fn without_suffix(name: &str) -> Option<&str> {
    let body = name.strip_suffix('>')?;
    let open = body.rfind('<')?;
    let digits = &body[open + 1..];
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then_some(&name[..open])
}

/// The normal components of `path`, each as text.
fn components(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

/// The last `depth` of `parts` joined by `/`, or the whole path from
/// the root once `depth` passes them; a directory's ends in `/`.
fn render(parts: &[String], depth: usize, directory: bool) -> String {
    let mut name = if depth > parts.len() {
        format!("/{}", parts.join("/"))
    } else {
        parts[parts.len() - depth..].join("/")
    };
    if directory && !name.ends_with('/') {
        name.push('/');
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(paths: &[&str], directory: bool) -> Vec<NamePath> {
        paths
            .iter()
            .map(|p| NamePath::new(Path::new(p), directory))
            .collect()
    }

    fn name_all(paths: &[NamePath], reserved: &HashSet<&str>) -> Vec<String> {
        let requests: Vec<NameRequest<'_>> = paths
            .iter()
            .map(|path| NameRequest { path, holds: "" })
            .collect();
        derive_names(&requests, |name| reserved.contains(name))
            .into_iter()
            .map(Cow::into_owned)
            .collect()
    }

    fn files(paths: &[&str]) -> Vec<String> {
        name_all(&split(paths, false), &HashSet::new())
    }

    #[test]
    fn a_lone_file_is_its_basename() {
        assert_eq!(files(&["/p/src/a.rs"]), ["a.rs"]);
    }

    #[test]
    fn two_alike_basenames_take_one_directory_each_written_forward() {
        assert_eq!(
            files(&["/p/src/a.rs", "/p/tests/a.rs", "/p/b.rs"]),
            ["src/a.rs", "tests/a.rs", "b.rs"]
        );
    }

    #[test]
    fn each_takes_only_the_directories_it_needs() {
        assert_eq!(
            files(&["/a/x/src/a.rs", "/b/x/src/a.rs", "/c/tests/a.rs"]),
            ["a/x/src/a.rs", "b/x/src/a.rs", "tests/a.rs"]
        );
    }

    #[test]
    fn a_path_that_runs_out_is_named_from_the_root() {
        assert_eq!(files(&["/a.rs", "/x/a.rs"]), ["/a.rs", "x/a.rs"]);
    }

    #[test]
    fn one_path_held_twice_takes_a_suffix_not_its_whole_path() {
        assert_eq!(files(&["/p/a.rs", "/p/a.rs"]), ["a.rs", "a.rs<2>"]);
    }

    /// E10c review 1's Low 2: where buffers share one path, the one
    /// already holding the plain name keeps it, wherever it stands in
    /// the order, and one holding a `<N>` keeps that; the rest take the
    /// lowest free `<N>` in order, and where none holds the plain name,
    /// the first takes it.
    #[test]
    fn the_buffer_already_holding_the_plain_name_keeps_it() {
        let a = NamePath::new(Path::new("/p/a.rs"), false);
        let named = |holds: &[&str]| -> Vec<String> {
            let requests: Vec<NameRequest<'_>> = holds
                .iter()
                .map(|&holds| NameRequest { path: &a, holds })
                .collect();
            derive_names(&requests, |_| false)
                .into_iter()
                .map(Cow::into_owned)
                .collect()
        };
        assert_eq!(named(&["*scratch*", "a.rs"]), ["a.rs<2>", "a.rs"]);
        assert_eq!(
            named(&["a.rs<3>", "a.rs<2>", "a.rs"]),
            ["a.rs<3>", "a.rs<2>", "a.rs"]
        );
        assert_eq!(
            named(&["x", "a.rs<3>", "a.rs"]),
            ["a.rs<2>", "a.rs<3>", "a.rs"]
        );
        assert_eq!(
            named(&["a.rs<03>", "a.rs<1>", "y"]),
            ["a.rs", "a.rs<2>", "a.rs<3>"]
        );
        assert_eq!(named(&["x", "y"]), ["a.rs", "a.rs<2>"]);
    }

    #[test]
    fn a_directory_ends_in_a_slash_and_never_meets_a_file() {
        let mut paths = split(&["/x/a/proj", "/x/b/proj"], true);
        paths.extend(split(&["/x/a/proj.d/proj"], false));
        paths.extend(split(&["/"], true));
        assert_eq!(
            name_all(&paths, &HashSet::new()),
            ["a/proj/", "b/proj/", "proj", "/"]
        );
    }

    #[test]
    fn a_reserved_name_moves_the_derived_one_and_keeps_its_own() {
        let paths = split(&["/p/src/a.rs"], false);
        let reserved: HashSet<&str> = ["a.rs", "*scratch*"].into_iter().collect();
        assert_eq!(name_all(&paths, &reserved), ["src/a.rs"]);
        let whole: HashSet<&str> = ["a.rs", "src/a.rs", "p/src/a.rs", "/p/src/a.rs"]
            .into_iter()
            .collect();
        assert_eq!(name_all(&paths, &whole), ["/p/src/a.rs<2>"]);
    }

    /// A path's group is its basename, and a name can meet only the
    /// group of its last component and, through a `<N>` suffix, the
    /// group of the name before it: what lets the registry rename within
    /// a group and look across one only for a `<N>`.
    #[test]
    fn a_name_meets_only_the_groups_of_its_basename() {
        let [file, listing, root] = [
            NamePath::new(Path::new("/p/src/a.rs"), false),
            NamePath::new(Path::new("/p/proj"), true),
            NamePath::new(Path::new("/"), true),
        ];
        assert_eq!(
            [file.group(), listing.group(), root.group()],
            ["a.rs", "proj/", "/"]
        );
        for (name, groups) in [
            ("a.rs", vec!["a.rs"]),
            ("src/a.rs", vec!["a.rs"]),
            ("/p/src/a.rs", vec!["a.rs"]),
            ("a/proj/", vec!["proj/"]),
            ("/", vec!["/"]),
            ("a.rs<2>", vec!["a.rs<2>", "a.rs"]),
            ("src/a.rs<12>", vec!["a.rs<12>", "a.rs"]),
            ("proj/<2>", vec!["<2>", "proj/"]),
            ("*scratch*", vec!["*scratch*"]),
            ("a.rs<x>", vec!["a.rs<x>"]),
        ] {
            assert_eq!(groups_of_name(name).collect::<Vec<_>>(), groups, "{name}");
        }
    }

    /// The definition, checked against every subset of a crowded set of
    /// paths: each name is the shortest trailing run of its path's
    /// components that no other path in the subset ends with, and no two
    /// names are equal.
    #[test]
    fn every_name_is_the_shortest_suffix_no_other_path_shares() {
        let pool = [
            "/a/x/src/a.rs",
            "/b/x/src/a.rs",
            "/c/tests/a.rs",
            "/c/src/a.rs",
            "/a.rs",
            "/d/a.rs",
            "/d/b.rs",
            "/e/src/b.rs",
        ];
        let parts = |p: &str| p.split('/').filter(|s| !s.is_empty()).count();
        let suffix = |p: &str, k: usize| {
            let parts: Vec<&str> = p.split('/').filter(|s| !s.is_empty()).collect();
            parts[parts.len() - k..].join("/")
        };
        let mut subsets = 0;
        for mask in 1u32..(1 << pool.len()) {
            let chosen: Vec<&str> = (0..pool.len())
                .filter(|i| mask & (1 << i) != 0)
                .map(|i| pool[i])
                .collect();
            let names = files(&chosen);
            let unique: HashSet<&String> = names.iter().collect();
            assert_eq!(unique.len(), names.len(), "{chosen:?} -> {names:?}");
            for (i, path) in chosen.iter().enumerate() {
                let expected = (1..=parts(path))
                    .map(|k| suffix(path, k))
                    .find(|s| {
                        chosen
                            .iter()
                            .enumerate()
                            .all(|(j, other)| j == i || !other.ends_with(&format!("/{s}")))
                    })
                    .unwrap_or_else(|| (*path).to_owned());
                assert_eq!(names[i], expected, "{chosen:?}");
            }
            subsets += 1;
        }
        assert_eq!(subsets, 255);
    }

    /// The names as `0adb83e` grew them, every colliding name one
    /// component at a time until none collides or each has run out:
    /// kept as the reference [`name_depth`] must agree with.
    fn grown(paths: &[NamePath], reserved: &HashSet<&str>) -> Vec<String> {
        let mut depth = vec![0usize; paths.len()];
        loop {
            let mut count: HashMap<&str, usize> = HashMap::new();
            for (path, &d) in paths.iter().zip(&depth) {
                *count.entry(path.names[d].as_str()).or_default() += 1;
            }
            let mut grew = false;
            for (path, d) in paths.iter().zip(depth.iter_mut()) {
                let name = path.names[*d].as_str();
                if (count[name] > 1 || reserved.contains(name)) && *d + 1 < path.names.len() {
                    *d += 1;
                    grew = true;
                }
            }
            if !grew {
                return paths
                    .iter()
                    .zip(&depth)
                    .map(|(p, &d)| p.names[d].clone())
                    .collect();
            }
        }
    }

    /// The first name no other path shares and nothing reserves is the
    /// name growing reaches, over seeded sets of crowded distinct paths,
    /// files and listings, with reserved names among their names.
    #[test]
    fn the_first_free_name_is_the_name_growing_reaches() {
        const POOL: [&str; 10] = [
            "/w/src/a.rs",
            "/w/tests/a.rs",
            "/w/x/src/a.rs",
            "/v/x/src/a.rs",
            "/a.rs",
            "/x/a.rs",
            "/w/a.rs",
            "/w/proj",
            "/v/proj",
            "/proj",
        ];
        const RESERVED: [&str; 8] = [
            "a.rs",
            "src/a.rs",
            "x/src/a.rs",
            "/a.rs",
            "proj/",
            "w/a.rs",
            "proj",
            "x/a.rs",
        ];
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..2000 {
            let mask = next();
            let mut paths: Vec<NamePath> = Vec::new();
            for (i, p) in POOL.iter().enumerate() {
                if mask & (1 << i) != 0 {
                    paths.push(NamePath::new(Path::new(p), mask & (1 << (i + 10)) != 0));
                }
            }
            let reserved: HashSet<&str> = RESERVED
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << (i + 20)) != 0)
                .map(|(_, r)| *r)
                .collect();
            let reference = grown(&paths, &reserved);
            let shared = |n: &str| {
                paths
                    .iter()
                    .filter(|p| p.names.iter().any(|m| m == n))
                    .count()
                    > 1
            };
            let firsts: Vec<String> = paths
                .iter()
                .map(|path| path.names[name_depth(path, shared, |n| reserved.contains(n))].clone())
                .collect();
            assert_eq!(firsts, reference, "{paths:?} reserved {reserved:?}");
        }
    }
}
