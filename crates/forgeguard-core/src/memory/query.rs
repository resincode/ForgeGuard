//! Retrieval over the persistent index.
//!
//! Every lookup is an indexed SQL query first and touches source only at the
//! level the caller asked for: metadata, structure, one snippet, and — last — a
//! whole file. A byte budget bounds what any single answer can return.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::{
    record_query,
    store::{Store, SymbolRow},
    MemoryStats, SymbolKind,
};
use crate::git;

/// How much context a retrieval is allowed to materialise.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Detail {
    /// Symbol, file, line range, kind, and fan-in/fan-out counts.
    #[default]
    Metadata,
    /// Adds signature, callers, callees, imports, dependents, and related tests.
    Structure,
    /// Adds the symbol's own source lines.
    Snippet,
    /// Adds the whole file. Only when there is a concrete reason.
    Full,
}

#[derive(Debug, Clone)]
pub struct RetrievalOptions {
    pub detail: Detail,
    /// Upper bound on returned source bytes. Source is dropped, never silently
    /// half-returned, when it would exceed the budget.
    pub max_bytes: usize,
}

impl Default for RetrievalOptions {
    fn default() -> Self {
        Self {
            detail: Detail::Metadata,
            max_bytes: 8 * 1024,
        }
    }
}

/// Relations returned per card before the list is capped.
const MAX_RELATIONS: usize = 20;
/// Entries listed per impact report section before the list is capped.
const MAX_IMPACT_ENTRIES: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolHit {
    pub qualified: String,
    pub name: String,
    pub path: PathBuf,
    pub kind: SymbolKind,
    pub language: String,
    pub start_line: usize,
    pub end_line: usize,
    pub lines: usize,
    pub exported: bool,
    pub is_test: bool,
    /// Fan-in: how many indexed symbols call this one.
    pub caller_count: usize,
    /// Fan-out: distinct callees inside it.
    pub callee_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SymbolRelation {
    pub qualified: String,
    pub path: PathBuf,
    pub start_line: usize,
}

impl SymbolRelation {
    fn from_row(row: &SymbolRow) -> Self {
        Self {
            qualified: row.qualified.clone(),
            path: row.path.clone(),
            start_line: row.start_line,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolCard {
    #[serde(flatten)]
    pub hit: SymbolHit,
    pub detail: Detail,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub calls: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extends: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub implementors: Vec<SymbolRelation>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub callers: Vec<SymbolRelation>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub callees: Vec<SymbolRelation>,
    /// Files this symbol's file imports (DEPENDS_ON).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
    /// Indexed files importing this symbol's file (USED_BY).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dependents: Vec<PathBuf>,
    /// Test symbols that reach this one (TESTS).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tests: Vec<SymbolRelation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    pub bytes_returned: usize,
    /// Source the file would have cost had it been read whole.
    pub bytes_avoided: usize,
    pub budget_exceeded: bool,
}

pub(crate) fn hit_from_row(store: &Store, row: &SymbolRow) -> Result<SymbolHit> {
    Ok(SymbolHit {
        qualified: row.qualified.clone(),
        name: row.name.clone(),
        path: row.path.clone(),
        kind: row.kind,
        language: row.language.clone(),
        start_line: row.start_line,
        end_line: row.end_line,
        lines: row.lines(),
        exported: row.exported,
        is_test: row.is_test,
        caller_count: store.caller_count(&row.name)?,
        callee_count: store.calls_of(row.id)?.len(),
        route_method: row.route_method.clone(),
        route_path: row.route_path.clone(),
    })
}

/// Level 1: cheap structural hits, no source read at all.
pub fn find_symbols(
    root: &Path,
    store: &Store,
    query: &str,
    limit: usize,
) -> Result<Vec<SymbolHit>> {
    let started = Instant::now();
    let rows = store.find_symbols(query, limit)?;
    let hits = rows
        .iter()
        .map(|row| hit_from_row(store, row))
        .collect::<Result<Vec<_>>>()?;
    record_query(root, started);
    Ok(hits)
}

/// Levels 1-4 for one symbol, expanding only as far as `options.detail` asks.
pub fn symbol_card(
    root: &Path,
    store: &Store,
    query: &str,
    options: &RetrievalOptions,
) -> Result<Option<SymbolCard>> {
    let started = Instant::now();
    let Some(row) = store.find_symbols(query, 1)?.into_iter().next() else {
        record_query(root, started);
        return Ok(None);
    };

    let mut card = SymbolCard {
        hit: hit_from_row(store, &row)?,
        detail: options.detail,
        signature: None,
        calls: Vec::new(),
        extends: Vec::new(),
        implementors: Vec::new(),
        callers: Vec::new(),
        callees: Vec::new(),
        dependencies: Vec::new(),
        dependents: Vec::new(),
        tests: Vec::new(),
        snippet: None,
        bytes_returned: 0,
        bytes_avoided: 0,
        budget_exceeded: false,
    };
    if options.detail != Detail::Metadata {
        fill_structure(store, &row, &mut card)?;
    }

    let absolute = root.join(&row.path);
    let file_bytes = fs::metadata(&absolute).map_or(0, |metadata| metadata.len() as usize);
    match options.detail {
        Detail::Snippet => apply_source(&mut card, &absolute, &row, options, file_bytes, false),
        Detail::Full => apply_source(&mut card, &absolute, &row, options, file_bytes, true),
        _ => card.bytes_avoided = file_bytes,
    }

    let mut stats = MemoryStats::load(root);
    stats.record_query(started.elapsed().as_millis() as u64);
    stats.bytes_returned = stats
        .bytes_returned
        .saturating_add(card.bytes_returned as u64);
    stats.bytes_avoided = stats
        .bytes_avoided
        .saturating_add(card.bytes_avoided as u64);
    if options.detail == Detail::Full {
        stats.full_file_reads = stats.full_file_reads.saturating_add(1);
    } else {
        stats.symbol_reads = stats.symbol_reads.saturating_add(1);
    }
    let _ = stats.save(root);
    Ok(Some(card))
}

fn fill_structure(store: &Store, row: &SymbolRow, card: &mut SymbolCard) -> Result<()> {
    card.signature = Some(row.signature.clone());
    card.calls = store
        .calls_of(row.id)?
        .into_iter()
        .map(|(name, receiver)| match receiver {
            Some(receiver) => format!("{receiver}.{name}"),
            None => name,
        })
        .collect();
    card.extends = store.extends_of(row.id)?;
    card.implementors = rows_to_relations(store.implementors_of(&row.name)?);
    let callers = store.callers_of(&row.name, row.container.as_deref())?;
    card.tests = callers
        .iter()
        .filter(|caller| caller.is_test)
        .map(SymbolRelation::from_row)
        .collect();
    card.callers = rows_to_relations(callers);
    card.callees = rows_to_relations(store.callees_of(row.id)?);
    card.dependencies = store.imports_of(&row.path)?;
    card.dependents = store.dependents_of(&row.path)?;
    // A hot symbol can have hundreds of callers. The exact counts stay on the
    // hit, so the caller can see it was capped.
    card.callers.truncate(MAX_RELATIONS);
    card.callees.truncate(MAX_RELATIONS);
    card.tests.truncate(MAX_RELATIONS);
    card.dependents.truncate(MAX_RELATIONS);
    card.implementors.truncate(MAX_RELATIONS);
    Ok(())
}

fn rows_to_relations(rows: Vec<SymbolRow>) -> Vec<SymbolRelation> {
    let mut relations = rows
        .iter()
        .map(SymbolRelation::from_row)
        .collect::<Vec<_>>();
    relations.sort();
    relations.dedup();
    relations
}

fn apply_source(
    card: &mut SymbolCard,
    absolute: &Path,
    row: &SymbolRow,
    options: &RetrievalOptions,
    file_bytes: usize,
    whole_file: bool,
) {
    // forgeguard: allow FG-SEC-007 -- the repository root joined with an indexed relative path; the store only ever records paths from the scanner's own walk
    let Ok(source) = fs::read_to_string(absolute) else {
        card.bytes_avoided = file_bytes;
        return;
    };
    let text = if whole_file {
        source
    } else {
        source
            .lines()
            .skip(row.start_line.saturating_sub(1))
            .take(row.lines())
            .collect::<Vec<_>>()
            .join("\n")
    };
    if text.len() > options.max_bytes {
        card.budget_exceeded = true;
        card.bytes_avoided = file_bytes;
        return;
    }
    card.bytes_returned = text.len();
    card.bytes_avoided = file_bytes.saturating_sub(text.len());
    card.snippet = Some(text);
}

/// How much attention a changed symbol deserves.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    #[default]
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedSymbol {
    #[serde(flatten)]
    pub symbol: SymbolRelation,
    pub kind: SymbolKind,
    pub risk: RiskLevel,
    pub caller_count: usize,
    pub test_count: usize,
    pub exported: bool,
    /// Why the risk landed where it did, in one clause per reason.
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    pub changed_files: Vec<PathBuf>,
    pub changed_symbols: Vec<ChangedSymbol>,
    pub callers: Vec<SymbolRelation>,
    pub dependent_files: Vec<PathBuf>,
    pub affected_tests: Vec<SymbolRelation>,
    /// Routes served by changed code: a changed public contract.
    pub affected_routes: Vec<SymbolRelation>,
    /// Files an agent has a structural reason to look at.
    pub blast_radius: usize,
    /// Highest risk among the changed symbols.
    pub risk: RiskLevel,
    /// Changed symbols with no test reaching them.
    pub untested_symbols: usize,
}

/// Change-aware analysis: Git diff to symbols to callers, dependents, and tests.
pub fn analyze_impact(root: &Path, store: &Store, base: Option<&str>) -> Result<ImpactReport> {
    let started = Instant::now();
    let scope = git::changed_scope(root, base)?;
    let mut report = ImpactReport {
        base: base.map(str::to_owned),
        changed_files: scope.paths.clone(),
        ..ImpactReport::default()
    };

    let mut callers = Vec::new();
    let mut tests = Vec::new();
    for path in &scope.paths {
        let ranges = scope.lines.get(path).map(Vec::as_slice);
        // forgeguard: allow FG-ALG-001 -- symbols of the changed files only, each visited once
        for row in store.symbols_touching(path, ranges)? {
            let reaching = store.callers_of(&row.name, row.container.as_deref())?;
            let (reaching_tests, reaching_callers): (Vec<_>, Vec<_>) =
                reaching.into_iter().partition(|caller| caller.is_test);
            if row.route_path.is_some() {
                report.affected_routes.push(SymbolRelation::from_row(&row));
            }
            report.changed_symbols.push(classify(
                &row,
                reaching_callers.len(),
                reaching_tests.len(),
            ));
            callers.extend(reaching_callers.iter().map(SymbolRelation::from_row));
            tests.extend(reaching_tests.iter().map(SymbolRelation::from_row));
        }
    }

    callers.sort();
    callers.dedup();
    tests.sort();
    tests.dedup();
    report.callers = callers;
    report.affected_tests = tests;
    report.affected_routes.sort();
    report.affected_routes.dedup();
    report.untested_symbols = report
        .changed_symbols
        .iter()
        .filter(|symbol| symbol.test_count == 0)
        .count();
    report.risk = report
        .changed_symbols
        .iter()
        .map(|symbol| symbol.risk)
        .max()
        .unwrap_or(RiskLevel::Low);

    let mut dependents = BTreeSet::new();
    for path in &scope.paths {
        dependents.extend(store.dependents_of(path)?);
    }
    for path in &scope.paths {
        dependents.remove(path);
    }
    report.dependent_files = dependents.into_iter().collect();

    let mut radius = report
        .changed_files
        .iter()
        .chain(report.dependent_files.iter())
        .collect::<BTreeSet<_>>();
    radius.extend(report.callers.iter().map(|caller| &caller.path));
    radius.extend(report.affected_tests.iter().map(|test| &test.path));
    report.blast_radius = radius.len();
    // The radius count above is exact; the listings are capped so one wide diff
    // cannot produce an unbounded payload.
    report.changed_symbols.truncate(MAX_IMPACT_ENTRIES);
    report.callers.truncate(MAX_IMPACT_ENTRIES);
    report.affected_tests.truncate(MAX_IMPACT_ENTRIES);
    report.dependent_files.truncate(MAX_IMPACT_ENTRIES);

    record_query(root, started);
    Ok(report)
}

/// Risk is structural, not statistical: reach, exposure, and test coverage.
fn classify(row: &SymbolRow, caller_count: usize, test_count: usize) -> ChangedSymbol {
    let mut score = 0_usize;
    let mut reasons = Vec::new();
    if caller_count >= 10 {
        score += 2;
        reasons.push(format!("{caller_count} callers"));
    } else if caller_count >= 3 {
        score += 1;
        reasons.push(format!("{caller_count} callers"));
    }
    if row.route_path.is_some() {
        score += 2;
        reasons.push("serves an HTTP route".to_owned());
    }
    if row.exported && !row.is_test {
        score += 1;
        reasons.push("exported from its module".to_owned());
    }
    if test_count == 0 && !row.is_test {
        score += 1;
        reasons.push("no indexed test reaches it".to_owned());
    }
    let risk = match score {
        0..=1 => RiskLevel::Low,
        2..=3 => RiskLevel::Medium,
        _ => RiskLevel::High,
    };
    ChangedSymbol {
        symbol: SymbolRelation::from_row(row),
        kind: row.kind,
        risk,
        caller_count,
        test_count,
        exported: row.exported,
        reasons,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layer {
    pub name: String,
    pub files: usize,
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteEntry {
    pub method: String,
    pub route: String,
    pub handler: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceEdge {
    pub from: String,
    pub from_path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Architecture {
    pub files: usize,
    pub symbols: usize,
    pub test_files: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// File counts per language.
    pub languages: BTreeMap<String, usize>,
    /// Top-level source directories and their weight.
    pub modules: Vec<Layer>,
    pub layers: Vec<Layer>,
    pub entry_points: Vec<SymbolRelation>,
    pub routes: Vec<RouteEntry>,
    /// Outbound HTTP calls, resolved to an indexed route when one matches.
    pub service_edges: Vec<ServiceEdge>,
    /// Imports that resolve to nothing indexed: third-party or platform.
    pub external_dependencies: Vec<String>,
}

const LAYER_KEYWORDS: &[(&str, &[&str])] = &[
    ("routes", &["route", "router", "endpoint"]),
    ("controllers", &["controller", "handler"]),
    ("services", &["service", "usecase", "domain"]),
    ("repositories", &["repository", "repo", "dao", "store"]),
    (
        "database",
        &["migration", "schema", "model", "entity", "query"],
    ),
    ("middleware", &["middleware", "interceptor", "guard"]),
    ("config", &["config", "settings", "env"]),
    ("cli", &["cli", "cmd", "main", "bin"]),
];

const ENTRY_POINT_NAMES: &[&str] = &["main", "handler", "lambda_handler", "run", "serve"];
const MAX_EXTERNAL_DEPENDENCIES: usize = 30;
const MAX_LAYER_PATHS: usize = 20;
const MAX_ROUTES: usize = 100;

/// One inexpensive request that answers "what is this repository" so an agent
/// does not rediscover it each session.
pub fn architecture(root: &Path, store: &Store) -> Result<Architecture> {
    let started = Instant::now();
    let (files, symbols) = store.counts()?;
    let mut architecture = Architecture {
        files,
        symbols,
        test_files: store.test_file_count()?,
        commit: store.meta("commit")?,
        languages: store.language_counts()?,
        ..Architecture::default()
    };

    let mut modules: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    let mut layers: BTreeMap<&str, Vec<PathBuf>> = BTreeMap::new();
    for (path, _) in store.file_paths_and_tests()? {
        if let Some(layer) = layer_of(&path) {
            layers.entry(layer).or_default().push(path.clone());
        }
        modules.entry(top_module(&path)).or_default().push(path);
    }
    architecture.modules = into_layers(modules.into_iter());
    architecture.layers = into_layers(
        layers
            .into_iter()
            .map(|(name, paths)| (name.to_owned(), paths)),
    );

    architecture.entry_points = store
        .all_symbols()?
        .iter()
        .filter(|row| is_entry_point(row))
        .map(SymbolRelation::from_row)
        .collect();
    architecture.entry_points.sort();
    architecture.entry_points.dedup();

    architecture.routes = store
        .routes()?
        .into_iter()
        .take(MAX_ROUTES)
        .filter_map(|row| {
            Some(RouteEntry {
                method: row.route_method.clone().unwrap_or_else(|| "ANY".to_owned()),
                route: row.route_path.clone()?,
                handler: row.qualified.clone(),
                path: row.path.clone(),
            })
        })
        .collect();
    architecture.service_edges = store
        .service_links()?
        .into_iter()
        .map(|link| ServiceEdge {
            from: link.symbol,
            from_path: link.path,
            method: link.method,
            url: link.url,
            target: link.target,
        })
        .collect();
    architecture.external_dependencies = external_dependencies(store)?;

    record_query(root, started);
    Ok(architecture)
}

/// An import whose tail matches no indexed file stem comes from outside the
/// repository. Resolution happens here rather than in SQL because the stem set
/// is small and the comparison is the same one the dependents query uses.
fn external_dependencies(store: &Store) -> Result<Vec<String>> {
    let internal = store.file_stems()?.into_iter().collect::<BTreeSet<_>>();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for (path, _) in store.file_paths_and_tests()? {
        // forgeguard: allow FG-ALG-001 -- one pass over every recorded import against a prebuilt stem set
        for import in store.imports_of(&path)? {
            if !internal.contains(&super::store::module_stem(&import)) {
                *counts.entry(import).or_default() += 1;
            }
        }
    }
    let mut ranked = counts.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
    Ok(ranked
        .into_iter()
        .take(MAX_EXTERNAL_DEPENDENCIES)
        .map(|(name, _)| name)
        .collect())
}

fn is_entry_point(row: &SymbolRow) -> bool {
    row.route_path.is_some()
        || (row.container.is_none()
            && ENTRY_POINT_NAMES.contains(&row.name.to_ascii_lowercase().as_str()))
}

fn into_layers(entries: impl Iterator<Item = (String, Vec<PathBuf>)>) -> Vec<Layer> {
    let mut layers = entries
        .map(|(name, paths)| Layer {
            name,
            files: paths.len(),
            paths: paths.into_iter().take(MAX_LAYER_PATHS).collect(),
        })
        .collect::<Vec<_>>();
    layers.sort_by(|left, right| {
        right
            .files
            .cmp(&left.files)
            .then(left.name.cmp(&right.name))
    });
    layers
}

fn top_module(path: &Path) -> String {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map_or_else(|| ".".to_owned(), |parent| parent.display().to_string())
}

/// Layer keywords match whole path words, not substrings: a repository named
/// `forgeguard` must not file every one of its files under `middleware` because
/// the name contains `guard`.
fn layer_of(path: &Path) -> Option<&'static str> {
    let words = path_words(path);
    LAYER_KEYWORDS
        .iter()
        .find(|(_, keywords)| {
            keywords.iter().any(|keyword| {
                // Directories are usually plural (`routes/`, `services/`) while the
                // keyword reads singular.
                words.contains(*keyword) || words.contains(&format!("{keyword}s"))
            })
        })
        .map(|(name, _)| *name)
}

fn path_words(path: &Path) -> BTreeSet<String> {
    path.to_string_lossy()
        .to_ascii_lowercase()
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}
