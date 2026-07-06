// lua_bindings/index.rs --- pmacs.index: project-scoped symbol index.

//! `pmacs.index.*` — the project symbol index surface (T M4.10). Split out
//! of `lua_bindings.rs` verbatim (audit F-016); behavior unchanged. Pure
//! leaf: depends only on `crate::project_index`, mlua, and std — no
//! shared-core coupling.

use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Lua, Table, Value};

// `lua_to_json` is a generic JSON converter that still physically lives in
// the `lsp` section of `mod.rs`; reachable here as a parent-private item.
// (A later tranche hoists it into shared core proper.)
use super::lua_to_json;

use crate::project_index::{
    FileEntry, ProjectIndexer, SearchHit, Symbol, SymbolKind, SymbolSource, extract_heuristic,
    fnv1a_64, ingest_lsp_symbols,
};

/// Cheaply-cloneable shared project index registry.
pub type SharedProjectIndexer = Rc<RefCell<ProjectIndexer>>;

fn symbol_kind_from_lua(tag: &str) -> SymbolKind {
    match tag {
        "function" => SymbolKind::Function,
        "method" => SymbolKind::Method,
        "struct" => SymbolKind::Struct,
        "class" => SymbolKind::Class,
        "trait" | "interface" => SymbolKind::Trait,
        "enum" => SymbolKind::Enum,
        "variable" => SymbolKind::Variable,
        "constant" => SymbolKind::Constant,
        "field" | "property" => SymbolKind::Field,
        "module" | "namespace" => SymbolKind::Module,
        "macro" => SymbolKind::Macro,
        "type_alias" | "type" => SymbolKind::TypeAlias,
        other => SymbolKind::Other(other.to_owned()),
    }
}

fn lua_symbol_from_table(t: &Table) -> mlua::Result<Symbol> {
    let name: String = t.get("name")?;
    let kind_tag: Option<String> = t.get("kind").ok().flatten();
    let kind = kind_tag
        .as_deref()
        .map_or(SymbolKind::Other("unknown".into()), symbol_kind_from_lua);
    let line: u32 = t.get("line").unwrap_or(0);
    let col: u32 = t.get("col").unwrap_or(0);
    let source_tag: Option<String> = t.get("source").ok().flatten();
    let source = source_tag
        .as_deref()
        .and_then(SymbolSource::from_tag)
        .unwrap_or(SymbolSource::Lua);
    let container: Option<String> = t.get("container").ok().flatten();
    Ok(Symbol {
        name,
        kind,
        line,
        col,
        source,
        container,
    })
}

fn search_hit_to_lua(lua: &Lua, hit: &SearchHit) -> mlua::Result<Table> {
    let t = lua.create_table_with_capacity(0, 9)?;
    t.set("name", hit.name.as_str())?;
    t.set("kind", hit.kind.tag())?;
    t.set("source", hit.source.tag())?;
    t.set("path", hit.path.display().to_string())?;
    t.set("relative_path", hit.relative_path.display().to_string())?;
    t.set("line", hit.line)?;
    t.set("col", hit.col)?;
    t.set("score", hit.score)?;
    if let Some(c) = &hit.container {
        t.set("container", c.as_str())?;
    }
    if let Some(l) = &hit.language {
        t.set("language", l.as_str())?;
    }
    Ok(t)
}

/// Install `pmacs.index.*` (T M4.10). Preserves any existing
/// `pmacs.index` keys (e.g. user-supplied indexer extensions
/// installed by builtin Lua chunks).
#[allow(
    clippy::too_many_lines,
    reason = "linear list of index bindings; splitting adds ceremony without clarity"
)]
pub fn install_project_index(lua: &Lua, indexer: &SharedProjectIndexer) -> mlua::Result<()> {
    let pmacs: Table = lua.globals().get("pmacs")?;
    let m: Table = match pmacs.get::<Option<Table>>("index")? {
        Some(t) => t,
        None => lua.create_table()?,
    };

    {
        // open(root) -> root_string. Ensures an index exists for
        // `root`; idempotent. Returns the canonicalised root the
        // caller should pass back to subsequent calls.
        let ix = indexer.clone();
        m.set(
            "open",
            lua.create_function(move |_, root: String| {
                let mut ix_ref = ix.borrow_mut();
                let idx = ix_ref.ensure(std::path::PathBuf::from(&root));
                Ok(idx.root.display().to_string())
            })?,
        )?;
    }

    {
        // close(root): drop the in-memory index. Does not touch disk.
        let ix = indexer.clone();
        m.set(
            "close",
            lua.create_function(move |_, root: String| {
                Ok(ix.borrow_mut().forget(std::path::Path::new(&root)))
            })?,
        )?;
    }

    {
        // upsert_file(root, path, language, source) -> { added }.
        // Runs the heuristic extractor on `source`, hashes it, and
        // replaces the entry for `path`.
        let ix = indexer.clone();
        m.set(
            "upsert_file",
            lua.create_function(
                move |lua,
                      (root, path, language, source): (
                    String,
                    String,
                    Option<String>,
                    String,
                )| {
                    let mut ix_ref = ix.borrow_mut();
                    let idx = ix_ref.ensure(std::path::PathBuf::from(&root));
                    let lang = language.as_deref().unwrap_or("");
                    let symbols = if lang.is_empty() {
                        crate::project_index::extract_raw(&source)
                    } else {
                        extract_heuristic(lang, &source)
                    };
                    let added = symbols.len();
                    let entry = FileEntry {
                        path: std::path::PathBuf::from(&path),
                        mtime_secs: 0,
                        content_hash: fnv1a_64(source.as_bytes()),
                        language: language.clone(),
                        symbols,
                    };
                    idx.upsert_file(entry);
                    let t = lua.create_table_with_capacity(0, 1)?;
                    t.set("added", added)?;
                    Ok(t)
                },
            )?,
        )?;
    }

    {
        // upsert_symbols(root, path, language, symbol_array): push
        // pre-extracted symbols (e.g. from a Lua-side indexer) into
        // the index. Each entry is a table with name/kind/line/col/
        // source/container fields.
        let ix = indexer.clone();
        m.set(
            "upsert_symbols",
            lua.create_function(
                move |_,
                      (root, path, language, symbols): (
                    String,
                    String,
                    Option<String>,
                    Vec<Table>,
                )| {
                    let mut parsed = Vec::with_capacity(symbols.len());
                    for t in &symbols {
                        parsed.push(lua_symbol_from_table(t)?);
                    }
                    let added = parsed.len();
                    let mut ix_ref = ix.borrow_mut();
                    let idx = ix_ref.ensure(std::path::PathBuf::from(&root));
                    let entry = FileEntry {
                        path: std::path::PathBuf::from(&path),
                        mtime_secs: 0,
                        content_hash: 0,
                        language,
                        symbols: parsed,
                    };
                    idx.upsert_file(entry);
                    Ok(added)
                },
            )?,
        )?;
    }

    {
        // ingest_lsp(root, lsp_response): merge symbols from a
        // workspace/symbol or documentSymbol response. Groups
        // results by path and replaces each path's entry.
        let ix = indexer.clone();
        m.set(
            "ingest_lsp",
            lua.create_function(move |_, (root, value): (String, Value)| {
                let json = lua_to_json(value)?;
                let inbound = ingest_lsp_symbols(&json);
                let mut ix_ref = ix.borrow_mut();
                let idx = ix_ref.ensure(std::path::PathBuf::from(&root));
                let mut by_path: std::collections::HashMap<
                    std::path::PathBuf,
                    (Option<String>, Vec<Symbol>),
                > = std::collections::HashMap::new();
                for entry in inbound {
                    let bucket = by_path
                        .entry(entry.path)
                        .or_insert_with(|| (entry.language.clone(), Vec::new()));
                    bucket.1.push(entry.symbol);
                }
                let merged = by_path.len();
                for (path, (lang, symbols)) in by_path {
                    idx.upsert_file(FileEntry {
                        path,
                        mtime_secs: 0,
                        content_hash: 0,
                        language: lang,
                        symbols,
                    });
                }
                Ok(merged)
            })?,
        )?;
    }

    {
        // invalidate(root, path): drop one file's entry.
        let ix = indexer.clone();
        m.set(
            "invalidate",
            lua.create_function(move |_, (root, path): (String, String)| {
                let mut ix_ref = ix.borrow_mut();
                Ok(ix_ref
                    .get_mut(std::path::Path::new(&root))
                    .is_some_and(|idx| idx.forget_file(std::path::Path::new(&path))))
            })?,
        )?;
    }

    {
        // is_fresh(root, path, mtime_secs, content_hash) -> bool
        let ix = indexer.clone();
        m.set(
            "is_fresh",
            lua.create_function(
                move |_, (root, path, mtime_secs, content_hash): (String, String, u64, u64)| {
                    let ix_ref = ix.borrow();
                    Ok(ix_ref.get(std::path::Path::new(&root)).is_some_and(|idx| {
                        idx.is_fresh(std::path::Path::new(&path), mtime_secs, content_hash)
                    }))
                },
            )?,
        )?;
    }

    {
        // search(root, query [, limit]) -> array of hit tables.
        let ix = indexer.clone();
        m.set(
            "search",
            lua.create_function(
                move |lua, (root, query, limit): (String, String, Option<usize>)| {
                    let ix_ref = ix.borrow();
                    let Some(idx) = ix_ref.get(std::path::Path::new(&root)) else {
                        return lua.create_table();
                    };
                    let hits = idx.search(&query, limit.unwrap_or(50));
                    let out = lua.create_table_with_capacity(hits.len(), 0)?;
                    for (i, h) in hits.iter().enumerate() {
                        out.set(i + 1, search_hit_to_lua(lua, h)?)?;
                    }
                    Ok(out)
                },
            )?,
        )?;
    }

    {
        // save(root [, path]): persist the index. Path defaults to
        // <root>/.pmacs/index.json.
        let ix = indexer.clone();
        m.set(
            "save",
            lua.create_function(move |_, (root, path): (String, Option<String>)| {
                let ix_ref = ix.borrow();
                let idx = ix_ref
                    .get(std::path::Path::new(&root))
                    .ok_or_else(|| mlua::Error::external(format!("unknown index root: {root}")))?;
                let dest = path.map_or_else(|| idx.default_cache_path(), std::path::PathBuf::from);
                idx.save(&dest).map_err(mlua::Error::external)?;
                Ok(dest.display().to_string())
            })?,
        )?;
    }

    {
        // load(root [, path]): replace the in-memory index for
        // `root` with the on-disk cache. A missing cache file
        // results in an empty index (cold-start).
        let ix = indexer.clone();
        m.set(
            "load",
            lua.create_function(move |_, (root, path): (String, Option<String>)| {
                let root_path = std::path::PathBuf::from(&root);
                let cache_path = path.map_or_else(
                    || crate::project_index::ProjectIndex::cache_path_for(&root_path),
                    std::path::PathBuf::from,
                );
                let idx = crate::project_index::ProjectIndex::load(root_path.clone(), &cache_path)
                    .map_err(mlua::Error::external)?;
                let symbol_count = idx.symbol_count();
                let file_count = idx.file_count();
                let mut ix_ref = ix.borrow_mut();
                let key = idx.root.clone();
                ix_ref.forget(&key);
                let slot = ix_ref.ensure(key);
                *slot = idx;
                Ok((file_count, symbol_count))
            })?,
        )?;
    }

    {
        // stats(root) -> { files, symbols, generation } or nil.
        let ix = indexer.clone();
        m.set(
            "stats",
            lua.create_function(move |lua, root: String| {
                let ix_ref = ix.borrow();
                let Some(idx) = ix_ref.get(std::path::Path::new(&root)) else {
                    return Ok(Value::Nil);
                };
                let t = lua.create_table_with_capacity(0, 4)?;
                t.set("files", idx.file_count())?;
                t.set("symbols", idx.symbol_count())?;
                t.set("generation", idx.generation)?;
                t.set("root", idx.root.display().to_string())?;
                Ok(Value::Table(t))
            })?,
        )?;
    }

    {
        // roots() -> array of registered index roots.
        let ix = indexer.clone();
        m.set(
            "roots",
            lua.create_function(move |lua, ()| {
                let ix_ref = ix.borrow();
                let mut roots: Vec<String> =
                    ix_ref.roots().map(|p| p.display().to_string()).collect();
                roots.sort();
                let out = lua.create_table_with_capacity(roots.len(), 0)?;
                for (i, r) in roots.iter().enumerate() {
                    out.set(i + 1, r.as_str())?;
                }
                Ok(out)
            })?,
        )?;
    }

    {
        // hash(text) -> u64. Exposes FNV-1a so Lua callers can
        // produce stable cache keys without a separate hash crate.
        m.set(
            "hash",
            lua.create_function(|_, text: String| Ok(fnv1a_64(text.as_bytes())))?,
        )?;
    }

    pmacs.set("index", m)?;
    Ok(())
}

/// Build a fresh [`ProjectIndexer`] and install `pmacs.index.*` over it.
pub fn make_project_indexer(lua: &Lua) -> mlua::Result<SharedProjectIndexer> {
    let ix: SharedProjectIndexer = Rc::new(RefCell::new(ProjectIndexer::new()));
    install_project_index(lua, &ix)?;
    Ok(ix)
}
