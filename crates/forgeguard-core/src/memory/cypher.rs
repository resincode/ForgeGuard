//! A deliberately tiny, read-only Cypher-like query language over the code
//! graph.
//!
//! The caller may be an untrusted agent, so this is a trust boundary. Two rules
//! make it safe rather than clever:
//!
//! 1. Only `MATCH ... RETURN` parses. Anything that looks like a mutation, a
//!    second statement, or a smuggled SQL keyword is refused before execution.
//! 2. No SQL is ever assembled here. A parsed query is translated into calls to
//!    the typed [`Store`] API, whose statements are fixed strings with bound
//!    parameters, so a user value cannot reach the SQL text at all. (The store's
//!    raw `select` helper takes no parameter list, so using it would have meant
//!    string concatenation; that is exactly what this must not do.)
//!
//! Supported grammar:
//!
//! ```text
//! MATCH (a:Label) [WHERE ...] RETURN a.prop[, ...] [LIMIT n]
//! MATCH (a:Label)-[:REL]->(b:Label) [WHERE ...] RETURN a.prop, b.prop [LIMIT n]
//! ```

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use super::{
    store::{module_stem, ServiceLink, SymbolRow},
    Store, SymbolKind,
};

/// Ceiling on how many candidate rows one label scan pulls from the store
/// before Rust-side filtering. A repository with more matching symbols than this
/// is not a query an agent can read anyway.
const SCAN_LIMIT: usize = 100_000;

const LABELS: &str = "Function, Method, Type, Module, Route, File";
const RELATIONSHIPS: &str =
    "CALLS, CALLED_BY, EXTENDS, IMPLEMENTED_BY, CONTAINS, IMPORTS, DEPENDS_ON, ROUTES_TO";
const PROPERTIES: &str =
    "name, qualified, kind, path, line, signature, exported, is_test, route_method, route_path";

/// SQL and Cypher verbs that have no business in a read-only query, checked as
/// whole words anywhere in the text — including inside string literals, so a
/// value cannot be used to carry one through.
const FORBIDDEN: &[&str] = &[
    "create", "delete", "detach", "set", "merge", "drop", "remove", "attach", "pragma", "insert",
    "update", "alter", "vacuum", "union", "select", "exec", "execute", "load", "foreach", "begin",
    "commit", "rollback", "replace", "truncate", "grant", "table", "into", "values",
];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub truncated: bool,
}

/// Run one read-only graph query. `limit` is the hard cap on returned rows; a
/// `LIMIT` clause in the query may lower it but never raise it.
pub fn run_query(root: &Path, store: &Store, query: &str, limit: usize) -> Result<QueryResult> {
    let started = Instant::now();
    let result = execute(store, query, limit);
    super::record_query(root, started);
    result
}

fn execute(store: &Store, query: &str, limit: usize) -> Result<QueryResult> {
    reject_writes(query)?;
    let parsed = parse(query)?;
    let cap = parsed.limit.map_or(limit, |value| value.min(limit));
    let columns = parsed
        .returns
        .iter()
        .map(|(alias, property)| format!("{alias}.{property}"))
        .collect();
    let mut rows = match &parsed.pattern {
        Pattern::Node { alias, label } => node_rows(store, &parsed, alias, *label, cap)?,
        Pattern::Edge {
            left,
            relation,
            right,
        } => edge_rows(store, &parsed, left, *relation, right, cap)?,
    };
    let truncated = rows.len() > cap;
    rows.truncate(cap);
    Ok(QueryResult {
        columns,
        rows,
        truncated,
    })
}

// ---------------------------------------------------------------------------
// Rejection
// ---------------------------------------------------------------------------

fn reject_writes(query: &str) -> Result<()> {
    if query.contains(';') {
        bail!("statement chaining with `;` is not allowed; {SUPPORTED}");
    }
    if query.contains("--") || query.contains("/*") {
        bail!("comments are not allowed in a query; {SUPPORTED}");
    }
    if let Some(word) = forbidden_word(query) {
        bail!("`{word}` is not allowed: this engine is read-only; {SUPPORTED}");
    }
    Ok(())
}

const SUPPORTED: &str = "the only supported shapes are \
`MATCH (a:Label) [WHERE ...] RETURN a.prop[, ...] [LIMIT n]` and \
`MATCH (a:Label)-[:REL]->(b:Label) [WHERE ...] RETURN a.prop, b.prop [LIMIT n]`";

fn forbidden_word(query: &str) -> Option<String> {
    query
        .split(|character: char| !is_word_char(character))
        .map(str::to_ascii_lowercase)
        .find(|word| FORBIDDEN.contains(&word.as_str()))
}

fn is_word_char(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Label {
    Function,
    Method,
    Type,
    Module,
    Route,
    File,
}

impl Label {
    fn parse(text: &str) -> Result<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "function" => Ok(Self::Function),
            "method" => Ok(Self::Method),
            "type" => Ok(Self::Type),
            "module" => Ok(Self::Module),
            "route" => Ok(Self::Route),
            "file" => Ok(Self::File),
            other => bail!("unknown label `{other}`; supported labels are {LABELS}"),
        }
    }

    fn kind(self) -> Option<SymbolKind> {
        match self {
            Self::Function => Some(SymbolKind::Function),
            Self::Method => Some(SymbolKind::Method),
            Self::Type => Some(SymbolKind::Type),
            Self::Module => Some(SymbolKind::Module),
            // Routes are symbols carrying a route path, not a distinct kind.
            Self::Route | Self::File => None,
        }
    }

    fn accepts(self, node: &Node) -> bool {
        match (self, node) {
            (Self::File, Node::File(_)) => true,
            (Self::Route, Node::Symbol(row)) => row.route_path.is_some(),
            (_, Node::Symbol(row)) => self.kind() == Some(row.kind),
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Relation {
    Calls,
    CalledBy,
    Extends,
    ImplementedBy,
    Contains,
    Imports,
    RoutesTo,
}

impl Relation {
    fn parse(text: &str) -> Result<Self> {
        match text.trim().to_ascii_uppercase().as_str() {
            "CALLS" => Ok(Self::Calls),
            "CALLED_BY" => Ok(Self::CalledBy),
            "EXTENDS" => Ok(Self::Extends),
            "IMPLEMENTED_BY" => Ok(Self::ImplementedBy),
            "CONTAINS" => Ok(Self::Contains),
            "IMPORTS" | "DEPENDS_ON" => Ok(Self::Imports),
            "ROUTES_TO" => Ok(Self::RoutesTo),
            other => bail!("unknown relationship `{other}`; supported are {RELATIONSHIPS}"),
        }
    }

    /// Endpoint labels a relationship can actually connect, checked up front so
    /// a nonsense pattern is an error rather than silently empty.
    fn check(self, left: Label, right: Label) -> Result<()> {
        let ok = match self {
            Self::Contains => left == Label::File && right != Label::File,
            Self::Imports => left == Label::File && right == Label::File,
            _ => left != Label::File && right != Label::File,
        };
        if ok {
            return Ok(());
        }
        bail!(
            "that relationship does not connect those labels: CONTAINS goes File->Symbol, \
             IMPORTS and DEPENDS_ON go File->File, and {RELATIONSHIPS} otherwise connect symbols"
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operator {
    Equals,
    Glob,
}

#[derive(Debug, Clone)]
struct Filter {
    alias: String,
    property: String,
    operator: Operator,
    value: String,
}

#[derive(Debug, Clone)]
enum Pattern {
    Node {
        alias: String,
        label: Label,
    },
    Edge {
        left: (String, Label),
        relation: Relation,
        right: (String, Label),
    },
}

#[derive(Debug, Clone)]
struct Query {
    pattern: Pattern,
    filters: Vec<Filter>,
    returns: Vec<(String, String)>,
    limit: Option<usize>,
}

fn parse(query: &str) -> Result<Query> {
    let text = query.trim();
    let upper = text.to_ascii_uppercase();
    if find_keyword(&upper, "MATCH", 0) != Some(0) {
        bail!("a query must start with MATCH; {SUPPORTED}");
    }
    let Some(return_at) = find_keyword(&upper, "RETURN", 5) else {
        bail!("a query must have a RETURN clause; {SUPPORTED}");
    };
    let where_at = find_keyword(&upper, "WHERE", 5).filter(|at| *at < return_at);
    let limit_at = find_keyword(&upper, "LIMIT", return_at + 6);

    let pattern = parse_pattern(&text[5..where_at.unwrap_or(return_at)])?;
    let filters = match where_at {
        Some(at) => parse_filters(&text[at + 5..return_at], &upper[at + 5..return_at])?,
        None => Vec::new(),
    };
    let returns = parse_returns(&text[return_at + 6..limit_at.unwrap_or(text.len())])?;
    let limit = match limit_at {
        Some(at) => Some(parse_limit(&text[at + 5..])?),
        None => None,
    };

    let aliases = pattern.aliases();
    check_aliases(&aliases, &filters, &returns)?;
    Ok(Query {
        pattern,
        filters,
        returns,
        limit,
    })
}

impl Pattern {
    fn aliases(&self) -> Vec<String> {
        match self {
            Self::Node { alias, .. } => vec![alias.clone()],
            Self::Edge { left, right, .. } => vec![left.0.clone(), right.0.clone()],
        }
    }
}

fn check_aliases(
    aliases: &[String],
    filters: &[Filter],
    returns: &[(String, String)],
) -> Result<()> {
    let used = filters
        .iter()
        .map(|filter| (&filter.alias, &filter.property))
        .chain(returns.iter().map(|(alias, property)| (alias, property)));
    for (alias, property) in used {
        if !aliases.contains(alias) {
            bail!(
                "unknown alias `{alias}`; the pattern declares {}",
                aliases.join(", ")
            );
        }
        if !PROPERTIES.split(", ").any(|known| known == property) {
            bail!("unknown property `{property}`; supported properties are {PROPERTIES}");
        }
    }
    Ok(())
}

fn parse_pattern(text: &str) -> Result<Pattern> {
    let text = text.trim();
    let Some((left, rest)) = text.split_once("-[") else {
        let (alias, label) = parse_node(text)?;
        return Ok(Pattern::Node { alias, label });
    };
    let Some((relation, right)) = rest.split_once("]->") else {
        bail!("a relationship must be written `-[:REL]->`; supported are {RELATIONSHIPS}");
    };
    let relation = Relation::parse(relation.trim().trim_start_matches(':'))?;
    let left = parse_node(left)?;
    let right = parse_node(right)?;
    relation.check(left.1, right.1)?;
    Ok(Pattern::Edge {
        left,
        relation,
        right,
    })
}

fn parse_node(text: &str) -> Result<(String, Label)> {
    let inner = text
        .trim()
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'));
    let Some((alias, label)) = inner.and_then(|value| value.split_once(':')) else {
        bail!("a node must be written `(alias:Label)`; supported labels are {LABELS}");
    };
    let alias = alias.trim();
    if alias.is_empty() || !alias.chars().all(is_word_char) {
        bail!("a node needs an alias, as in `(a:Function)`");
    }
    Ok((alias.to_owned(), Label::parse(label)?))
}

fn parse_filters(text: &str, upper: &str) -> Result<Vec<Filter>> {
    let mut filters = Vec::new();
    let mut start = 0;
    while let Some(at) = find_keyword(upper, "AND", start) {
        filters.push(parse_filter(&text[start..at])?);
        start = at + 3;
    }
    filters.push(parse_filter(&text[start..])?);
    Ok(filters)
}

fn parse_filter(text: &str) -> Result<Filter> {
    let text = text.trim();
    let (left, operator, right) = match text.split_once("=~") {
        Some((left, right)) => (left, Operator::Glob, right),
        None => match text.split_once('=') {
            Some((left, right)) => (left, Operator::Equals, right),
            None => bail!(
                "a WHERE condition must be `alias.prop = \"value\"` or \
                 `alias.prop =~ \"glob\"`, joined with AND"
            ),
        },
    };
    let Some((alias, property)) = left.trim().split_once('.') else {
        bail!("a WHERE condition must name a property, as in `a.name = \"x\"`");
    };
    Ok(Filter {
        alias: alias.trim().to_owned(),
        property: property.trim().to_owned(),
        operator,
        value: parse_string(right)?,
    })
}

fn parse_string(text: &str) -> Result<String> {
    let text = text.trim();
    let unquoted = text
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            text.strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        });
    match unquoted {
        Some(value) => Ok(value.to_owned()),
        None => bail!("a comparison value must be a quoted string, as in `a.name = \"x\"`"),
    }
}

fn parse_returns(text: &str) -> Result<Vec<(String, String)>> {
    let mut returns = Vec::new();
    for item in text.split(',') {
        let item = item.trim();
        if item.is_empty() {
            bail!("RETURN needs at least one `alias.prop`; supported properties are {PROPERTIES}");
        }
        let Some((alias, property)) = item.split_once('.') else {
            bail!("RETURN takes `alias.prop`, as in `RETURN a.name`; whole nodes are not returned");
        };
        returns.push((alias.trim().to_owned(), property.trim().to_owned()));
    }
    if returns.is_empty() {
        bail!("RETURN needs at least one `alias.prop`");
    }
    Ok(returns)
}

fn parse_limit(text: &str) -> Result<usize> {
    text.trim()
        .parse::<usize>()
        .map_err(|_| anyhow::anyhow!("LIMIT takes a whole number, as in `LIMIT 20`"))
}

/// Byte offset of `keyword` in an ASCII-uppercased copy of the query, matched as
/// a whole word so `a.name` never looks like a clause.
fn find_keyword(upper: &str, keyword: &str, from: usize) -> Option<usize> {
    let bytes = upper.as_bytes();
    let mut cursor = from;
    // forgeguard: allow FG-ALG-002 -- cursor advances past each match, so the scan is linear in the query string
    while let Some(offset) = upper.get(cursor..)?.find(keyword) {
        let start = cursor + offset;
        let end = start + keyword.len();
        let before = start == 0 || !is_word_byte(bytes[start - 1]);
        let after = end >= bytes.len() || !is_word_byte(bytes[end]);
        if before && after {
            return Some(start);
        }
        cursor = end;
    }
    None
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Node {
    Symbol(Box<SymbolRow>),
    File(PathBuf),
}

fn node_rows(
    store: &Store,
    query: &Query,
    alias: &str,
    label: Label,
    cap: usize,
) -> Result<Vec<Vec<String>>> {
    let nodes = scan(store, label, &query.filters, alias)?;
    let bindings = nodes
        .iter()
        .take(cap + 1)
        .map(|node| vec![(alias, node)])
        .collect::<Vec<_>>();
    Ok(project(&bindings, &query.returns))
}

fn edge_rows(
    store: &Store,
    query: &Query,
    left: &(String, Label),
    relation: Relation,
    right: &(String, Label),
    cap: usize,
) -> Result<Vec<Vec<String>>> {
    let sources = scan(store, left.1, &query.filters, &left.0)?;
    let context = Context::build(store, relation)?;
    let mut rows = Vec::new();
    for source in &sources {
        // forgeguard: allow FG-ALG-001 -- one expansion per matched source node, and `cap` stops it as soon as the page is full
        for target in expand(store, &context, relation, source)? {
            if !right.1.accepts(&target) || !matches(&target, &query.filters, &right.0) {
                continue;
            }
            let binding = vec![(left.0.as_str(), source), (right.0.as_str(), &target)];
            rows.extend(project(&[binding], &query.returns));
        }
        if rows.len() > cap {
            break;
        }
    }
    Ok(rows)
}

fn project(bindings: &[Vec<(&str, &Node)>], returns: &[(String, String)]) -> Vec<Vec<String>> {
    bindings
        .iter()
        .map(|binding| {
            returns
                .iter()
                .map(|(alias, property)| {
                    binding
                        .iter()
                        .find(|(name, _)| name == alias)
                        .map_or_else(String::new, |(_, node)| value_of(node, property))
                })
                .collect()
        })
        .collect()
}

fn scan(store: &Store, label: Label, filters: &[Filter], alias: &str) -> Result<Vec<Node>> {
    let nodes: Vec<Node> = match label {
        Label::File => store.all_paths()?.into_iter().map(Node::File).collect(),
        Label::Route => store.routes()?.into_iter().map(into_node).collect(),
        other => store
            .match_symbols(
                narrowing(filters, alias).as_deref(),
                other.kind(),
                SCAN_LIMIT,
            )?
            .into_iter()
            .map(into_node)
            .collect(),
    };
    Ok(nodes
        .into_iter()
        .filter(|node| matches(node, filters, alias))
        .collect())
}

fn into_node(row: SymbolRow) -> Node {
    Node::Symbol(Box::new(row))
}

/// A glob the store can pre-filter with, using its own bound-parameter LIKE.
/// Only `qualified` is safe to narrow on: a glob anchored at a bare `name`
/// would wrongly exclude `Service.getUser`, so that case falls through to the
/// Rust-side filter.
fn narrowing(filters: &[Filter], alias: &str) -> Option<String> {
    filters
        .iter()
        .find(|filter| {
            filter.alias == alias
                && (filter.property == "qualified"
                    || (filter.property == "name" && filter.operator == Operator::Equals))
        })
        .map(|filter| widen(&filter.value))
}

/// Characters the store's wildcard translation treats specially, plus the
/// backslash it emits as an escape.
const LIKE_SPECIAL: &str = "_%*?\\";

/// Widen a user value into a pattern that is only ever a *superset* of the real
/// match, because the store's `match_symbols` escapes `_` and `%` as `\_` / `\%`
/// while its `LIKE` has no `ESCAPE` clause — so SQLite reads the backslash
/// literally and `run_gate` silently matches nothing. Every character that
/// translation would mangle becomes `*`, which the store maps to `%`; the exact
/// comparison is then done in Rust by `matches`, which is where correctness
/// lives. The leading and trailing `*` are explicit because the store only
/// wraps a pattern in `%...%` when it contains no wildcard of its own.
fn widen(value: &str) -> String {
    let body = value
        .chars()
        .map(|character| {
            if LIKE_SPECIAL.contains(character) {
                '*'
            } else {
                character
            }
        })
        .collect::<String>();
    format!("*{body}*")
}

fn matches(node: &Node, filters: &[Filter], alias: &str) -> bool {
    filters
        .iter()
        .filter(|filter| filter.alias == alias)
        .all(|filter| {
            let actual = value_of(node, &filter.property);
            match filter.operator {
                Operator::Equals => actual.eq_ignore_ascii_case(&filter.value),
                Operator::Glob => {
                    glob_matches(&filter.value.to_lowercase(), &actual.to_lowercase())
                }
            }
        })
}

/// `*` and `?` only, matched with the classic backtracking two-pointer walk. A
/// regex engine would be a dependency and a denial-of-service surface for no
/// gain here.
fn glob_matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.chars().collect::<Vec<_>>();
    let value = value.chars().collect::<Vec<_>>();
    let (mut p, mut v) = (0, 0);
    let (mut star, mut resume) = (None, 0);
    while v < value.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == value[v]) {
            p += 1;
            v += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            resume = v;
            p += 1;
        } else if let Some(at) = star {
            p = at + 1;
            resume += 1;
            v = resume;
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|character| *character == '*')
}

fn value_of(node: &Node, property: &str) -> String {
    match node {
        Node::Symbol(row) => symbol_value(row, property),
        Node::File(path) => file_value(path, property),
    }
}

fn symbol_value(row: &SymbolRow, property: &str) -> String {
    match property {
        "name" => row.name.clone(),
        "qualified" => row.qualified.clone(),
        "kind" => row.kind.as_str().to_owned(),
        "path" => row.path.to_string_lossy().replace('\\', "/"),
        "line" => row.start_line.to_string(),
        "signature" => row.signature.clone(),
        "exported" => row.exported.to_string(),
        "is_test" => row.is_test.to_string(),
        "route_method" => row.route_method.clone().unwrap_or_default(),
        "route_path" => row.route_path.clone().unwrap_or_default(),
        _ => String::new(),
    }
}

fn file_value(path: &Path, property: &str) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    match property {
        "name" => path
            .file_name()
            .map(|value| value.to_string_lossy().into_owned())
            .unwrap_or_default(),
        "qualified" | "path" => text,
        "kind" => "file".to_owned(),
        _ => String::new(),
    }
}

/// Lookup tables a relationship needs that the store does not expose directly.
/// Each is built only for the relationship that uses it.
#[derive(Default)]
struct Context {
    stems: BTreeMap<String, Vec<PathBuf>>,
    routes: Vec<SymbolRow>,
    links: Vec<ServiceLink>,
}

impl Context {
    fn build(store: &Store, relation: Relation) -> Result<Self> {
        match relation {
            Relation::Imports => Ok(Self {
                stems: stem_index(store)?,
                ..Self::default()
            }),
            Relation::RoutesTo => Ok(Self {
                routes: store.routes()?,
                links: store.service_links()?,
                ..Self::default()
            }),
            _ => Ok(Self::default()),
        }
    }
}

fn stem_index(store: &Store) -> Result<BTreeMap<String, Vec<PathBuf>>> {
    let mut index: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for path in store.all_paths()? {
        let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        index.entry(stem.to_lowercase()).or_default().push(path);
    }
    Ok(index)
}

fn expand(store: &Store, context: &Context, relation: Relation, node: &Node) -> Result<Vec<Node>> {
    match (relation, node) {
        (Relation::Calls, Node::Symbol(row)) => rows(store.callees_of(row.id)?),
        (Relation::CalledBy, Node::Symbol(row)) => {
            rows(store.callers_of(&row.name, row.container.as_deref())?)
        }
        (Relation::ImplementedBy, Node::Symbol(row)) => rows(store.implementors_of(&row.name)?),
        (Relation::Extends, Node::Symbol(row)) => supertypes(store, row),
        (Relation::Contains, Node::File(path)) => rows(store.symbols_in_file(path)?),
        (Relation::Imports, Node::File(path)) => imported(store, context, path),
        (Relation::RoutesTo, Node::Symbol(row)) => Ok(routed(context, row)),
        _ => Ok(Vec::new()),
    }
}

fn rows(found: Vec<SymbolRow>) -> Result<Vec<Node>> {
    Ok(found.into_iter().map(into_node).collect())
}

fn supertypes(store: &Store, row: &SymbolRow) -> Result<Vec<Node>> {
    let mut nodes = Vec::new();
    for name in store.extends_of(row.id)? {
        // forgeguard: allow FG-ALG-001 -- a type declares a handful of
        // supertypes, and each lookup is an indexed exact-name query
        nodes.extend(store.find_symbols(&name, 8)?.into_iter().map(into_node));
    }
    Ok(nodes)
}

fn imported(store: &Store, context: &Context, path: &Path) -> Result<Vec<Node>> {
    let mut nodes = Vec::new();
    for module in store.imports_of(path)? {
        let Some(targets) = context.stems.get(&module_stem(&module)) else {
            continue;
        };
        // forgeguard: allow FG-ALG-001 -- imports resolve against a prebuilt
        // stem index, so the inner loop is over the few files sharing a stem
        nodes.extend(
            targets
                .iter()
                .filter(|target| target.as_path() != path)
                .cloned()
                .map(Node::File),
        );
    }
    Ok(nodes)
}

fn routed(context: &Context, row: &SymbolRow) -> Vec<Node> {
    context
        .links
        .iter()
        .filter(|link| link.symbol == row.qualified && link.path == row.path)
        .filter_map(|link| {
            let target = link.target.as_deref()?;
            context
                .routes
                .iter()
                .find(|route| route.qualified == target)
                .cloned()
                .map(into_node)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{Call, FileEntry, Route, ServiceCall, Symbol};

    fn base(name: &str, kind: SymbolKind, line: usize) -> Symbol {
        Symbol {
            name: name.to_owned(),
            container: None,
            kind,
            start_line: line,
            end_line: line + 3,
            signature: format!("fn {name}()"),
            exported: true,
            calls: Vec::new(),
            extends: Vec::new(),
            route: None,
            links: Vec::new(),
        }
    }

    fn entry(imports: Vec<&str>, symbols: Vec<Symbol>) -> FileEntry {
        FileEntry {
            language: "rust".to_owned(),
            content_hash: "0".to_owned(),
            symbols_hash: "0".to_owned(),
            mtime: 0,
            size: 0,
            is_test: false,
            last_indexed: 0,
            imports: imports.into_iter().map(str::to_owned).collect(),
            exports: Vec::new(),
            symbols,
        }
    }

    fn fixture() -> Store {
        let mut store = Store::open_memory().expect("open store");

        let mut handler = base("handle_login", SymbolKind::Function, 10);
        handler.calls = vec![Call {
            name: "validate".to_owned(),
            ..Call::default()
        }];
        handler.route = Some(Route {
            method: "POST".to_owned(),
            path: "/login".to_owned(),
        });

        let mut client = base("call_login", SymbolKind::Function, 30);
        client.links = vec![ServiceCall {
            method: Some("POST".to_owned()),
            url: "https://api.example.com/login".to_owned(),
        }];

        let mut service = base("AuthService", SymbolKind::Type, 1);
        service.extends = vec!["Service".to_owned()];

        store
            .upsert_file(
                Path::new("src/auth.rs"),
                &entry(vec!["crate::util"], vec![handler, client, service]),
            )
            .expect("upsert auth");
        store
            .upsert_file(
                Path::new("src/util.rs"),
                &entry(
                    Vec::new(),
                    vec![
                        base("validate", SymbolKind::Function, 4),
                        base("Service", SymbolKind::Type, 20),
                    ],
                ),
            )
            .expect("upsert util");
        store
    }

    fn run(store: &Store, query: &str, limit: usize) -> Result<QueryResult> {
        let root = tempfile::tempdir().expect("tempdir");
        run_query(root.path(), store, query, limit)
    }

    fn ok(store: &Store, query: &str, limit: usize) -> QueryResult {
        run(store, query, limit).expect("query should succeed")
    }

    fn rejected(query: &str) -> String {
        let store = Store::open_memory().expect("open store");
        run(&store, query, 10)
            .expect_err("query must be rejected")
            .to_string()
    }

    #[test]
    fn matches_a_single_label() {
        let result = ok(&fixture(), "MATCH (f:Function) RETURN f.name, f.path", 10);
        assert_eq!(result.columns, vec!["f.name", "f.path"]);
        assert_eq!(
            result.rows,
            vec![
                vec!["handle_login".to_owned(), "src/auth.rs".to_owned()],
                vec!["call_login".to_owned(), "src/auth.rs".to_owned()],
                vec!["validate".to_owned(), "src/util.rs".to_owned()],
            ]
        );
        assert!(!result.truncated);
    }

    #[test]
    fn filters_on_exact_name() {
        let result = ok(
            &fixture(),
            "MATCH (f:Function) WHERE f.name = \"validate\" RETURN f.qualified, f.line",
            10,
        );
        assert_eq!(
            result.rows,
            vec![vec!["validate".to_owned(), "4".to_owned()]]
        );
    }

    #[test]
    fn filters_on_glob_and_and() {
        let result = ok(
            &fixture(),
            "MATCH (f:Function) WHERE f.name =~ \"*login\" AND f.path = \"src/auth.rs\" RETURN f.name",
            10,
        );
        assert_eq!(
            result.rows,
            vec![
                vec!["handle_login".to_owned()],
                vec!["call_login".to_owned()]
            ]
        );
    }

    #[test]
    fn filters_on_kind_and_exported() {
        let result = ok(
            &fixture(),
            "MATCH (t:Type) WHERE t.kind = \"type\" AND t.exported = \"true\" RETURN t.name",
            10,
        );
        assert_eq!(
            result.rows,
            vec![vec!["AuthService".to_owned()], vec!["Service".to_owned()]]
        );
    }

    #[test]
    fn walks_calls_and_called_by() {
        let store = fixture();
        let calls = ok(
            &store,
            "MATCH (a:Function)-[:CALLS]->(b:Function) RETURN a.name, b.name",
            10,
        );
        assert_eq!(
            calls.rows,
            vec![vec!["handle_login".to_owned(), "validate".to_owned()]]
        );
        let reverse = ok(
            &store,
            "MATCH (a:Function)-[:CALLED_BY]->(b:Function) RETURN a.name, b.name",
            10,
        );
        assert_eq!(
            reverse.rows,
            vec![vec!["validate".to_owned(), "handle_login".to_owned()]]
        );
    }

    #[test]
    fn walks_extends_and_implemented_by() {
        let store = fixture();
        let extends = ok(
            &store,
            "MATCH (a:Type)-[:EXTENDS]->(b:Type) RETURN a.name, b.name",
            10,
        );
        assert_eq!(
            extends.rows,
            vec![vec!["AuthService".to_owned(), "Service".to_owned()]]
        );
        let implemented = ok(
            &store,
            "MATCH (a:Type)-[:IMPLEMENTED_BY]->(b:Type) RETURN a.name, b.name",
            10,
        );
        assert_eq!(
            implemented.rows,
            vec![vec!["Service".to_owned(), "AuthService".to_owned()]]
        );
    }

    #[test]
    fn walks_contains_imports_and_depends_on() {
        let store = fixture();
        let contains = ok(
            &store,
            "MATCH (f:File)-[:CONTAINS]->(s:Type) WHERE f.path = \"src/util.rs\" RETURN f.name, s.name",
            10,
        );
        assert_eq!(
            contains.rows,
            vec![vec!["util.rs".to_owned(), "Service".to_owned()]]
        );
        let imports = ok(
            &store,
            "MATCH (a:File)-[:IMPORTS]->(b:File) RETURN a.path, b.path",
            10,
        );
        assert_eq!(
            imports.rows,
            vec![vec!["src/auth.rs".to_owned(), "src/util.rs".to_owned()]]
        );
        let depends = ok(
            &store,
            "MATCH (a:File)-[:DEPENDS_ON]->(b:File) RETURN a.path, b.path",
            10,
        );
        assert_eq!(depends.rows, imports.rows);
    }

    #[test]
    fn walks_routes_to() {
        let result = ok(
            &fixture(),
            "MATCH (a:Function)-[:ROUTES_TO]->(b:Route) RETURN a.name, b.route_method, b.route_path",
            10,
        );
        assert_eq!(
            result.rows,
            vec![vec![
                "call_login".to_owned(),
                "POST".to_owned(),
                "/login".to_owned()
            ]]
        );
    }

    #[test]
    fn route_label_selects_symbols_with_a_route() {
        let result = ok(&fixture(), "MATCH (r:Route) RETURN r.route_path", 10);
        assert_eq!(result.rows, vec![vec!["/login".to_owned()]]);
    }

    /// Names carrying a SQL LIKE metacharacter. `_` is the common one in Rust,
    /// Python, and Go, so equality that cannot survive it is equality that does
    /// not work on most repositories.
    fn underscore_fixture() -> Store {
        let mut store = Store::open_memory().expect("open store");

        let mut run = base("run_gate", SymbolKind::Function, 10);
        run.calls = vec![Call {
            name: "finish_gate".to_owned(),
            ..Call::default()
        }];

        let mut method = base("run_gate", SymbolKind::Function, 40);
        method.container = Some("Gate".to_owned());

        store
            .upsert_file(
                Path::new("crates/core/gate.rs"),
                &entry(
                    Vec::new(),
                    vec![run, method, base("finish_gate", SymbolKind::Function, 70)],
                ),
            )
            .expect("upsert gate");
        store
    }

    #[test]
    fn equality_matches_names_containing_an_underscore() {
        let store = underscore_fixture();

        let by_name = ok(
            &store,
            "MATCH (f:Function) WHERE f.name = \"run_gate\" RETURN f.qualified",
            10,
        );
        assert_eq!(
            by_name.rows,
            vec![
                vec!["run_gate".to_owned()],
                vec!["Gate.run_gate".to_owned()]
            ]
        );

        let bare = ok(
            &store,
            "MATCH (f:Function) WHERE f.qualified = \"run_gate\" RETURN f.line",
            10,
        );
        assert_eq!(bare.rows, vec![vec!["10".to_owned()]]);

        let contained = ok(
            &store,
            "MATCH (f:Function) WHERE f.qualified = \"Gate.run_gate\" RETURN f.line",
            10,
        );
        assert_eq!(contained.rows, vec![vec!["40".to_owned()]]);
    }

    #[test]
    fn two_hop_equality_on_an_underscored_name() {
        let result = ok(
            &underscore_fixture(),
            "MATCH (f:Function)-[:CALLS]->(g:Function) WHERE f.name = \"run_gate\" RETURN f.name, g.qualified",
            10,
        );
        assert_eq!(
            result.rows,
            vec![vec!["run_gate".to_owned(), "finish_gate".to_owned()]]
        );
    }

    #[test]
    fn limit_clause_and_cap_both_truncate() {
        let store = fixture();
        let clause = ok(&store, "MATCH (f:Function) RETURN f.name LIMIT 1", 10);
        assert_eq!(clause.rows.len(), 1);
        assert!(clause.truncated);

        let cap = ok(&store, "MATCH (f:Function) RETURN f.name", 2);
        assert_eq!(cap.rows.len(), 2);
        assert!(cap.truncated);

        // The caller's cap wins when the clause asks for more.
        let capped = ok(&store, "MATCH (f:Function) RETURN f.name LIMIT 99", 1);
        assert_eq!(capped.rows.len(), 1);
        assert!(capped.truncated);
    }

    #[test]
    fn edge_query_respects_the_cap() {
        let result = ok(
            &fixture(),
            "MATCH (a:Function)-[:CALLS]->(b:Function) RETURN a.name",
            0,
        );
        assert!(result.rows.is_empty());
        assert!(result.truncated);
    }

    #[test]
    fn rejects_mutations_and_injection() {
        for query in [
            "CREATE (n:Function {name: \"x\"}) RETURN n.name",
            "MATCH (n:Function) DELETE n RETURN n.name",
            "MATCH (n:Function) SET n.name = \"x\" RETURN n.name",
            "MERGE (n:Function) RETURN n.name",
            "DROP TABLE files",
            "MATCH (n:Function) RETURN n.name; DROP TABLE files",
            "ATTACH DATABASE 'evil.db' AS evil",
            "PRAGMA table_info(files)",
            "MATCH (n:Function) WHERE n.name = \"x'; DROP TABLE files --\" RETURN n.name",
            "MATCH (n:Function) WHERE n.name = \"a\" UNION SELECT 1 RETURN n.name",
            "MATCH (n:Function) RETURN n.name -- comment",
        ] {
            let message = rejected(query);
            assert!(
                message.contains("not allowed") || message.contains("must start with MATCH"),
                "unexpected message for {query}: {message}"
            );
        }
    }

    #[test]
    fn rejects_malformed_but_harmless_queries() {
        assert!(rejected("MATCH (n:Function)").contains("RETURN"));
        assert!(rejected("MATCH (n:Widget) RETURN n.name").contains("Function"));
        assert!(
            rejected("MATCH (n:Function)-[:OWNS]->(m:Function) RETURN n.name").contains("CALLS")
        );
        assert!(rejected("MATCH (n:Function) RETURN n").contains("alias.prop"));
        assert!(rejected("MATCH (n:Function) RETURN n.colour").contains("qualified"));
        assert!(rejected("MATCH (n:Function) RETURN m.name").contains("unknown alias"));
        assert!(rejected("MATCH (n:Function) WHERE n.name = x RETURN n.name").contains("quoted"));
        assert!(rejected("MATCH (n:Function) RETURN n.name LIMIT many").contains("whole number"));
        assert!(
            rejected("MATCH (f:File)-[:IMPORTS]->(n:Function) RETURN n.name")
                .contains("does not connect")
        );
    }

    #[test]
    fn glob_matching_handles_stars_and_question_marks() {
        assert!(glob_matches("*login", "handle_login"));
        assert!(glob_matches("handle_*", "handle_login"));
        assert!(glob_matches("h?ndle_login", "handle_login"));
        assert!(glob_matches("*", "anything"));
        assert!(!glob_matches("login*", "handle_login"));
        assert!(!glob_matches("h?ndle", "handle_login"));
    }

    #[test]
    fn counters_record_the_query() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = fixture();
        run_query(root.path(), &store, "MATCH (f:Function) RETURN f.name", 10).expect("query");
        let stats = crate::memory::MemoryStats::load(root.path());
        assert_eq!(stats.graph_queries, 1);
    }
}
