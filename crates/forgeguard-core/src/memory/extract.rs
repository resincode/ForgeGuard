//! Symbol extraction on top of the scanner's tree-sitter parsers.
//!
//! Grammars differ per language, but their node names are conventional enough
//! (`*function*`, `*method*`, `class_*`, `struct_*`, `*import*`) that one
//! structural walk covers every parser the scanner already links, instead of one
//! hand-written query per language.
//!
//! Receiver types, routes, and outbound service calls are resolved by the same
//! structural convention: a static, dependency-free pass that answers only when
//! the answer is unambiguous. A wrong receiver poisons caller/callee joins in
//! the store, so every resolver here returns `None` rather than a guess.

use std::collections::{BTreeMap, BTreeSet};

use tree_sitter::{Node, Parser};

use super::{Call, Route, ServiceCall, Symbol, SymbolKind};
use crate::scanner::LanguageProfile;

#[derive(Debug, Default)]
pub(crate) struct FileFacts {
    pub imports: Vec<String>,
    pub exports: Vec<String>,
    pub symbols: Vec<Symbol>,
}

const MAX_SIGNATURE: usize = 200;
const MAX_IMPORT: usize = 120;

const HTTP_VERBS: &[&str] = &["get", "post", "put", "delete", "patch", "head", "options"];

/// Substrings that mark a receiver as an HTTP client rather than a router.
const CLIENT_TOKENS: &[&str] = &[
    "axios",
    "requests",
    "reqwest",
    "httpx",
    "http",
    "client",
    "session",
    "urllib",
    "superagent",
];

/// Names that always denote the enclosing type in the languages we resolve.
const SELF_NAMES: &[&str] = &["self", "this", "Self", "cls"];

pub(crate) fn extract(profile: LanguageProfile, source: &str) -> FileFacts {
    let mut parser = Parser::new();
    if parser.set_language(&profile.language()).is_err() {
        return FileFacts::default();
    }
    let Some(tree) = parser.parse(source, None) else {
        return FileFacts::default();
    };
    let root = tree.root_node();

    let mut walker = Walker {
        source,
        profile,
        types: known_types(root, source),
        containers: Vec::new(),
        facts: FileFacts::default(),
    };
    walker.walk(root);
    let mut facts = walker.facts;
    attach_routes(root, source, &mut facts.symbols);

    facts.exports = facts
        .symbols
        .iter()
        .filter(|symbol| symbol.exported)
        .map(Symbol::qualified)
        .collect();
    facts.imports.sort();
    facts.imports.dedup();
    facts.exports.sort();
    facts.exports.dedup();
    facts
}

struct Walker<'a> {
    source: &'a str,
    profile: LanguageProfile,
    types: BTreeSet<String>,
    containers: Vec<String>,
    facts: FileFacts,
}

impl Walker<'_> {
    fn walk(&mut self, node: Node) {
        if is_import(node.kind()) {
            self.facts.imports.extend(import_targets(node, self.source));
            return;
        }
        if let Some(symbol) = self.symbol_at(node) {
            let nests = matches!(symbol.kind, SymbolKind::Type | SymbolKind::Module);
            let name = symbol.name.clone();
            self.facts.symbols.push(symbol);
            if !nests {
                // Function bodies hold statements, not further top-level structure.
                return;
            }
            self.containers.push(name);
            self.walk_children(node);
            self.containers.pop();
            return;
        }
        self.walk_children(node);
    }

    fn walk_children(&mut self, node: Node) {
        for index in 0..node.named_child_count() {
            if let Some(child) = node.named_child(index as u32) {
                self.walk(child);
            }
        }
    }

    fn symbol_at(&self, node: Node) -> Option<Symbol> {
        let kind = classify(node.kind())?;
        let name = symbol_name(node, self.source)?;
        let container = method_container(node, self.source, &self.containers);
        let scope = Scope {
            types: &self.types,
            locals: local_types(node, self.source, container.as_deref()),
            container: container.clone(),
        };
        let (calls, links) = call_facts(node, self.source, &scope);
        let route = annotations(node, self.source)
            .iter()
            .find_map(|raw| route_from_annotation(raw));
        Some(Symbol {
            exported: is_exported(node, self.source, self.profile, &name),
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            signature: signature(node, self.source),
            extends: collect_extends(node, self.source),
            kind: if route.is_some() {
                SymbolKind::Route
            } else {
                kind
            },
            calls,
            links,
            route,
            name,
            container,
        })
    }
}

/// Node kinds are grammar-specific, so classification is by convention with an
/// explicit reject list for the many `*_type`, `*_body`, `*_call` nodes that
/// contain the same words but declare nothing.
fn classify(kind: &str) -> Option<SymbolKind> {
    const REJECT_SUFFIX: &[&str] = &[
        "_body",
        "_type",
        "_expression",
        "_parameters",
        "_parameter",
        "_arguments",
        "_argument",
        "_list",
        "_clause",
        "_pattern",
        "_identifier",
        "_block",
        "_call",
        "_reference",
        "_constraint",
        "_modifier",
        "_variant",
        "_use",
    ];
    if REJECT_SUFFIX.iter().any(|suffix| kind.ends_with(suffix)) {
        return None;
    }
    if kind.contains("method") || kind == "singleton_method" {
        return Some(SymbolKind::Method);
    }
    if kind.contains("function") || kind == "fun_decl" || kind == "constructor_declaration" {
        return Some(SymbolKind::Function);
    }
    if kind.contains("class")
        || kind.contains("struct")
        || kind.contains("interface")
        || kind.contains("trait")
        || kind.contains("enum")
        || kind == "impl_item"
        || kind == "object_declaration"
        || kind.contains("type_alias")
    {
        return Some(SymbolKind::Type);
    }
    if kind.contains("namespace") || kind == "mod_item" || kind == "module" {
        return Some(SymbolKind::Module);
    }
    None
}

fn symbol_name(node: Node, source: &str) -> Option<String> {
    // `impl Trait for Type` names the type it extends, not itself.
    if node.kind() == "impl_item" {
        return node
            .child_by_field_name("type")
            .map(|child| terminal(text(child, source)))
            .filter(|value| !value.is_empty());
    }
    for field in ["name", "declarator", "pattern", "alias"] {
        if let Some(child) = node.child_by_field_name(field) {
            let value = terminal(text(child, source));
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    (0..node.named_child_count())
        .filter_map(|index| node.named_child(index as u32))
        .find(|child| child.kind().ends_with("identifier") || child.kind() == "word")
        .map(|child| terminal(text(child, source)))
        .filter(|value| !value.is_empty())
}

/// Methods declared with an explicit receiver (Go, Rust free-standing impls)
/// carry their owner in the node rather than in the enclosing scope.
fn method_container(node: Node, source: &str, containers: &[String]) -> Option<String> {
    if let Some(receiver) = node.child_by_field_name("receiver") {
        let owner = terminal(text(receiver, source).trim_matches(|c: char| !c.is_alphanumeric()));
        if !owner.is_empty() {
            return Some(owner);
        }
    }
    containers.last().cloned()
}

fn collect_extends(node: Node, source: &str) -> Vec<String> {
    if node.kind() == "impl_item" {
        return node
            .child_by_field_name("trait")
            .map(|child| vec![terminal(text(child, source))])
            .unwrap_or_default();
    }
    let mut names = Vec::new();
    for index in 0..node.named_child_count() {
        let Some(child) = node.named_child(index as u32) else {
            continue;
        };
        let kind = child.kind();
        if kind.contains("superclass")
            || kind.contains("base_clause")
            || kind.contains("extends")
            || kind.contains("implements")
            || kind.contains("heritage")
            || kind == "argument_list"
        {
            names.extend(identifiers(child, source));
        }
    }
    names.sort();
    names.dedup();
    names
}

/// Every named node under `node`, itself included, in one pass.
fn descendants(node: Node<'_>) -> Vec<Node<'_>> {
    let mut out = Vec::new();
    let mut stack = vec![node];
    while let Some(current) = stack.pop() {
        // forgeguard: allow FG-ALG-001 -- pushes each child once; the walk visits every node exactly once
        for index in 0..current.named_child_count() {
            if let Some(child) = current.named_child(index as u32) {
                stack.push(child);
            }
        }
        out.push(current);
    }
    out
}

fn identifiers(node: Node, source: &str) -> Vec<String> {
    descendants(node)
        .into_iter()
        .filter(|current| current.kind().ends_with("identifier"))
        .map(|current| terminal(text(current, source)))
        .filter(|value| !value.is_empty())
        .collect()
}

/// Types this file can name with certainty: declared here, or imported under an
/// uppercase name. Anything else stays unresolved.
fn known_types(root: Node, source: &str) -> BTreeSet<String> {
    let mut types = BTreeSet::new();
    for node in descendants(root) {
        if is_import(node.kind()) {
            types.extend(
                identifiers(node, source)
                    .into_iter()
                    .filter(|value| starts_upper(value)),
            );
            continue;
        }
        if classify(node.kind()) == Some(SymbolKind::Type) {
            types.extend(symbol_name(node, source));
        }
    }
    types
}

struct Scope<'a> {
    types: &'a BTreeSet<String>,
    /// `None` marks a name bound to more than one type: unresolvable, not absent.
    locals: BTreeMap<String, Option<String>>,
    container: Option<String>,
}

/// Callees invoked inside the declaration plus the outbound HTTP calls among
/// them, both deduplicated.
fn call_facts(node: Node, source: &str, scope: &Scope) -> (Vec<Call>, Vec<ServiceCall>) {
    let mut calls = Vec::new();
    let mut links = Vec::new();
    for current in descendants(node) {
        if !is_call(current.kind()) {
            continue;
        }
        let Some(path) = callee_path(current, source) else {
            continue;
        };
        links.extend(service_call(&path, current, source));
        let name = terminal(&path);
        if !name.is_empty() {
            let position = callee_position(current).unwrap_or_else(|| current.start_position());
            calls.push(Call {
                receiver: resolve_receiver(&path, scope),
                qualifier: split_path(&path).map(|(left, _)| left.to_owned()),
                line: position.row + 1,
                character: position.column,
                name,
            });
        }
    }
    calls.sort_by(|left, right| (&left.name, &left.receiver).cmp(&(&right.name, &right.receiver)));
    calls.dedup();
    links.sort_by(|left, right| (&left.url, &left.method).cmp(&(&right.url, &right.method)));
    links.dedup();
    (calls, links)
}

/// Where a language server should be asked about this call: the callee
/// expression itself, not the argument list around it.
fn callee_position(node: Node) -> Option<tree_sitter::Point> {
    for field in ["function", "name", "constructor", "macro"] {
        if let Some(child) = node.child_by_field_name(field) {
            return Some(child.start_position());
        }
    }
    node.named_child(0).map(|child| child.start_position())
}

fn is_call(kind: &str) -> bool {
    matches!(
        kind,
        "call"
            | "call_expression"
            | "function_call"
            | "function_call_expression"
            | "method_invocation"
            | "invocation_expression"
            | "macro_invocation"
            | "new_expression"
            | "command"
    )
}

/// The callee as written, e.g. `self.foo`, `Repo::new`, `axios.get`.
fn callee_path(node: Node, source: &str) -> Option<String> {
    for field in ["function", "name", "constructor", "macro"] {
        if let Some(child) = node.child_by_field_name(field) {
            let value = text(child, source).trim();
            if !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    node.named_child(0)
        .map(|child| text(child, source).trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// Split `a.b` / `A::b` into the qualifier and the invoked name.
fn split_path(path: &str) -> Option<(&str, &str)> {
    let index = path.rfind(['.', ':'])?;
    let last = path[index + 1..].trim();
    let left = path[..index].trim_end_matches(':').trim();
    (!left.is_empty() && !last.is_empty()).then_some((left, last))
}

fn resolve_receiver(path: &str, scope: &Scope) -> Option<String> {
    let (left, _) = split_path(path)?;
    if !is_identifier(left) {
        // `a.b.c()` or a call chain: the owning type is not statically obvious.
        return None;
    }
    if SELF_NAMES.contains(&left) {
        return scope.container.clone();
    }
    if let Some(bound) = scope.locals.get(left) {
        return bound.clone();
    }
    scope.types.contains(left).then(|| left.to_owned())
}

fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_alphanumeric() || character == '_')
}

fn starts_upper(value: &str) -> bool {
    value.chars().next().is_some_and(char::is_uppercase)
}

/// Local variables whose declaration binds a constructor call, plus the method
/// receiver variable. A name bound twice to different types maps to `None`.
fn local_types(
    node: Node,
    source: &str,
    container: Option<&str>,
) -> BTreeMap<String, Option<String>> {
    let mut locals = BTreeMap::new();
    if let (Some(name), Some(owner)) = (receiver_name(node, source), container) {
        locals.insert(name, Some(owner.to_owned()));
    }
    for current in descendants(node) {
        if !is_declaration(current.kind()) {
            continue;
        }
        let (Some(name), Some(bound)) =
            (declared_name(current, source), bound_type(current, source))
        else {
            continue;
        };
        locals
            .entry(name)
            .and_modify(|existing| {
                if existing.as_deref() != Some(bound.as_str()) {
                    *existing = None;
                }
            })
            .or_insert(Some(bound));
    }
    locals
}

fn is_declaration(kind: &str) -> bool {
    matches!(
        kind,
        "let_declaration"
            | "variable_declarator"
            | "assignment"
            | "assignment_expression"
            | "short_var_declaration"
            | "var_spec"
            | "const_spec"
            | "local_variable_declaration"
    )
}

fn declared_name(node: Node, source: &str) -> Option<String> {
    for field in ["pattern", "name", "left", "declarator"] {
        if let Some(child) = node.child_by_field_name(field) {
            let value = terminal(text(child, source));
            if is_identifier(&value) {
                return Some(value);
            }
        }
    }
    None
}

/// The type a declaration binds, when the right-hand side is a constructor.
fn bound_type(node: Node, source: &str) -> Option<String> {
    let value = ["value", "right"]
        .iter()
        .find_map(|field| node.child_by_field_name(field))
        .map(unwrap_list);
    let Some(value) = value else {
        // `var x Foo`: no initializer, but the annotation is the type.
        return node
            .child_by_field_name("type")
            .map(|child| terminal(text(child, source)))
            .filter(|value| starts_upper(value));
    };
    let kind = value.kind();
    if kind == "new_expression"
        || kind.contains("composite_literal")
        || kind.contains("struct_expr")
    {
        return ["constructor", "type", "name"]
            .iter()
            .find_map(|field| value.child_by_field_name(field))
            .map(|child| terminal(text(child, source)))
            .filter(|value| starts_upper(value));
    }
    if is_call(kind) {
        return callee_path(value, source)
            .as_deref()
            .and_then(head_type)
            .filter(|value| starts_upper(value));
    }
    None
}

/// Go wraps both sides of `x := Foo{}` in an expression list.
fn unwrap_list(node: Node<'_>) -> Node<'_> {
    if node.kind().ends_with("_list") {
        return node.named_child(0).unwrap_or(node);
    }
    node
}

fn head_type(path: &str) -> Option<String> {
    let head = path.split(['.', ':', '(', '<']).next()?.trim();
    is_identifier(head).then(|| head.to_owned())
}

fn receiver_name(node: Node, source: &str) -> Option<String> {
    let receiver = node.child_by_field_name("receiver")?;
    let raw = text(receiver, source).trim_matches(['(', ')']);
    let mut parts = raw.split_whitespace();
    let name = parts.next()?;
    // A receiver without a type word is `(Foo)`, which binds no variable.
    parts.next()?;
    is_identifier(name).then(|| name.to_owned())
}

/// Outbound HTTP calls. Only absolute literal URLs are recorded: a relative path
/// names this service's own route, and a dynamic URL names nothing.
fn service_call(path: &str, node: Node, source: &str) -> Option<ServiceCall> {
    let (left, last) = split_path(path).unwrap_or(("", path));
    let verb = last.to_ascii_lowercase();
    let method = if verb == "fetch" {
        None
    } else if HTTP_VERBS.contains(&verb.as_str()) && is_client(left) {
        Some(verb.to_ascii_uppercase())
    } else {
        return None;
    };
    let url = argument(node, 0).and_then(|arg| literal_prefix(arg, source))?;
    url.starts_with("http")
        .then_some(ServiceCall { method, url })
}

fn is_client(left: &str) -> bool {
    let lower = left.to_ascii_lowercase();
    CLIENT_TOKENS.iter().any(|token| lower.contains(token))
}

fn argument<'t>(node: Node<'t>, index: usize) -> Option<Node<'t>> {
    node.child_by_field_name("arguments")?
        .named_child(index as u32)
}

fn quoted(node: Node, source: &str) -> Option<String> {
    let kind = node.kind();
    if !(kind.contains("string") || kind.contains("template")) {
        return None;
    }
    let raw = text(node, source).trim_matches(['"', '\'', '`']);
    (!raw.is_empty()).then(|| raw.to_owned())
}

/// The literal head of a string, stopping at the first interpolation.
fn literal_prefix(node: Node, source: &str) -> Option<String> {
    let raw = quoted(node, source)?;
    let value = raw
        .split("${")
        .next()
        .unwrap_or(&raw)
        .split('{')
        .next()
        .unwrap_or(&raw);
    (!value.is_empty()).then(|| value.to_owned())
}

/// Decorators, annotations, and attribute macros attached to a declaration.
fn annotations<'a>(node: Node, source: &'a str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut sibling = node.prev_named_sibling();
    while let Some(current) = sibling {
        if !is_annotation(current.kind()) {
            break;
        }
        out.push(text(current, source));
        sibling = current.prev_named_sibling();
    }
    for index in 0..node.named_child_count() {
        let Some(child) = node.named_child(index as u32) else {
            continue;
        };
        if is_annotation(child.kind()) {
            out.push(text(child, source));
        } else if child.kind() == "modifiers" {
            out.extend(annotation_children(child, source));
        }
    }
    out
}

fn annotation_children<'a>(node: Node, source: &'a str) -> Vec<&'a str> {
    (0..node.named_child_count())
        .filter_map(|index| node.named_child(index as u32))
        .filter(|child| is_annotation(child.kind()))
        .map(|child| text(child, source))
        .collect()
}

fn is_annotation(kind: &str) -> bool {
    kind.contains("decorator") || kind.contains("annotation") || kind.starts_with("attribute")
}

fn route_from_annotation(raw: &str) -> Option<Route> {
    let body = raw.trim().trim_start_matches(['@', '#', '[']).trim_end();
    let (head, rest) = body.split_once('(')?;
    let args = rest.rsplit_once(')').map_or(rest, |(before, _)| before);
    let verb = head.rsplit(['.', ':']).next()?.trim();
    let method = annotation_method(verb, args)?;
    match first_literal(args) {
        // A verb that is not applied to a path is something else entirely, such
        // as `@patch("module.attr")` in a test.
        Some(value) if !value.starts_with('/') => None,
        Some(path) => Some(Route { method, path }),
        None => Some(Route {
            method,
            path: "/".to_owned(),
        }),
    }
}

fn annotation_method(verb: &str, args: &str) -> Option<String> {
    let lower = verb.to_ascii_lowercase();
    if HTTP_VERBS.contains(&lower.as_str()) {
        return Some(lower.to_ascii_uppercase());
    }
    if lower == "route" || lower == "requestmapping" {
        return Some(method_arg(args).unwrap_or_else(|| "GET".to_owned()));
    }
    let base = lower.strip_suffix("mapping")?;
    HTTP_VERBS
        .contains(&base)
        .then(|| base.to_ascii_uppercase())
}

/// `methods=["POST"]` and `method=RequestMethod.POST` both reduce to one verb
/// word once the punctuation is dropped.
fn method_arg(args: &str) -> Option<String> {
    let rest = args.split_once("method")?.1;
    rest.split(|character: char| !character.is_alphabetic())
        .map(str::to_ascii_lowercase)
        .find(|word| HTTP_VERBS.contains(&word.as_str()))
        .map(|word| word.to_ascii_uppercase())
}

fn first_literal(value: &str) -> Option<String> {
    let start = value.find(['"', '\'', '`'])?;
    let quote = value.as_bytes()[start] as char;
    let rest = &value[start + 1..];
    let end = rest.find(quote)?;
    (end > 0).then(|| rest[..end].to_owned())
}

/// Routes declared as a registration call attach to the handler they name, or to
/// the symbol that encloses the registration when the handler is inline.
fn attach_routes(root: Node, source: &str, symbols: &mut [Symbol]) {
    for node in descendants(root) {
        if !is_call(node.kind()) {
            continue;
        }
        let Some((route, handler)) = registration(node, source) else {
            continue;
        };
        let line = node.start_position().row + 1;
        let Some(index) = target_symbol(symbols, line, handler.as_deref()) else {
            continue;
        };
        if symbols[index].route.is_some() {
            continue;
        }
        symbols[index].route = Some(route);
        symbols[index].kind = SymbolKind::Route;
    }
}

fn registration(node: Node, source: &str) -> Option<(Route, Option<String>)> {
    let path = callee_path(node, source)?;
    let (left, last) = split_path(&path)?;
    if is_client(left) {
        return None;
    }
    // Every registration form passes the path first and a handler after it.
    let path_arg = argument(node, 0).and_then(|arg| quoted(arg, source))?;
    let handler = handler_name(node, source);
    if !path_arg.starts_with('/') || argument(node, 1).is_none() {
        return None;
    }
    let method = registration_method(last, node, source)?;
    Some((
        Route {
            method,
            path: path_arg,
        },
        handler,
    ))
}

fn registration_method(verb: &str, node: Node, source: &str) -> Option<String> {
    let lower = verb.to_ascii_lowercase();
    if HTTP_VERBS.contains(&lower.as_str()) {
        return Some(lower.to_ascii_uppercase());
    }
    if lower == "handlefunc" || lower == "handle" {
        // gorilla/http register one handler for every verb.
        return Some("ANY".to_owned());
    }
    (lower == "route").then(|| wrapper_method(node, source).unwrap_or_else(|| "ANY".to_owned()))
}

/// axum spells the verb as a wrapper around the handler: `.route("/x", get(h))`.
fn wrapper_method(node: Node, source: &str) -> Option<String> {
    let arg = argument(node, 1)?;
    if !is_call(arg.kind()) {
        return None;
    }
    let verb = terminal(&callee_path(arg, source)?).to_ascii_lowercase();
    HTTP_VERBS
        .contains(&verb.as_str())
        .then(|| verb.to_ascii_uppercase())
}

fn handler_name(node: Node, source: &str) -> Option<String> {
    let arg = argument(node, 1)?;
    if arg.kind().ends_with("identifier") {
        return Some(terminal(text(arg, source)));
    }
    if is_call(arg.kind()) {
        let inner = argument(arg, 0)?;
        return inner
            .kind()
            .ends_with("identifier")
            .then(|| terminal(text(inner, source)));
    }
    None
}

fn target_symbol(symbols: &[Symbol], line: usize, handler: Option<&str>) -> Option<usize> {
    if let Some(name) = handler {
        let declared = symbols
            .iter()
            .position(|symbol| symbol.name == name && symbol.kind != SymbolKind::Type);
        if declared.is_some() {
            return declared;
        }
    }
    symbols
        .iter()
        .enumerate()
        .filter(|(_, symbol)| symbol.start_line <= line && line <= symbol.end_line)
        .min_by_key(|(_, symbol)| symbol.lines())
        .map(|(index, _)| index)
}

fn is_import(kind: &str) -> bool {
    kind.contains("import") && !kind.ends_with("_clause") && !kind.ends_with("_specifier")
        || kind == "use_declaration"
        || kind == "preproc_include"
        || kind == "require"
}

/// The module paths a declaration imports. The store joins these against file
/// stems to build DEPENDS_ON edges, so the result must be a path and never
/// source text: a stored `use std::{` stems to `{` and resolves to nothing.
///
/// A declaration that names several modules yields one entry each, because the
/// shared prefix of `use crate::{config, scanner}` is `crate`, which stems to
/// nothing and would lose exactly the internal edges the graph is for.
fn import_targets(node: Node, source: &str) -> Vec<String> {
    let literals = string_literals(node, source);
    if !literals.is_empty() {
        // A grouped Go block holds one literal per module, not just the first.
        return literals;
    }
    modules_from_text(&flatten(text(node, source)))
}

fn string_literals(node: Node, source: &str) -> Vec<String> {
    descendants(node)
        .into_iter()
        .filter(|current| current.kind().contains("string"))
        .map(|current| text(current, source).trim_matches(['"', '\'', '`', '<', '>']))
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

fn flatten(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Declarations that name their modules in code rather than in a string.
fn modules_from_text(declaration: &str) -> Vec<String> {
    let line = strip_visibility(declaration.trim());
    let (keyword, rest) = match line.split_once(char::is_whitespace) {
        Some((keyword, rest)) => (keyword, rest.trim()),
        None => return Vec::new(),
    };
    match keyword {
        "use" => rust_paths(rest.trim_end_matches(';')),
        // `from x.y import (a, b)` imports from the module named first.
        "from" => rest
            .split_whitespace()
            .next()
            .map(|module| vec![module.to_owned()])
            .unwrap_or_default(),
        "import" => plain_modules(rest),
        _ => vec![truncate(line, MAX_IMPORT)],
    }
}

fn strip_visibility(line: &str) -> &str {
    match line.split_once(char::is_whitespace) {
        Some((first, rest)) if first.starts_with("pub") => rest.trim(),
        _ => line,
    }
}

/// `a::{b, c::D}` expands to `a::b` and `a::c`; a trailing CamelCase segment is
/// the imported item, not a module, so it is dropped.
fn rust_paths(path: &str) -> Vec<String> {
    let path = path.trim();
    let Some(open) = path.find('{') else {
        return rust_module(path).into_iter().collect();
    };
    let prefix = path[..open].trim().trim_end_matches(':');
    let mut modules = Vec::new();
    for part in split_top_level(brace_body(&path[open..])) {
        if part == "self" {
            modules.extend(rust_module(prefix));
            continue;
        }
        modules.extend(rust_paths(&qualify(prefix, part)));
    }
    modules
}

fn qualify(prefix: &str, part: &str) -> String {
    if prefix.is_empty() {
        return part.to_owned();
    }
    format!("{prefix}::{part}")
}

fn rust_module(path: &str) -> Option<String> {
    let path = path.split(" as ").next().unwrap_or(path);
    let mut parts: Vec<&str> = path
        .split("::")
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    let leaf_is_item = parts
        .last()
        .is_some_and(|last| starts_upper(last) || *last == "*");
    if parts.len() > 1 && leaf_is_item {
        parts.pop();
    }
    (!parts.is_empty()).then(|| truncate(&parts.join("::"), MAX_IMPORT))
}

fn brace_body(value: &str) -> &str {
    let mut depth = 0usize;
    for (index, character) in value.char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return &value[1..index];
                }
            }
            _ => {}
        }
    }
    value.strip_prefix('{').unwrap_or(value)
}

/// Split on commas that are not inside a nested brace group.
fn split_top_level(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, character) in value.char_indices() {
        match character {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(value[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(value[start..].trim());
    parts.retain(|part| !part.is_empty());
    parts
}

fn plain_modules(rest: &str) -> Vec<String> {
    rest.trim_matches(['(', ')', ';'])
        .split(',')
        .map(|part| part.split(" as ").next().unwrap_or(part))
        .map(|part| part.trim().trim_matches(['(', ')', ';']).trim())
        .filter(|part| !part.is_empty())
        .map(|part| truncate(part, MAX_IMPORT))
        .collect()
}

fn is_exported(node: Node, source: &str, profile: LanguageProfile, name: &str) -> bool {
    match profile.family() {
        "rust" => text(node, source).starts_with("pub"),
        "go" => name.chars().next().is_some_and(char::is_uppercase),
        "python" => !name.starts_with('_'),
        "javascript" => node
            .parent()
            .is_some_and(|parent| parent.kind().starts_with("export")),
        _ => true,
    }
}

fn signature(node: Node, source: &str) -> String {
    let first = text(node, source).split('\n').next().unwrap_or_default();
    truncate(first.trim(), MAX_SIGNATURE)
}

fn text<'a>(node: Node, source: &'a str) -> &'a str {
    source.get(node.byte_range()).unwrap_or_default()
}

/// `foo::bar::baz`, `self.foo.bar`, `a.b()`, and `Graph<'a>` all identify
/// `baz` / `bar` / `b` / `Graph`: generics and arguments are cut first so the
/// tail is always the declared name.
fn terminal(value: &str) -> String {
    value
        .rsplit(['.', ':', '!', ' ', '&', '*', '\n'])
        .map(|part| part.split(['<', '(', '{', '[']).next().unwrap_or_default())
        .map(|part| {
            part.trim()
                .trim_matches(|character: char| !(character.is_alphanumeric() || character == '_'))
        })
        .find(|part| !part.is_empty())
        .unwrap_or_default()
        .to_owned()
}

fn truncate(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }
    let mut end = limit;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &value[..end])
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn facts(name: &str, source: &str) -> FileFacts {
        let profile = LanguageProfile::from_path(Path::new(name)).expect("known extension");
        extract(profile, source)
    }

    fn symbol<'a>(facts: &'a FileFacts, name: &str) -> &'a Symbol {
        facts
            .symbols
            .iter()
            .find(|symbol| symbol.name == name)
            .unwrap_or_else(|| panic!("no symbol {name}"))
    }

    /// Distinct receivers recorded for `callee`. Calls are stored per site, so a
    /// callee invoked twice through the same expression still counts once here.
    fn receivers(facts: &FileFacts, symbol_name: &str, callee: &str) -> Vec<Option<String>> {
        let mut found: Vec<_> = symbol(facts, symbol_name)
            .calls
            .iter()
            .filter(|call| call.name == callee)
            .map(|call| call.receiver.clone())
            .collect();
        found.sort();
        found.dedup();
        assert!(!found.is_empty(), "no call {callee}");
        found
    }

    fn receiver(facts: &FileFacts, symbol_name: &str, callee: &str) -> Option<String> {
        let found = receivers(facts, symbol_name, callee);
        assert_eq!(found.len(), 1, "{callee} resolved more than one way");
        found[0].clone()
    }

    #[test]
    fn resolves_rust_receivers() {
        let facts = facts(
            "x.rs",
            r#"
struct Repo;
impl Repo {
    pub fn find(&self) {}
    pub fn run(&self) {
        self.find();
        let other = Repo::new();
        other.find();
        helper();
    }
}
"#,
        );
        assert_eq!(receiver(&facts, "run", "find"), Some("Repo".to_owned()));
        assert_eq!(receiver(&facts, "run", "new"), Some("Repo".to_owned()));
        assert_eq!(receiver(&facts, "run", "helper"), None);
    }

    #[test]
    fn resolves_python_receivers() {
        let facts = facts(
            "x.py",
            r#"
class Repo:
    def find(self):
        pass

    def run(self):
        self.find()
        other = Repo()
        other.find()
        unknown.find()
"#,
        );
        assert_eq!(
            receivers(&facts, "run", "find"),
            vec![None, Some("Repo".to_owned())]
        );
    }

    #[test]
    fn resolves_javascript_receivers() {
        let facts = facts(
            "x.js",
            r#"
class Repo {
  find() {}
  run() {
    this.find();
    const other = new Repo();
    other.find();
  }
}
"#,
        );
        assert_eq!(receiver(&facts, "run", "find"), Some("Repo".to_owned()));
    }

    #[test]
    fn resolves_go_receivers() {
        let facts = facts(
            "x.go",
            r#"
package main

type Repo struct{}

func (r *Repo) Find() {}

func (r *Repo) Run() {
	r.Find()
	other := Repo{}
	other.Find()
}
"#,
        );
        assert_eq!(receiver(&facts, "Run", "Find"), Some("Repo".to_owned()));
    }

    #[test]
    fn ambiguous_receivers_stay_unresolved() {
        let facts = facts(
            "x.py",
            r#"
class Repo:
    def run(self):
        thing = Repo()
        thing = Other()
        thing.find()
"#,
        );
        assert_eq!(receiver(&facts, "run", "find"), None);
    }

    #[test]
    fn reads_python_decorator_routes() {
        let facts = facts(
            "x.py",
            r#"
@app.get("/users")
def list_users():
    pass

@app.route("/users", methods=["POST"])
def create_user():
    pass

@router.post("/items")
def create_item():
    pass
"#,
        );
        let listed = symbol(&facts, "list_users");
        assert_eq!(listed.kind, SymbolKind::Route);
        assert_eq!(
            listed.route,
            Some(Route {
                method: "GET".to_owned(),
                path: "/users".to_owned()
            })
        );
        assert_eq!(
            symbol(&facts, "create_user")
                .route
                .as_ref()
                .map(|r| &r.method),
            Some(&"POST".to_owned())
        );
        assert_eq!(
            symbol(&facts, "create_item")
                .route
                .as_ref()
                .map(|r| &r.path),
            Some(&"/items".to_owned())
        );
    }

    #[test]
    fn reads_express_registration_routes() {
        let facts = facts(
            "x.js",
            r#"
function listUsers(req, res) {}
app.get("/users", listUsers);
"#,
        );
        assert_eq!(
            symbol(&facts, "listUsers").route,
            Some(Route {
                method: "GET".to_owned(),
                path: "/users".to_owned()
            })
        );
    }

    #[test]
    fn reads_nest_decorator_routes() {
        let facts = facts(
            "x.ts",
            r#"
class UserController {
  @Get("/users")
  list() {}

  @Post()
  create() {}
}
"#,
        );
        assert_eq!(
            symbol(&facts, "list").route.as_ref().map(|r| &r.path),
            Some(&"/users".to_owned())
        );
        assert_eq!(
            symbol(&facts, "create").route,
            Some(Route {
                method: "POST".to_owned(),
                path: "/".to_owned()
            })
        );
    }

    #[test]
    fn reads_go_registration_routes() {
        let facts = facts(
            "x.go",
            r#"
package main

func handleUsers() {}

func main() {
	mux.HandleFunc("/users", handleUsers)
}
"#,
        );
        assert_eq!(
            symbol(&facts, "handleUsers").route,
            Some(Route {
                method: "ANY".to_owned(),
                path: "/users".to_owned()
            })
        );
    }

    #[test]
    fn reads_rust_attribute_and_axum_routes() {
        let facts = facts(
            "x.rs",
            r#"
#[get("/health")]
fn health() {}

fn users() {}

fn app() {
    Router::new().route("/users", get(users));
}
"#,
        );
        assert_eq!(
            symbol(&facts, "health").route,
            Some(Route {
                method: "GET".to_owned(),
                path: "/health".to_owned()
            })
        );
        assert_eq!(
            symbol(&facts, "users").route,
            Some(Route {
                method: "GET".to_owned(),
                path: "/users".to_owned()
            })
        );
    }

    #[test]
    fn reads_java_mapping_routes() {
        let facts = facts(
            "x.java",
            r#"
class UserController {
    @GetMapping("/users")
    public void list() {}

    @RequestMapping(value = "/users", method = RequestMethod.POST)
    public void create() {}
}
"#,
        );
        assert_eq!(
            symbol(&facts, "list").route,
            Some(Route {
                method: "GET".to_owned(),
                path: "/users".to_owned()
            })
        );
        assert_eq!(
            symbol(&facts, "create").route,
            Some(Route {
                method: "POST".to_owned(),
                path: "/users".to_owned()
            })
        );
    }

    #[test]
    fn records_cross_service_links() {
        let facts = facts(
            "x.js",
            r#"
async function load(id) {
  await fetch("http://billing/invoices");
  await axios.get(`http://billing/invoices/${id}`);
  await axios.get(dynamicUrl);
}
"#,
        );
        assert_eq!(
            symbol(&facts, "load").links,
            vec![
                ServiceCall {
                    method: None,
                    url: "http://billing/invoices".to_owned()
                },
                ServiceCall {
                    method: Some("GET".to_owned()),
                    url: "http://billing/invoices/".to_owned()
                },
            ]
        );
    }

    #[test]
    fn rust_imports_record_module_paths() {
        let facts = facts(
            "x.rs",
            r#"
use std::{
    fs,
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use serde::Serialize;
use crate::config::CONFIG_FILE;
use anyhow::{Context, Result};
pub use crate::scanner::LanguageProfile;
"#,
        );
        assert_eq!(
            facts.imports,
            vec![
                "anyhow",
                "crate::config",
                "crate::scanner",
                "serde",
                "std::collections",
                "std::fs",
                "std::path",
            ]
        );
    }

    #[test]
    fn python_imports_record_module_paths() {
        let facts = facts(
            "x.py",
            r#"
from x.y import (
    alpha,
    beta,
)
import os, sys
from .config import CONFIG
"#,
        );
        assert_eq!(facts.imports, vec![".config", "os", "sys", "x.y"]);
    }

    #[test]
    fn go_grouped_imports_record_every_module() {
        let facts = facts(
            "x.go",
            r#"
package main

import (
	"fmt"
	"net/http"
)
"#,
        );
        assert_eq!(facts.imports, vec!["fmt", "net/http"]);
    }

    /// Guards the join `store::module_stem` performs: the last `/`, `.`, `:` or
    /// `\` segment of a stored import must be the stem of the file it names.
    #[test]
    fn stored_imports_stem_to_their_file() {
        let facts = facts(
            "x.rs",
            "use crate::config::CONFIG_FILE;\nuse crate::memory::store::Store;\n",
        );
        let stems: Vec<String> = facts
            .imports
            .iter()
            .map(|import| crate::memory::store::module_stem(import))
            .collect();
        assert_eq!(stems, vec!["config", "store"]);
    }

    #[test]
    fn records_python_service_links() {
        let facts = facts(
            "x.py",
            r#"
def push():
    requests.post("http://billing/charge")
"#,
        );
        assert_eq!(
            symbol(&facts, "push").links,
            vec![ServiceCall {
                method: Some("POST".to_owned()),
                url: "http://billing/charge".to_owned()
            }]
        );
    }
}
