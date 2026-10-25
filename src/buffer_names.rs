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
//! name equal to a whole path) takes a `<N>` suffix, the registry's
//! order deciding which.
//!
//! The registry calls [`derive_names`] whenever the set of names can
//! have changed ([`crate::buffer_registry::BufferRegistry::refresh_names`]).

use std::collections::{HashMap, HashSet};
use std::hash::BuildHasher;
use std::path::{Component, Path};

/// One buffer to name: the path its name derives from, and whether that
/// path is the directory it lists (named with a trailing `/`) rather
/// than the file it holds.
#[derive(Clone, Copy, Debug)]
pub struct NameRequest<'a> {
    /// The buffer's normalized path.
    pub path: &'a Path,
    /// True for a directory listing (dired), false for a file.
    pub directory: bool,
}

/// The names of `requests`, in order: each the fewest trailing
/// components of its path that no other request's name and no
/// `reserved` name shares, written forward with `/` between, a
/// directory's ending in `/`.
///
/// A name grows one component at a time while it collides, which ends
/// at the smallest suffix telling it apart: two names with one basename
/// share every shorter suffix once they share a longer one, so growing
/// both never passes the first suffix that differs. A path that runs out
/// of components is named by the whole of it, leading `/` included.
#[must_use]
pub fn derive_names<S: BuildHasher>(
    requests: &[NameRequest<'_>],
    reserved: &HashSet<&str, S>,
) -> Vec<String> {
    // One key per distinct path: a path held twice is one name and a
    // suffix, never a reason to grow.
    let mut keys: Vec<(Vec<String>, bool)> = Vec::new();
    let mut index: HashMap<(Vec<String>, bool), usize> = HashMap::new();
    let key_of: Vec<usize> = requests
        .iter()
        .map(|request| {
            let key = (components(request.path), request.directory);
            *index.entry(key.clone()).or_insert_with(|| {
                keys.push(key);
                keys.len() - 1
            })
        })
        .collect();
    let mut depth = vec![1usize; keys.len()];
    let names = loop {
        let names: Vec<String> = keys
            .iter()
            .zip(&depth)
            .map(|((parts, directory), &depth)| render(parts, depth, *directory))
            .collect();
        let mut count: HashMap<&str, usize> = HashMap::new();
        for name in &names {
            *count.entry(name.as_str()).or_default() += 1;
        }
        let mut grew = false;
        for (i, name) in names.iter().enumerate() {
            let collides = count[name.as_str()] > 1 || reserved.contains(name.as_str());
            if collides && depth[i] <= keys[i].0.len() {
                depth[i] += 1;
                grew = true;
            }
        }
        if !grew {
            break names;
        }
    };
    let per_request: Vec<String> = key_of.iter().map(|&k| names[k].clone()).collect();
    suffix_total_collisions(&per_request, reserved)
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

/// Give a `<N>` suffix to every name still shared after growing, or
/// still reserved: the first holder of an unreserved name keeps it, and
/// the rest take the lowest free `<N>`, from 2, in order.
fn suffix_total_collisions<S: BuildHasher>(
    names: &[String],
    reserved: &HashSet<&str, S>,
) -> Vec<String> {
    let mut taken: HashSet<String> = reserved.iter().map(|&s| s.to_owned()).collect();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for name in names {
        *seen.entry(name.clone()).or_default() += 1;
    }
    let mut out = vec![String::new(); names.len()];
    let mut pending = Vec::new();
    for (i, name) in names.iter().enumerate() {
        if seen[name] == 1 && !taken.contains(name) {
            taken.insert(name.clone());
            out[i].clone_from(name);
        } else {
            pending.push(i);
        }
    }
    for i in pending {
        let base = &names[i];
        if !taken.contains(base) {
            taken.insert(base.clone());
            out[i].clone_from(base);
            continue;
        }
        let mut n = 2usize;
        loop {
            let candidate = format!("{base}<{n}>");
            if taken.insert(candidate.clone()) {
                out[i] = candidate;
                break;
            }
            n += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn files(paths: &[&str]) -> Vec<String> {
        let owned: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        let requests: Vec<NameRequest<'_>> = owned
            .iter()
            .map(|p| NameRequest {
                path: p,
                directory: false,
            })
            .collect();
        derive_names(&requests, &HashSet::new())
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

    #[test]
    fn a_directory_ends_in_a_slash_and_never_meets_a_file() {
        let proj_a = PathBuf::from("/x/a/proj");
        let proj_b = PathBuf::from("/x/b/proj");
        let file = PathBuf::from("/x/a/proj.d/proj");
        let root = PathBuf::from("/");
        let requests = [
            NameRequest {
                path: &proj_a,
                directory: true,
            },
            NameRequest {
                path: &proj_b,
                directory: true,
            },
            NameRequest {
                path: &file,
                directory: false,
            },
            NameRequest {
                path: &root,
                directory: true,
            },
        ];
        assert_eq!(
            derive_names(&requests, &HashSet::new()),
            ["a/proj/", "b/proj/", "proj", "/"]
        );
    }

    #[test]
    fn a_reserved_name_moves_the_derived_one_and_keeps_its_own() {
        let a = PathBuf::from("/p/src/a.rs");
        let requests = [NameRequest {
            path: &a,
            directory: false,
        }];
        let reserved: HashSet<&str> = ["a.rs", "*scratch*"].into_iter().collect();
        assert_eq!(derive_names(&requests, &reserved), ["src/a.rs"]);
        let whole: HashSet<&str> = ["a.rs", "src/a.rs", "p/src/a.rs", "/p/src/a.rs"]
            .into_iter()
            .collect();
        assert_eq!(derive_names(&requests, &whole), ["/p/src/a.rs<2>"]);
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
}
