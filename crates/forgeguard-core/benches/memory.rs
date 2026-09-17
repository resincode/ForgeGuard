//! Old workflow versus code memory on three realistic lookups.
//!
//! Old: read each file that holds the symbol, whole.
//! New: index once, then answer from the graph and return one snippet.

use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

use forgeguard_core::{
    config::ScanConfig,
    memory::{
        find_symbols, index_repository, symbol_card, Detail, IndexOptions, RetrievalOptions, Store,
    },
};

/// Files a "find the symbol, read the file" workflow would pull in whole.
const SOURCES: &[&str] = &["scanner.rs", "git.rs", "hook.rs", "init.rs", "runner.rs"];
const LOOKUPS: &[&str] = &["scan_project", "changed_scope", "evaluate_stop_hook"];

fn main() {
    let source_directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let root = std::env::temp_dir().join(format!("forgeguard-memory-bench-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("src")).expect("create benchmark directory");
    let mut baseline_bytes = 0;
    for name in SOURCES {
        let contents = fs::read_to_string(source_directory.join(name)).expect("read source");
        baseline_bytes += contents.len();
        fs::write(root.join("src").join(name), contents).expect("write benchmark source");
    }

    let started = Instant::now();
    let report = index_repository(&root, &ScanConfig::default(), &IndexOptions::default())
        .expect("index benchmark repository");
    let index_millis = started.elapsed().as_millis();
    let store = Store::open(&root).expect("open store");

    println!(
        "indexed {} files, {} symbols in {index_millis} ms",
        report.files, report.symbols
    );
    println!(
        "old workflow: {baseline_bytes} bytes read for {} lookups",
        LOOKUPS.len()
    );

    for detail in [Detail::Metadata, Detail::Structure, Detail::Snippet] {
        let started = Instant::now();
        let bytes = measure(&root, &store, detail);
        let micros = started.elapsed().as_micros() / LOOKUPS.len() as u128;
        let saved = 100.0 - (bytes as f64 / baseline_bytes as f64) * 100.0;
        println!(
            "{:>9?}: {bytes:>7} bytes returned, {saved:>5.1}% less than reading the files, {micros} us per query",
            detail
        );
    }

    fs::remove_dir_all(&root).expect("remove benchmark directory");
}

/// Bytes an agent would actually receive: the serialized answer, not just source.
fn measure(root: &Path, store: &Store, detail: Detail) -> usize {
    let mut bytes = 0;
    for query in LOOKUPS {
        let hits = find_symbols(root, store, query, 5).expect("find symbols");
        bytes += serde_json::to_string(&hits).expect("serialize hits").len();
        if detail == Detail::Metadata {
            continue;
        }
        let card = symbol_card(
            root,
            store,
            query,
            &RetrievalOptions {
                detail,
                max_bytes: 16 * 1024,
            },
        )
        .expect("retrieve symbol")
        .expect("symbol is indexed");
        bytes += serde_json::to_string(&card).expect("serialize card").len();
    }
    bytes
}
