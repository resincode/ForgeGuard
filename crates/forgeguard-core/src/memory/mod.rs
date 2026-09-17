//! Persistent codebase memory: a structural index of the repository that lets an
//! agent answer "where is this symbol, who calls it, what breaks if it changes"
//! without re-reading source files.
//!
//! The index is derived from the same tree-sitter parsers the scanner uses and
//! stored in one SQLite database under `.forgeguard/cache/memory`. It is
//! refreshed incrementally: a file is reparsed only when its size, mtime, or
//! content hash moved. Source text is never copied into the index; snippets are
//! read back from the working tree by line range, so the index cannot serve
//! stale code.

pub mod artifact;
mod cypher;
mod extract;
pub mod lsp;
mod projects;
mod query;
mod search;
pub mod store;
mod trace;
mod watch;

use std::{
    fs,
    hash::Hasher,
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{config::ScanConfig, git, scanner};

pub use artifact::{ArtifactReport, ARTIFACT_FILE, BEST_LEVEL, FAST_LEVEL};
pub use cypher::{run_query, QueryResult};
pub use lsp::{available_servers, LspOptions, LspStats};
pub use projects::{
    delete_project, index_status, list_projects, register_project, IndexStatus, ProjectEntry,
};
pub use query::{
    analyze_impact, architecture, find_symbols, symbol_card, Architecture, Detail, ImpactReport,
    Layer, RetrievalOptions, RiskLevel, SymbolCard, SymbolHit, SymbolRelation,
};
pub use search::{search_symbols, SearchHit};
pub use store::{Store, DATABASE_FILE, SCHEMA_VERSION};
pub use trace::{trace_path, Direction, TraceNode, TraceReport};
pub use watch::{watch_repository, WatchOptions, WatchTick};

pub const MEMORY_DIR: &str = ".forgeguard/cache/memory";
pub const STATS_FILE: &str = ".forgeguard/cache/memory/stats.json";
/// Written by the JSON-backed prototype; removed on sight.
const LEGACY_INDEX_FILE: &str = ".forgeguard/cache/memory/index.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SymbolKind {
    Function,
    Method,
    Type,
    Module,
    Route,
}

impl SymbolKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Method => "method",
            Self::Type => "type",
            Self::Module => "module",
            Self::Route => "route",
        }
    }

    pub fn from_label(value: &str) -> Self {
        match value {
            "method" => Self::Method,
            "type" => Self::Type,
            "module" => Self::Module,
            "route" => Self::Route,
            _ => Self::Function,
        }
    }
}

/// A call site: the callee name and, when static resolution succeeded, the type
/// that owns it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver: Option<String>,
    /// Expression the call was invoked on, when there was one. Its presence is
    /// what marks a call as worth asking a language server about: `foo()` has no
    /// owner to resolve, `repo.foo()` does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualifier: Option<String>,
    /// Position of the qualifier, or of the callee when there is none: 1-based
    /// line, 0-based character, which is what the LSP position encoding wants.
    #[serde(default)]
    pub line: usize,
    #[serde(default)]
    pub character: usize,
}

/// An HTTP route a symbol serves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    pub method: String,
    pub path: String,
}

/// An outbound HTTP call, which may resolve to a route in another service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceCall {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    /// Enclosing class, struct, trait, or impl block, when the grammar exposes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
    pub kind: SymbolKind,
    pub start_line: usize,
    pub end_line: usize,
    /// First line of the declaration, which is the part an agent usually needs.
    pub signature: String,
    pub exported: bool,
    /// Callees invoked inside the symbol body (CALLS).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub calls: Vec<Call>,
    /// Supertypes and implemented interfaces (EXTENDS / IMPLEMENTS).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extends: Vec<String>,
    /// HTTP route served by this symbol (ROUTE).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<Route>,
    /// Outbound HTTP calls made by this symbol (CALLS_SERVICE).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<ServiceCall>,
}

impl Symbol {
    pub fn qualified(&self) -> String {
        match &self.container {
            Some(container) => format!("{container}.{}", self.name),
            None => self.name.clone(),
        }
    }

    pub fn lines(&self) -> usize {
        self.end_line.saturating_sub(self.start_line) + 1
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub language: String,
    pub content_hash: String,
    pub symbols_hash: String,
    pub mtime: u64,
    pub size: u64,
    pub is_test: bool,
    pub last_indexed: u64,
    /// Module paths this file imports (IMPORTS).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imports: Vec<String>,
    /// Names this file exports (EXPORTS).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exports: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub symbols: Vec<Symbol>,
}

/// Counters an agent can read to see what the memory layer saved it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryStats {
    pub indexed_files: u64,
    pub reused_files: u64,
    pub graph_queries: u64,
    pub symbol_reads: u64,
    pub full_file_reads: u64,
    pub bytes_returned: u64,
    pub bytes_avoided: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub index_millis: u64,
    pub query_millis: u64,
}

impl MemoryStats {
    /// Four bytes per token is the usual rule of thumb for source text; it is an
    /// estimate and is reported as one.
    pub fn estimated_tokens_returned(&self) -> u64 {
        self.bytes_returned / 4
    }

    pub fn estimated_tokens_avoided(&self) -> u64 {
        self.bytes_avoided / 4
    }

    pub fn load(root: &Path) -> Self {
        // forgeguard: allow FG-SEC-007 -- the caller's repository root joined with a crate constant
        fs::read_to_string(root.join(STATS_FILE))
            .ok()
            .and_then(|source| serde_json::from_str(&source).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let path = root.join(STATS_FILE);
        let directory = path.parent().context("memory stats path has no parent")?;
        fs::create_dir_all(directory)
            .with_context(|| format!("failed to create {}", directory.display()))?;
        fs::write(&path, serde_json::to_vec(self)?)
            .with_context(|| format!("failed to write {}", path.display()))
    }

    pub fn record_query(&mut self, elapsed_millis: u64) {
        self.graph_queries = self.graph_queries.saturating_add(1);
        self.query_millis = self.query_millis.saturating_add(elapsed_millis);
    }
}

/// Record one query against the repository's counters. Failures to persist
/// counters never fail a query: they are telemetry, not results.
pub(crate) fn record_query(root: &Path, started: Instant) {
    let mut stats = MemoryStats::load(root);
    stats.record_query(started.elapsed().as_millis() as u64);
    let _ = stats.save(root);
}

#[derive(Debug, Clone, Default)]
pub struct IndexOptions {
    /// Reparse every file even when its hash is unchanged.
    pub force: bool,
    /// Limit indexing to these repository-relative paths.
    pub paths: Option<Vec<PathBuf>>,
    /// Ask installed language servers about call sites the static pass could not
    /// attribute. Off by default: it needs external tools and costs seconds.
    pub lsp: Option<LspOptions>,
}

/// Call sites one LSP pass will look at. Past this the pass stops being a
/// refinement and starts being the whole index run.
pub const MAX_LSP_CALLS: usize = 2000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexReport {
    pub files: usize,
    pub symbols: usize,
    pub parsed: usize,
    pub reused: usize,
    pub removed: usize,
    pub renamed: usize,
    pub duration_millis: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// Present when the run asked language servers about unresolved receivers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lsp: Option<LspReport>,
}

/// What one language-server pass cost and returned.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LspReport {
    /// Call sites the static pass left without an owning type.
    pub candidates: usize,
    pub resolved: usize,
    pub requests: u64,
    pub timeouts: u64,
    pub servers_started: u64,
    pub families_skipped: u64,
    pub budget_skips: u64,
    pub duration_millis: u64,
    /// Servers found on PATH, by language family.
    pub servers: Vec<String>,
}

/// Open a graph that matches the working tree: an empty index is built, an
/// existing one is refreshed from the Git diff. A refresh failure (no Git, for
/// example) leaves the existing index in use, because a stale answer beats no
/// answer. Callers that must not answer from an empty graph check
/// [`Store::is_empty`] on the result.
pub fn ensure_current(root: &Path, config: &ScanConfig) -> Result<Store> {
    let store = Store::open(root)?;
    if store.is_empty()? {
        index_repository(root, config, &IndexOptions::default())?;
    } else {
        let _ = refresh_changed(root, config, None);
    }
    Store::open(root)
}

/// Build or refresh the index. Files whose size, mtime, and content hash are
/// unchanged are carried over untouched; everything else is reparsed.
pub fn index_repository(
    root: &Path,
    config: &ScanConfig,
    options: &IndexOptions,
) -> Result<IndexReport> {
    let started = Instant::now();
    let _ = fs::remove_file(root.join(LEGACY_INDEX_FILE));
    let mut store = Store::open(root)?;
    if options.force {
        store.reset()?;
    } else if store.is_empty()? {
        // Nothing indexed yet: a committed artifact saves the first full parse.
        let _ = import_artifact(root, &mut store);
    }

    let scan_options = scanner::ScanOptions {
        paths: options.paths.clone(),
    };
    // The memory layer indexes tests too: "which tests cover this symbol" is one
    // of the questions it exists to answer.
    let config = ScanConfig {
        include_tests: true,
        ..config.clone()
    };
    let files = scanner::collect_source_files(root, &config, &scan_options)?;

    let mut report = IndexReport::default();
    let mut present = Vec::new();
    for path in &files {
        let relative = path.strip_prefix(root).unwrap_or(path).to_path_buf();
        match index_file(path, &relative, &mut store, &config, options.force)? {
            Some(FileOutcome::Parsed { renamed }) => {
                report.parsed += 1;
                report.renamed += usize::from(renamed);
                present.push(relative);
            }
            Some(FileOutcome::Reused) => {
                report.reused += 1;
                present.push(relative);
            }
            None => {}
        }
    }

    if options.paths.is_none() {
        // A full run owns the whole graph; a scoped run must not prune the rest.
        let known = store.all_paths()?;
        for path in known {
            if !present.contains(&path) && store.remove_file(&path)? {
                report.removed += 1;
            }
        }
    }

    let commit = git::head_commit(root).unwrap_or(None);
    if let Some(commit) = &commit {
        store.set_meta("commit", commit)?;
    }
    store.set_meta("last_indexed", &now_seconds().to_string())?;

    let (files_total, symbols_total) = store.counts()?;
    report.files = files_total;
    report.symbols = symbols_total;
    report.duration_millis = elapsed_millis(started);
    report.commit = commit;

    if let Some(options) = options.lsp.clone() {
        report.lsp = Some(resolve_with_servers(root, &store, options)?);
    }

    let mut stats = MemoryStats::load(root);
    stats.indexed_files = stats.indexed_files.saturating_add(report.parsed as u64);
    stats.reused_files = stats.reused_files.saturating_add(report.reused as u64);
    stats.cache_hits = stats.cache_hits.saturating_add(report.reused as u64);
    stats.cache_misses = stats.cache_misses.saturating_add(report.parsed as u64);
    stats.index_millis = stats.index_millis.saturating_add(report.duration_millis);
    stats.save(root)?;
    projects::register_project(root, report.files, report.symbols)?;

    Ok(report)
}

/// Re-index only what Git reports as changed. Deleted paths leave the graph.
pub fn refresh_changed(
    root: &Path,
    config: &ScanConfig,
    base: Option<&str>,
) -> Result<IndexReport> {
    let scope = git::changed_scope(root, base)?;
    let mut removed = 0;
    {
        let store = Store::open(root)?;
        let (all, _existing) = git::changed_paths_partitioned(root)?;
        for path in all {
            if !root.join(&path).is_file() && store.remove_file(&path)? {
                removed += 1;
            }
        }
    }
    let mut report = index_repository(
        root,
        config,
        &IndexOptions {
            force: false,
            paths: Some(scope.paths),
            ..IndexOptions::default()
        },
    )?;
    report.removed = removed;
    Ok(report)
}

/// Write the portable artifact a team commits next to its source.
pub fn export_artifact(
    root: &Path,
    destination: Option<&Path>,
    level: i32,
) -> Result<artifact::ArtifactReport> {
    let store = Store::open(root)?;
    let path = destination.map_or_else(|| root.join(ARTIFACT_FILE), Path::to_path_buf);
    artifact::export(&store, &path, level)
}

/// Seed an empty cache from a committed artifact. Incremental indexing then
/// fills in whatever the local checkout changed.
pub fn import_artifact(root: &Path, store: &mut Store) -> Result<bool> {
    let Some(path) = artifact::resolve(root) else {
        return Ok(false);
    };
    let source = artifact::open_artifact(&path)?;
    if source.is_empty()? {
        return Ok(false);
    }
    store.reset()?;
    for (path, _) in source.file_paths_and_tests()? {
        if let Some(entry) = source.file_entry(&path)? {
            store.upsert_file(&path, &entry)?;
        }
    }
    Ok(true)
}

/// Hybrid resolution: the static pass runs first and owns everything it can
/// answer; a language server is asked only about what it left open, so a run
/// without any server installed costs one directory scan and nothing else.
fn resolve_with_servers(root: &Path, store: &Store, options: LspOptions) -> Result<LspReport> {
    let started = Instant::now();
    let servers = lsp::available_servers()
        .iter()
        .map(|server| server.family.to_owned())
        .collect::<Vec<_>>();
    let candidates = store.unresolved_calls(MAX_LSP_CALLS)?;
    let mut report = LspReport {
        candidates: candidates.len(),
        servers,
        ..LspReport::default()
    };
    if report.servers.is_empty() || candidates.is_empty() {
        report.duration_millis = elapsed_millis(started);
        return Ok(report);
    }

    let mut resolver = lsp::LspResolver::new(root, options);
    for call in candidates {
        let absolute = root.join(&call.path);
        if let Some(receiver) =
            resolver.receiver_at(&absolute, &call.language, call.line, call.character)
        {
            store.set_call_receiver(call.id, &receiver)?;
            report.resolved += 1;
        }
    }
    resolver.shutdown();

    let stats = resolver.stats();
    report.requests = stats.requests;
    report.timeouts = stats.timeouts;
    report.servers_started = stats.servers_started;
    report.families_skipped = stats.families_skipped;
    report.budget_skips = stats.budget_skips;
    report.duration_millis = elapsed_millis(started);
    Ok(report)
}

enum FileOutcome {
    Parsed { renamed: bool },
    Reused,
}

fn index_file(
    path: &Path,
    relative: &Path,
    store: &mut Store,
    config: &ScanConfig,
    force: bool,
) -> Result<Option<FileOutcome>> {
    let Some(profile) = scanner::LanguageProfile::from_path(path) else {
        return Ok(None);
    };
    let Ok(metadata) = fs::metadata(path) else {
        return Ok(None);
    };
    if metadata.len() > config.max_file_bytes {
        return Ok(None);
    }
    let mtime = modified_nanos(&metadata);

    // Cheapest possible reuse check: no read at all when the stat data matches.
    if !force {
        if let Some(stat) = store.file_stat(relative)? {
            if stat.mtime == mtime && stat.size == metadata.len() {
                return Ok(Some(FileOutcome::Reused));
            }
        }
    }

    // forgeguard: allow FG-SEC-007 -- the path comes from the scanner's own walk of the repository
    let Ok(source) = fs::read_to_string(path) else {
        return Ok(None);
    };
    let content_hash = hash(source.as_bytes());
    if !force && store.content_hash(relative)?.as_deref() == Some(content_hash.as_str()) {
        // Touched but not edited: keep the symbols, refresh the stat data.
        store.touch_file(relative, mtime, metadata.len())?;
        return Ok(Some(FileOutcome::Reused));
    }

    // Same content under a path the graph knows by another name: a rename.
    let renamed = store
        .paths_with_hash(&content_hash)?
        .iter()
        .any(|known| known != relative && !path.with_file_name(known).exists());

    let facts = extract::extract(profile, &source);
    let entry = FileEntry {
        language: profile.family().to_owned(),
        symbols_hash: symbols_hash(&facts.symbols),
        content_hash,
        mtime,
        size: metadata.len(),
        is_test: scanner::is_test_path(path),
        last_indexed: now_seconds(),
        imports: facts.imports,
        exports: facts.exports,
        symbols: facts.symbols,
    };
    store.upsert_file(relative, &entry)?;
    Ok(Some(FileOutcome::Parsed { renamed }))
}

pub(crate) fn hash(bytes: &[u8]) -> String {
    // Same hasher family the changed-file fingerprint already uses. A hasher
    // change across toolchains only looks like "everything changed", which costs
    // one re-index and never serves stale symbols.
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hasher.write(bytes);
    format!("{:016x}", hasher.finish())
}

fn symbols_hash(symbols: &[Symbol]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for symbol in symbols {
        hasher.write(symbol.qualified().as_bytes());
        hasher.write(symbol.signature.as_bytes());
        hasher.write(symbol.kind.as_str().as_bytes());
    }
    format!("{:016x}", hasher.finish())
}

fn modified_nanos(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos() as u64)
}

pub(crate) fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn elapsed_millis(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}
