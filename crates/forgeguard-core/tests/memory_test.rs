use std::{fs, path::Path, process::Command};

use forgeguard_core::{
    config::ScanConfig,
    memory::{
        analyze_impact, architecture, delete_project, export_artifact, find_symbols,
        index_repository, index_status, refresh_changed, run_query, search_symbols, symbol_card,
        trace_path, watch_repository, Detail, Direction, IndexOptions, IndexReport, LspOptions,
        MemoryStats, RetrievalOptions, RiskLevel, Store, SymbolKind, WatchOptions, BEST_LEVEL,
    },
};
use tempfile::{tempdir, TempDir};

const AUTH_SERVICE: &str = r#"
import { Session } from "./session";

export class AuthService {
    validateToken(token) {
        return Session.lookup(token) && this.isFresh(token);
    }

    isFresh(token) {
        return token.expiry > Date.now();
    }
}
"#;

const MIDDLEWARE: &str = r#"
import { AuthService } from "./auth";

export function requireAuth(request) {
    const service = new AuthService();
    return service.validateToken(request.token);
}
"#;

const SESSION: &str = r#"
export class Session {
    static lookup(token) {
        return token;
    }
}
"#;

fn fixture() -> TempDir {
    let directory = tempdir().expect("temp directory");
    write(directory.path(), "src/auth.ts", AUTH_SERVICE);
    write(directory.path(), "src/middleware.ts", MIDDLEWARE);
    write(directory.path(), "src/session.ts", SESSION);
    write(
        directory.path(),
        "tests/auth.test.ts",
        "import { AuthService } from \"../src/auth\";\n\
         export function testValidateToken() {\n\
         \x20   return new AuthService().validateToken(\"t\");\n\
         }\n",
    );
    directory
}

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("parent directory")).expect("create directory");
    fs::write(path, contents).expect("write fixture");
}

fn index(root: &Path) -> IndexReport {
    index_repository(root, &ScanConfig::default(), &IndexOptions::default()).expect("index")
}

fn store(root: &Path) -> Store {
    Store::open(root).expect("open store")
}

fn git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(root)
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?} failed");
}

fn git_repository() -> TempDir {
    let directory = fixture();
    git(directory.path(), &["init", "--quiet"]);
    git(
        directory.path(),
        &["config", "user.email", "test@example.com"],
    );
    git(
        directory.path(),
        &["config", "user.name", "ForgeGuard Test"],
    );
    git(directory.path(), &["add", "."]);
    git(directory.path(), &["commit", "--quiet", "-m", "base"]);
    directory
}

#[test]
fn initial_indexing_records_files_symbols_and_relationships() {
    let directory = fixture();
    let report = index(directory.path());

    assert_eq!(report.files, 4, "every source file is indexed");
    assert_eq!(report.reused, 0);
    assert!(report.parsed >= 4);

    let store = store(directory.path());
    let auth = store
        .file_entry(Path::new("src/auth.ts"))
        .expect("read entry")
        .expect("auth file entry");
    // The scanner's language families group TypeScript with JavaScript.
    assert_eq!(auth.language, "javascript");
    assert!(auth.imports.iter().any(|import| import.contains("session")));

    let validate = auth
        .symbols
        .iter()
        .find(|symbol| symbol.name == "validateToken")
        .expect("validateToken symbol");
    assert_eq!(validate.kind, SymbolKind::Method);
    assert_eq!(validate.container.as_deref(), Some("AuthService"));
    assert!(validate.calls.iter().any(|call| call.name == "lookup"));
    assert_eq!(validate.qualified(), "AuthService.validateToken");
}

#[test]
fn second_run_reuses_unchanged_files_without_reparsing() {
    let directory = fixture();
    index(directory.path());

    let report = index(directory.path());

    assert_eq!(report.parsed, 0, "nothing changed, nothing reparsed");
    assert_eq!(report.reused, 4);
}

#[test]
fn touching_a_file_without_editing_it_keeps_the_cached_symbols() {
    let directory = fixture();
    index(directory.path());
    // Rewrite identical bytes so size matches but mtime moves.
    write(directory.path(), "src/auth.ts", AUTH_SERVICE);

    let report = index(directory.path());

    assert_eq!(report.parsed, 0, "identical content hash short-circuits");
    assert_eq!(report.reused, 4);
}

#[test]
fn modifying_a_file_reindexes_only_that_file_and_updates_the_call_graph() {
    let directory = fixture();
    index(directory.path());
    write(
        directory.path(),
        "src/middleware.ts",
        "import { AuthService } from \"./auth\";\n\
         export function requireAuth(request) {\n\
         \x20   return new AuthService().isFresh(request.token);\n\
         }\n",
    );

    let report = index(directory.path());
    assert_eq!(report.parsed, 1);
    assert_eq!(report.reused, 3);

    let card = symbol_card(
        directory.path(),
        &store(directory.path()),
        "AuthService.validateToken",
        &RetrievalOptions {
            detail: Detail::Structure,
            ..RetrievalOptions::default()
        },
    )
    .expect("card")
    .expect("symbol found");
    assert!(
        !card
            .callers
            .iter()
            .any(|caller| caller.path == Path::new("src/middleware.ts")),
        "middleware no longer calls validateToken"
    );
    assert!(
        card.callers
            .iter()
            .any(|caller| caller.path == Path::new("tests/auth.test.ts")),
        "the test still calls it"
    );
}

#[test]
fn deleting_a_file_removes_its_symbols_from_the_graph() {
    let directory = fixture();
    index(directory.path());
    fs::remove_file(directory.path().join("src/session.ts")).expect("remove file");

    let report = index(directory.path());

    assert_eq!(report.removed, 1);
    let store = store(directory.path());
    assert!(store
        .file_entry(Path::new("src/session.ts"))
        .expect("read entry")
        .is_none());
    assert!(find_symbols(directory.path(), &store, "Session", 10)
        .expect("find")
        .iter()
        .all(|hit| hit.path != Path::new("src/session.ts")));
}

#[test]
fn renaming_a_file_moves_its_symbols_to_the_new_path() {
    let directory = fixture();
    index(directory.path());
    fs::rename(
        directory.path().join("src/session.ts"),
        directory.path().join("src/session_store.ts"),
    )
    .expect("rename file");

    let report = index(directory.path());

    assert_eq!(report.renamed, 1, "identical content under a new path");
    assert_eq!(report.removed, 1);
    let store = store(directory.path());
    assert!(store
        .file_entry(Path::new("src/session_store.ts"))
        .expect("read entry")
        .is_some());
    assert!(store
        .file_entry(Path::new("src/session.ts"))
        .expect("read entry")
        .is_none());
}

#[test]
fn adding_and_removing_symbols_is_reflected_in_lookups() {
    let directory = fixture();
    index(directory.path());
    write(
        directory.path(),
        "src/auth.ts",
        "export class AuthService {\n\
         \x20   refreshToken(token) {\n\
         \x20       return token;\n\
         \x20   }\n\
         }\n",
    );

    index(directory.path());
    let store = store(directory.path());

    assert!(find_symbols(directory.path(), &store, "refreshToken", 10)
        .expect("find")
        .iter()
        .any(|hit| hit.qualified == "AuthService.refreshToken"));
    assert!(
        find_symbols(directory.path(), &store, "AuthService.isFresh", 10)
            .expect("find")
            .is_empty(),
        "removed symbols are not served"
    );
}

#[test]
fn a_schema_change_invalidates_the_whole_index_instead_of_serving_stale_symbols() {
    let directory = fixture();
    index(directory.path());
    {
        let store = store(directory.path());
        store
            .set_meta("schema_version", "999")
            .expect("rewrite version");
    }

    // Opening with a foreign schema version drops the tables.
    let store = store(directory.path());
    assert_eq!(store.counts().expect("counts").0, 0);

    let report = index(directory.path());
    assert_eq!(report.parsed, 4, "and rebuilt from source");
}

#[test]
fn an_entry_pointing_outside_the_repository_is_refused() {
    let directory = fixture();
    index(directory.path());
    let mut store = store(directory.path());
    let entry = store
        .file_entry(Path::new("src/auth.ts"))
        .expect("read entry")
        .expect("entry");

    let refused = store.upsert_file(Path::new("../../etc/passwd"), &entry);

    assert!(
        refused.is_err(),
        "escaping paths never reach a filesystem read"
    );
    assert!(store
        .file_entry(Path::new("src/middleware.ts"))
        .expect("read entry")
        .is_some());
}

#[test]
fn generic_rust_impl_blocks_qualify_their_methods_with_the_bare_type_name() {
    let directory = tempdir().expect("temp directory");
    write(
        directory.path(),
        "src/graph.rs",
        "pub struct Graph<'a> { pub name: &'a str }\n\
         \n\
         impl<'a> Graph<'a> {\n\
         \x20   pub fn hit(&self) -> usize {\n\
         \x20       self.name.len()\n\
         \x20   }\n\
         }\n",
    );
    index(directory.path());
    let store = store(directory.path());

    let hits = find_symbols(directory.path(), &store, "Graph.hit", 10).expect("find");

    assert_eq!(hits.len(), 1, "generic parameters are not part of the name");
    assert_eq!(hits[0].qualified, "Graph.hit");
    assert!(
        store
            .all_symbols()
            .expect("symbols")
            .iter()
            .all(|symbol| !symbol.name.is_empty()),
        "no nameless symbols are stored"
    );
}

#[test]
fn symbol_lookup_supports_exact_qualified_and_fuzzy_queries() {
    let directory = fixture();
    index(directory.path());
    let store = store(directory.path());

    let exact = find_symbols(directory.path(), &store, "AuthService.validateToken", 10)
        .expect("find exact");
    assert_eq!(exact.len(), 1);
    assert_eq!(exact[0].path, Path::new("src/auth.ts"));

    let fuzzy = find_symbols(directory.path(), &store, "validatetok", 10).expect("find fuzzy");
    assert!(fuzzy.iter().any(|hit| hit.name == "validateToken"));
}

#[test]
fn lookups_handle_names_containing_sql_wildcard_characters() {
    let directory = tempdir().expect("temp directory");
    write(
        directory.path(),
        "src/gate.rs",
        "pub fn run_gate() -> usize { 1 }\n\
         pub fn run_other() -> usize { 2 }\n",
    );
    index(directory.path());
    let store = store(directory.path());

    // `_` is a SQL LIKE wildcard; the substring fallback must still be exact
    // about it rather than matching `runXgate` or nothing at all.
    let hits = find_symbols(directory.path(), &store, "run_gate", 10).expect("find");

    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].qualified, "run_gate");
    assert!(
        find_symbols(directory.path(), &store, "un_ga", 10)
            .expect("find")
            .iter()
            .any(|hit| hit.name == "run_gate"),
        "a substring containing the wildcard still resolves"
    );
}

#[test]
fn ranked_search_finds_a_symbol_from_separated_words() {
    let directory = fixture();
    index(directory.path());
    let store = store(directory.path());

    let hits = search_symbols(directory.path(), &store, "validate token", 10).expect("search");

    assert!(
        hits.iter()
            .any(|hit| hit.qualified.contains("validateToken")),
        "camelCase names are matched by their parts"
    );
}

#[test]
fn structure_level_returns_callers_callees_dependents_and_tests_without_source() {
    let directory = fixture();
    index(directory.path());

    let card = symbol_card(
        directory.path(),
        &store(directory.path()),
        "AuthService.validateToken",
        &RetrievalOptions {
            detail: Detail::Structure,
            ..RetrievalOptions::default()
        },
    )
    .expect("card")
    .expect("symbol found");

    assert_eq!(card.bytes_returned, 0, "structure level reads no source");
    assert!(card.signature.is_some());
    assert!(card
        .callers
        .iter()
        .any(|caller| caller.qualified == "requireAuth"));
    assert!(card
        .callees
        .iter()
        .any(|callee| callee.qualified.contains("lookup")));
    assert!(card
        .dependents
        .contains(&std::path::PathBuf::from("src/middleware.ts")));
    assert!(card
        .tests
        .iter()
        .any(|test| test.path == Path::new("tests/auth.test.ts")));

    // The flattened hit must not collide with the card's own relation lists.
    let json: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&card).expect("serialize card"))
            .expect("card is valid JSON");
    assert!(json["callers"].is_array());
    assert!(json["caller_count"].is_number());
}

#[test]
fn snippet_level_returns_only_the_symbol_body() {
    let directory = fixture();
    index(directory.path());

    let card = symbol_card(
        directory.path(),
        &store(directory.path()),
        "AuthService.validateToken",
        &RetrievalOptions {
            detail: Detail::Snippet,
            ..RetrievalOptions::default()
        },
    )
    .expect("card")
    .expect("symbol found");

    let snippet = card.snippet.expect("snippet");
    assert!(snippet.contains("validateToken"));
    assert!(!snippet.contains("isFresh(token) {"), "neighbours stay out");
    assert!(card.bytes_avoided > card.bytes_returned);
}

#[test]
fn the_byte_budget_drops_source_instead_of_overrunning_it() {
    let directory = fixture();
    index(directory.path());

    let card = symbol_card(
        directory.path(),
        &store(directory.path()),
        "AuthService.validateToken",
        &RetrievalOptions {
            detail: Detail::Full,
            max_bytes: 10,
        },
    )
    .expect("card")
    .expect("symbol found");

    assert!(card.budget_exceeded);
    assert!(card.snippet.is_none());
    assert_eq!(card.bytes_returned, 0);
}

#[test]
fn retrieval_counters_record_what_the_layer_avoided() {
    let directory = fixture();
    index(directory.path());
    symbol_card(
        directory.path(),
        &store(directory.path()),
        "AuthService.validateToken",
        &RetrievalOptions {
            detail: Detail::Snippet,
            ..RetrievalOptions::default()
        },
    )
    .expect("card");

    let stats = MemoryStats::load(directory.path());
    assert_eq!(stats.symbol_reads, 1);
    assert_eq!(stats.full_file_reads, 0);
    assert!(stats.bytes_avoided > 0);
    assert!(stats.estimated_tokens_avoided() > 0);
    assert!(stats.graph_queries >= 1);
}

#[test]
fn tracing_walks_the_call_chain_in_both_directions() {
    let directory = fixture();
    index(directory.path());
    let store = store(directory.path());

    let inbound = trace_path(
        directory.path(),
        &store,
        "AuthService.validateToken",
        Direction::Inbound,
        3,
    )
    .expect("trace")
    .expect("symbol found");
    assert!(inbound
        .nodes
        .iter()
        .any(|node| node.qualified == "requireAuth"));

    let outbound = trace_path(
        directory.path(),
        &store,
        "requireAuth",
        Direction::Outbound,
        3,
    )
    .expect("trace")
    .expect("symbol found");
    // requireAuth -> validateToken -> lookup is two hops.
    assert!(outbound
        .nodes
        .iter()
        .any(|node| node.qualified.contains("lookup") && node.depth == 2));
}

#[test]
fn trace_depth_is_clamped_to_the_documented_maximum() {
    let directory = fixture();
    index(directory.path());

    let report = trace_path(
        directory.path(),
        &store(directory.path()),
        "requireAuth",
        Direction::Both,
        99,
    )
    .expect("trace")
    .expect("symbol found");

    assert_eq!(report.depth, 5);
}

#[test]
fn graph_queries_answer_structural_questions_and_reject_mutations() {
    let directory = fixture();
    index(directory.path());
    let store = store(directory.path());

    let result = run_query(
        directory.path(),
        &store,
        "MATCH (f:Method) RETURN f.qualified",
        50,
    )
    .expect("query");
    assert!(result
        .rows
        .iter()
        .any(|row| row.iter().any(|value| value.contains("validateToken"))));

    for attempt in [
        "DELETE FROM files",
        "MATCH (f:Function) RETURN f.name; DROP TABLE files",
        "CREATE (n:Function)",
    ] {
        assert!(
            run_query(directory.path(), &store, attempt, 10).is_err(),
            "{attempt} must be refused"
        );
    }
}

#[test]
fn architecture_summarises_modules_layers_and_entry_points() {
    let directory = fixture();
    write(
        directory.path(),
        "src/routes/user_controller.ts",
        "export function main() { return 1; }\n",
    );
    index(directory.path());

    let architecture =
        architecture(directory.path(), &store(directory.path())).expect("architecture");

    assert_eq!(architecture.languages.get("javascript"), Some(&5));
    assert_eq!(architecture.test_files, 1);
    assert!(architecture
        .modules
        .iter()
        .any(|module| module.name.contains("src")));
    assert!(architecture
        .layers
        .iter()
        .any(|layer| layer.name == "routes"));
    assert!(architecture
        .entry_points
        .iter()
        .any(|entry| entry.qualified == "main"));
}

#[test]
fn layer_keywords_match_path_words_not_substrings() {
    let directory = tempdir().expect("temp directory");
    // "forgeguard" contains "guard", which must not file the file under middleware.
    write(
        directory.path(),
        "crates/forgeguard-core/src/gate.rs",
        "pub fn run_gate() -> usize { 1 }\n",
    );
    write(
        directory.path(),
        "src/auth/guard.rs",
        "pub fn check() -> usize { 1 }\n",
    );
    index(directory.path());

    let architecture =
        architecture(directory.path(), &store(directory.path())).expect("architecture");
    let middleware = architecture
        .layers
        .iter()
        .find(|layer| layer.name == "middleware");

    assert_eq!(
        middleware.map(|layer| layer.files),
        Some(1),
        "only the real guard file counts: {:?}",
        architecture.layers
    );
    assert!(middleware
        .expect("middleware layer")
        .paths
        .contains(&std::path::PathBuf::from("src/auth/guard.rs")));
}

#[test]
fn routes_and_cross_service_calls_become_graph_edges() {
    let directory = tempdir().expect("temp directory");
    write(
        directory.path(),
        "src/api.py",
        "from fastapi import APIRouter\n\
         router = APIRouter()\n\
         \n\
         @router.get(\"/users/{id}\")\n\
         def read_user(id):\n\
         \x20   return id\n",
    );
    write(
        directory.path(),
        "src/client.ts",
        "export async function loadUser(id) {\n\
         \x20   return fetch(\"http://api/users/7\");\n\
         }\n",
    );
    index(directory.path());

    let architecture =
        architecture(directory.path(), &store(directory.path())).expect("architecture");

    assert!(
        architecture
            .routes
            .iter()
            .any(|route| route.method == "GET" && route.route == "/users/{id}"),
        "the decorator became a route node: {:?}",
        architecture.routes
    );
    assert!(
        architecture
            .service_edges
            .iter()
            .any(|edge| edge.url.contains("/users/7") && edge.target.is_some()),
        "the client call resolved to the route: {:?}",
        architecture.service_edges
    );
}

#[test]
fn git_diff_impact_reaches_callers_dependents_tests_and_scores_risk() {
    let directory = git_repository();
    index(directory.path());

    write(
        directory.path(),
        "src/auth.ts",
        &AUTH_SERVICE.replace("token.expiry > Date.now()", "token.expiry >= Date.now()"),
    );
    let report = refresh_changed(directory.path(), &ScanConfig::default(), None).expect("refresh");
    assert_eq!(report.parsed, 1, "only the changed file is reparsed");

    let impact = analyze_impact(directory.path(), &store(directory.path()), None).expect("impact");

    assert_eq!(
        impact.changed_files,
        vec![std::path::PathBuf::from("src/auth.ts")]
    );
    assert!(impact
        .changed_symbols
        .iter()
        .any(|symbol| symbol.symbol.qualified == "AuthService.isFresh"));
    assert!(impact
        .dependent_files
        .contains(&std::path::PathBuf::from("src/middleware.ts")));
    assert!(impact.blast_radius >= 2);
    assert!(
        impact.risk >= RiskLevel::Low,
        "every changed symbol carries a risk level"
    );
    assert!(
        impact
            .changed_symbols
            .iter()
            .all(|symbol| symbol.risk == RiskLevel::Low || !symbol.reasons.is_empty()),
        "anything above low risk says why"
    );
}

#[test]
fn a_deleted_file_leaves_the_graph_on_a_changed_only_refresh() {
    let directory = git_repository();
    index(directory.path());
    fs::remove_file(directory.path().join("src/session.ts")).expect("remove file");

    let report = refresh_changed(directory.path(), &ScanConfig::default(), None).expect("refresh");

    assert_eq!(report.removed, 1);
    let store = store(directory.path());
    assert!(store
        .file_entry(Path::new("src/session.ts"))
        .expect("read entry")
        .is_none());
    assert!(
        store
            .file_entry(Path::new("src/auth.ts"))
            .expect("read entry")
            .is_some(),
        "a scoped refresh keeps the rest of the graph"
    );
}

#[test]
fn status_reports_index_presence_and_staleness_against_head() {
    let directory = git_repository();
    let before = index_status(directory.path()).expect("status");
    assert!(!before.indexed);

    index(directory.path());
    let after = index_status(directory.path()).expect("status");

    assert!(after.indexed);
    assert_eq!(after.files, 4);
    assert!(!after.stale, "indexed at the checked-out commit");
    assert_eq!(after.commit, after.head_commit);
}

#[test]
fn deleting_a_project_removes_the_graph_but_not_the_source() {
    let directory = fixture();
    index(directory.path());

    assert!(delete_project(directory.path()).expect("delete"));

    assert!(
        directory.path().join("src/auth.ts").is_file(),
        "source is never touched"
    );
    assert!(!index_status(directory.path()).expect("status").indexed);
}

#[test]
fn the_shared_artifact_seeds_a_fresh_checkout_without_a_full_parse() {
    let source = fixture();
    index(source.path());
    let artifact = export_artifact(source.path(), None, BEST_LEVEL).expect("export");
    assert!(artifact.path.is_file());
    assert!(
        artifact.compressed_bytes < artifact.raw_bytes,
        "the committed artifact is compressed: {artifact:?}"
    );

    // A teammate's checkout: same sources, no local cache, artifact committed.
    let clone = fixture();
    fs::create_dir_all(clone.path().join(".forgeguard/memory")).expect("create artifact directory");
    fs::copy(
        &artifact.path,
        clone.path().join(forgeguard_core::memory::ARTIFACT_FILE),
    )
    .expect("commit the artifact");

    let report = index(clone.path());

    assert_eq!(
        report.parsed, 0,
        "the artifact carried the symbols; only stat checks ran"
    );
    assert_eq!(report.reused, 4);
    assert!(
        find_symbols(clone.path(), &store(clone.path()), "validateToken", 5)
            .expect("find")
            .len()
            == 1
    );
}

#[test]
fn the_language_server_pass_is_optional_and_never_fails_an_index_run() {
    let directory = fixture();

    let report = index_repository(
        directory.path(),
        &ScanConfig::default(),
        &IndexOptions {
            lsp: Some(LspOptions {
                budget: std::time::Duration::from_secs(2),
                ..LspOptions::default()
            }),
            ..IndexOptions::default()
        },
    )
    .expect("indexing must succeed whether or not a language server is installed");

    let lsp = report.lsp.expect("the pass reports itself when asked for");
    assert!(
        lsp.candidates > 0,
        "the fixture has call sites the static pass cannot attribute"
    );
    // Resolutions depend on what is installed on the machine, so the invariant is
    // the bound, not the count: never more answers than questions.
    assert!(lsp.resolved <= lsp.candidates);
    assert_eq!(report.files, 4, "the graph is complete either way");

    let plain = index(directory.path());
    assert!(
        plain.lsp.is_none(),
        "no language server is contacted unless the caller asks"
    );
}

#[test]
fn the_watcher_reports_only_ticks_that_changed_something() {
    let directory = fixture();
    index(directory.path());
    let mut ticks = Vec::new();

    watch_repository(
        directory.path(),
        &ScanConfig::default(),
        &WatchOptions {
            interval: std::time::Duration::from_millis(1),
            iterations: Some(1),
            base: None,
        },
        |tick| ticks.push(tick.clone()),
    )
    .expect("watch");

    assert_eq!(ticks.len(), 1);
    assert!(!ticks[0].changed, "an unchanged tree parses nothing");
    assert_eq!(ticks[0].report.parsed, 0);
}
