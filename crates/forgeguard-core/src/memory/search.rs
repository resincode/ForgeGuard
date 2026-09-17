//! Ranked symbol search.
//!
//! `store.find_symbols` answers "is there a symbol called exactly this"; it has
//! no notion of "closest". This module adds that: a plain BM25 ranking over the
//! indexed symbols so a fuzzy phrase like `validate token` finds
//! `AuthService.validateToken` without an embedding model, a network call, or a
//! new dependency.
//!
//! The document for a symbol is its qualified name, its signature, and its
//! repository-relative path. Tokenisation splits on non-alphanumeric characters
//! *and* on camelCase humps, and additionally keeps the un-split chunk, so
//! `validateToken` is reachable from `validate token`, `token`, and
//! `validatetoken`.

use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::{store::SymbolRow, Store, SymbolKind};

/// Standard BM25 constants. `k1` bounds how much a repeated term can keep
/// adding; `b` is how hard a long document is penalised.
const K1: f64 = 1.2;
const B: f64 = 0.75;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    pub score: f64,
    pub qualified: String,
    pub name: String,
    pub path: PathBuf,
    pub kind: SymbolKind,
    pub language: String,
    pub start_line: usize,
    pub end_line: usize,
    pub is_test: bool,
    pub exported: bool,
}

/// Rank every indexed symbol against `query` and return the best `limit`.
///
/// An empty query, an empty index, or `limit == 0` yields no hits rather than an
/// error: "nothing matched" is a result, not a failure.
pub fn search_symbols(
    root: &Path,
    store: &Store,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    let started = Instant::now();
    let hits = ranked(store, query, limit);
    // Counters are telemetry and must reflect the query even when it found
    // nothing, so this runs before the result is propagated.
    super::record_query(root, started);
    hits
}

fn ranked(store: &Store, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
    let terms = distinct(&tokenize(query));
    if terms.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }

    // ponytail: corpus statistics (document frequency, average length) are
    // recomputed from the store on every call. That is one full symbol scan per
    // search, which is fine at repository scale; the upgrade path when it stops
    // being fine is a precomputed term table written during indexing, not a
    // heavier algorithm here.
    let rows = store.all_symbols()?;
    let documents = rows.iter().map(document).collect::<Vec<_>>();
    if documents.is_empty() {
        return Ok(Vec::new());
    }
    let total = documents.len();
    let average = documents.iter().map(|entry| entry.length).sum::<usize>() as f64 / total as f64;
    let idf = inverse_frequencies(&documents, &terms, total);

    let mut hits = rows
        .iter()
        .zip(&documents)
        .map(|(row, entry)| hit(row, score(entry, &terms, &idf, average)))
        .filter(|candidate| candidate.score > 0.0)
        .collect::<Vec<_>>();
    hits.sort_by(compare);
    hits.truncate(limit);
    Ok(hits)
}

struct Document {
    frequencies: HashMap<String, u32>,
    length: usize,
}

fn document(row: &SymbolRow) -> Document {
    let text = format!(
        "{} {} {}",
        row.qualified,
        row.signature,
        row.path.to_string_lossy()
    );
    let tokens = tokenize(&text);
    let mut frequencies: HashMap<String, u32> = HashMap::new();
    for token in &tokens {
        *frequencies.entry(token.clone()).or_default() += 1;
    }
    Document {
        frequencies,
        length: tokens.len(),
    }
}

/// `ln(1 + (N - df + 0.5) / (df + 0.5))`: the BM25 idf variant that stays
/// positive, so a term present in every document contributes ~0 instead of
/// pushing scores negative.
fn inverse_frequencies(
    documents: &[Document],
    terms: &[String],
    total: usize,
) -> HashMap<String, f64> {
    let mut frequencies: HashMap<String, f64> = HashMap::new();
    for term in terms {
        // forgeguard: allow FG-ALG-001 -- one pass per query term over the symbol
        // set; query terms are a handful, so this is linear in practice
        let seen = documents
            .iter()
            .filter(|entry| entry.frequencies.contains_key(term))
            .count() as f64;
        let value = ((total as f64 - seen + 0.5) / (seen + 0.5) + 1.0).ln();
        frequencies.insert(term.clone(), value);
    }
    frequencies
}

fn score(entry: &Document, terms: &[String], idf: &HashMap<String, f64>, average: f64) -> f64 {
    let normalised = if average > 0.0 {
        entry.length as f64 / average
    } else {
        1.0
    };
    terms
        .iter()
        .map(|term| term_score(entry, term, idf, normalised))
        .sum()
}

fn term_score(entry: &Document, term: &str, idf: &HashMap<String, f64>, normalised: f64) -> f64 {
    let Some(frequency) = entry.frequencies.get(term) else {
        return 0.0;
    };
    let frequency = f64::from(*frequency);
    let weight = idf.get(term).copied().unwrap_or_default();
    weight * (frequency * (K1 + 1.0)) / (frequency + K1 * (1.0 - B + B * normalised))
}

/// Highest score first, then repository path, then line: two runs over the same
/// index always produce the same order.
fn compare(left: &SearchHit, right: &SearchHit) -> Ordering {
    right
        .score
        .partial_cmp(&left.score)
        .unwrap_or(Ordering::Equal)
        .then_with(|| left.path.cmp(&right.path))
        .then_with(|| left.start_line.cmp(&right.start_line))
}

fn hit(row: &SymbolRow, score: f64) -> SearchHit {
    SearchHit {
        score,
        qualified: row.qualified.clone(),
        name: row.name.clone(),
        path: row.path.clone(),
        kind: row.kind,
        language: row.language.clone(),
        start_line: row.start_line,
        end_line: row.end_line,
        is_test: row.is_test,
        exported: row.exported,
    }
}

fn distinct(tokens: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    tokens
        .iter()
        .filter(|token| seen.insert((*token).clone()))
        .cloned()
        .collect()
}

/// Lowercased tokens: every alphanumeric run, split again at camelCase humps.
/// The un-split run is kept too, which is what lets a query spelled
/// `validatetoken` reach `validateToken`.
fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    for chunk in text.split(|character: char| !character.is_alphanumeric()) {
        if chunk.is_empty() {
            continue;
        }
        let parts = split_case(chunk);
        if parts.len() > 1 {
            tokens.extend(parts);
        }
        tokens.push(chunk.to_lowercase());
    }
    tokens
}

fn split_case(chunk: &str) -> Vec<String> {
    let characters = chunk.chars().collect::<Vec<_>>();
    let mut parts = Vec::new();
    let mut start = 0;
    for index in 1..characters.len() {
        if is_boundary(&characters, index) {
            parts.push(lowercase(&characters[start..index]));
            start = index;
        }
    }
    parts.push(lowercase(&characters[start..]));
    parts.retain(|part| !part.is_empty());
    parts
}

/// A hump starts where a non-uppercase character is followed by an uppercase
/// one, or where an uppercase run gives way to a word (`HTTPServer`).
fn is_boundary(characters: &[char], index: usize) -> bool {
    let current = characters[index];
    if !current.is_uppercase() {
        return false;
    }
    if !characters[index - 1].is_uppercase() {
        return true;
    }
    characters
        .get(index + 1)
        .is_some_and(|next| next.is_lowercase())
}

fn lowercase(characters: &[char]) -> String {
    characters.iter().collect::<String>().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{FileEntry, Symbol};

    fn symbol(name: &str, container: Option<&str>, signature: &str, line: usize) -> Symbol {
        Symbol {
            name: name.to_owned(),
            container: container.map(str::to_owned),
            kind: SymbolKind::Function,
            start_line: line,
            end_line: line + 4,
            signature: signature.to_owned(),
            exported: true,
            calls: Vec::new(),
            extends: Vec::new(),
            route: None,
            links: Vec::new(),
        }
    }

    fn entry(symbols: Vec<Symbol>) -> FileEntry {
        FileEntry {
            language: "rust".to_owned(),
            content_hash: "0".to_owned(),
            symbols_hash: "0".to_owned(),
            mtime: 0,
            size: 0,
            is_test: false,
            last_indexed: 0,
            imports: Vec::new(),
            exports: Vec::new(),
            symbols,
        }
    }

    fn fixture() -> Store {
        let mut store = Store::open_memory().expect("open store");
        store
            .upsert_file(
                Path::new("src/auth.rs"),
                &entry(vec![
                    symbol(
                        "validateToken",
                        Some("AuthService"),
                        "fn validateToken(token: &str) -> bool",
                        10,
                    ),
                    symbol("refresh", Some("AuthService"), "fn refresh()", 40),
                ]),
            )
            .expect("upsert auth");
        store
            .upsert_file(
                Path::new("src/render.rs"),
                &entry(vec![symbol(
                    "render_widget",
                    None,
                    "fn render_widget(widget: &Widget)",
                    5,
                )]),
            )
            .expect("upsert render");
        store
    }

    fn names(hits: &[SearchHit]) -> Vec<String> {
        hits.iter().map(|hit| hit.name.clone()).collect()
    }

    fn search(store: &Store, query: &str, limit: usize) -> Vec<SearchHit> {
        let root = tempfile::tempdir().expect("tempdir");
        search_symbols(root.path(), store, query, limit).expect("search")
    }

    #[test]
    fn splits_camel_case_and_keeps_the_whole_run() {
        assert_eq!(
            tokenize("validateToken"),
            vec!["validate", "token", "validatetoken"]
        );
        assert_eq!(tokenize("token"), vec!["token"]);
        assert_eq!(
            tokenize("HTTPServerPool"),
            vec!["http", "server", "pool", "httpserverpool"]
        );
        assert_eq!(tokenize("render_widget"), vec!["render", "widget"]);
    }

    #[test]
    fn phrase_query_finds_the_camel_case_symbol() {
        let store = fixture();
        assert_eq!(
            names(&search(&store, "validate token", 5))[0],
            "validateToken"
        );
        assert_eq!(names(&search(&store, "token", 5))[0], "validateToken");
        assert_eq!(
            names(&search(&store, "validatetoken", 5))[0],
            "validateToken"
        );
    }

    #[test]
    fn ranks_the_better_match_first() {
        let store = fixture();
        let hits = search(&store, "widget render", 5);
        assert_eq!(names(&hits), vec!["render_widget"]);
        assert!(hits[0].score > 0.0);
    }

    #[test]
    fn rare_term_outranks_common_term() {
        let store = fixture();
        let hits = search(&store, "auth token", 5);
        // Both AuthService symbols match "auth"; only one matches "token".
        assert_eq!(names(&hits), vec!["validateToken", "refresh"]);
        assert!(hits[0].score > hits[1].score);
    }

    #[test]
    fn empty_query_returns_nothing() {
        let store = fixture();
        assert!(search(&store, "", 5).is_empty());
        assert!(search(&store, "   ***   ", 5).is_empty());
    }

    #[test]
    fn unknown_term_returns_nothing() {
        let store = fixture();
        assert!(search(&store, "kubernetes", 5).is_empty());
    }

    #[test]
    fn limit_caps_results() {
        let store = fixture();
        assert_eq!(search(&store, "auth", 1).len(), 1);
        assert!(search(&store, "auth", 0).is_empty());
    }

    #[test]
    fn ordering_is_stable_across_runs() {
        let store = fixture();
        let first = names(&search(&store, "src rs", 10));
        let second = names(&search(&store, "src rs", 10));
        assert_eq!(first, second);
        assert!(!first.is_empty());
    }

    #[test]
    fn empty_index_yields_no_hits() {
        let store = Store::open_memory().expect("open store");
        assert!(search(&store, "anything", 5).is_empty());
    }

    #[test]
    fn counters_record_the_query() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = fixture();
        search_symbols(root.path(), &store, "token", 5).expect("search");
        let stats = crate::memory::MemoryStats::load(root.path());
        assert_eq!(stats.graph_queries, 1);
    }
}
