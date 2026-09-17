//! The committed team artifact: a zstd-compressed SQLite snapshot of the graph.
//!
//! Export compacts the database with `VACUUM INTO`, drops every index, and
//! compresses the result. The indexes are pure derived data — `Store::open_at`
//! applies the schema with `CREATE INDEX IF NOT EXISTS` on every open — so
//! shipping them would only make the committed file bigger for no reader.

use std::{
    fs::{self, File},
    io::{BufReader, BufWriter, Read},
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::Store;

/// Path of the compressed artifact, relative to the repository root.
pub const ARTIFACT_FILE: &str = ".forgeguard/memory/graph.db.zst";
/// Uncompressed artifact written by the previous format; still readable.
pub const LEGACY_ARTIFACT_FILE: &str = ".forgeguard/memory/graph.db";

/// Explicit `memory export`: the artifact is written once and read by everyone.
pub const BEST_LEVEL: i32 = 9;
/// Watcher and other low-latency refreshes, where the write is on the hot path.
pub const FAST_LEVEL: i32 = 3;

const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactReport {
    pub path: PathBuf,
    pub raw_bytes: u64,
    pub compressed_bytes: u64,
    pub ratio: f64,
}

/// Compact the store with `VACUUM INTO`, strip indexes, then zstd-compress it.
pub fn export(store: &Store, destination: &Path, level: i32) -> Result<ArtifactReport> {
    let scratch = TempFile::new("export");
    store.export_to(scratch.path())?;
    strip_indexes(scratch.path())?;

    let raw_bytes = fs::metadata(scratch.path())
        .with_context(|| format!("failed to stat {}", scratch.path().display()))?
        .len();

    if let Some(directory) = destination.parent() {
        // forgeguard: allow FG-SEC-007 -- the destination is the repository root joined with a crate constant, or an operator-supplied path
        fs::create_dir_all(directory)
            .with_context(|| format!("failed to create {}", directory.display()))?;
    }
    let reader = BufReader::new(
        File::open(scratch.path())
            .with_context(|| format!("failed to read {}", scratch.path().display()))?,
    );
    let mut writer = BufWriter::new(
        File::create(destination)
            .with_context(|| format!("failed to write {}", destination.display()))?,
    );
    zstd::stream::copy_encode(reader, &mut writer, level)
        .with_context(|| format!("failed to compress {}", destination.display()))?;
    drop(writer);

    let compressed_bytes = fs::metadata(destination)
        .with_context(|| format!("failed to stat {}", destination.display()))?
        .len();
    Ok(ArtifactReport {
        path: destination.to_path_buf(),
        raw_bytes,
        compressed_bytes,
        ratio: raw_bytes as f64 / compressed_bytes.max(1) as f64,
    })
}

/// Open an artifact, compressed or not. The format is decided by magic bytes,
/// because a committed file can be named anything.
pub fn open_artifact(path: &Path) -> Result<Store> {
    match format_of(path)? {
        // Nothing to unpack: read the committed file where it lies, as the
        // uncompressed format always did.
        Format::Sqlite => Store::open_at(path),
        Format::Zstd => {
            let scratch = TempFile::new("import");
            let reader = BufReader::new(
                // forgeguard: allow FG-SEC-007 -- the artifact path is named by the operator or resolved from a crate constant under the repository root
                File::open(path).with_context(|| format!("failed to read {}", path.display()))?,
            );
            let mut writer = BufWriter::new(
                File::create(scratch.path())
                    .with_context(|| format!("failed to write {}", scratch.path().display()))?,
            );
            zstd::stream::copy_decode(reader, &mut writer)
                .with_context(|| format!("failed to decompress {}", path.display()))?;
            drop(writer);
            // ponytail: the graph is copied into memory so the scratch file can
            // be deleted before returning. A repository graph is a few MB; if
            // that stops being true, hand the caller a handle that owns the file.
            let source = Store::open_at(scratch.path())?;
            copy_graph(&source)
        }
    }
}

/// The artifact a repository should read: the compressed one when present.
pub fn resolve(root: &Path) -> Option<PathBuf> {
    [ARTIFACT_FILE, LEGACY_ARTIFACT_FILE]
        .into_iter()
        .map(|name| root.join(name))
        .find(|path| path.is_file())
}

fn copy_graph(source: &Store) -> Result<Store> {
    let mut target = Store::open_memory()?;
    for (path, _) in source.file_paths_and_tests()? {
        if let Some(entry) = source.file_entry(&path)? {
            target.upsert_file(&path, &entry)?;
        }
    }
    for key in ["commit", "last_indexed"] {
        if let Some(value) = source.meta(key)? {
            target.set_meta(key, &value)?;
        }
    }
    Ok(target)
}

/// Drop every index and reclaim the pages they used. The importer rebuilds them.
fn strip_indexes(path: &Path) -> Result<()> {
    let connection =
        Connection::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let names = {
        let mut statement = connection.prepare(
            "SELECT name FROM sqlite_master WHERE type = 'index' AND name NOT LIKE 'sqlite_%'",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.filter_map(Result::ok).collect::<Vec<_>>()
    };
    for name in names {
        // The names come from sqlite_master itself; quoting keeps any identifier valid.
        connection.execute_batch(&format!("DROP INDEX IF EXISTS \"{name}\""))?;
    }
    connection.execute_batch("VACUUM")?;
    Ok(())
}

enum Format {
    Zstd,
    Sqlite,
}

fn format_of(path: &Path) -> Result<Format> {
    let mut header = [0u8; SQLITE_MAGIC.len()];
    let mut file = {
        // forgeguard: allow FG-SEC-007 -- same operator-named artifact path, opened only to read its magic bytes
        File::open(path).with_context(|| format!("failed to read {}", path.display()))?
    };
    let mut filled = 0;
    while filled < header.len() {
        match file.read(&mut header[filled..])? {
            0 => break,
            read => filled += read,
        }
    }
    let header = &header[..filled];
    if header.starts_with(&ZSTD_MAGIC) {
        return Ok(Format::Zstd);
    }
    if header.starts_with(SQLITE_MAGIC) {
        return Ok(Format::Sqlite);
    }
    anyhow::bail!(
        "{} is not a ForgeGuard memory artifact: expected a zstd frame or a SQLite database",
        path.display()
    )
}

/// A scratch file in the system temp directory, removed however the caller
/// leaves — including on error, because `Drop` runs while unwinding.
struct TempFile {
    path: PathBuf,
}

impl TempFile {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        Self {
            path: std::env::temp_dir().join(format!(
                "forgeguard-artifact-{}-{tag}-{unique}.db",
                process::id()
            )),
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        // forgeguard: allow FG-SEC-007 -- a path this type built itself under the system temp directory
        let _ = fs::remove_file(&self.path);
        for suffix in ["-wal", "-shm"] {
            let mut sidecar = self.path.clone().into_os_string();
            sidecar.push(suffix);
            // forgeguard: allow FG-SEC-007 -- the same self-built temp path, plus SQLite's own sidecar suffix
            let _ = fs::remove_file(PathBuf::from(sidecar));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{Call, FileEntry, Symbol, SymbolKind};

    fn entry(index: usize, symbols: usize) -> FileEntry {
        FileEntry {
            language: "rust".to_owned(),
            content_hash: format!("hash{index:04}"),
            symbols_hash: format!("symbols{index:04}"),
            mtime: 1_700_000_000 + index as u64,
            size: 4096,
            is_test: index.is_multiple_of(5),
            last_indexed: 1_700_000_000,
            imports: vec![format!("crate::module{index}"), "std::fs".to_owned()],
            exports: vec![format!("export_{index}")],
            symbols: (0..symbols)
                .map(|nth| Symbol {
                    name: format!("function_{index}_{nth}"),
                    container: Some(format!("Type{index}")),
                    kind: SymbolKind::Method,
                    start_line: nth * 10 + 1,
                    end_line: nth * 10 + 9,
                    signature: format!("fn function_{index}_{nth}(argument: &str) -> Result<()>"),
                    exported: nth.is_multiple_of(2),
                    calls: vec![Call {
                        name: "with_context".to_owned(),
                        ..Call::default()
                    }],
                    extends: Vec::new(),
                    route: None,
                    links: Vec::new(),
                })
                .collect(),
        }
    }

    fn store_with(directory: &Path, files: usize, symbols: usize) -> Store {
        let mut store = Store::open_at(&directory.join("graph.db")).unwrap();
        for index in 0..files {
            store
                .upsert_file(
                    Path::new(&format!("src/file{index}.rs")),
                    &entry(index, symbols),
                )
                .unwrap();
        }
        store
    }

    /// Every test that creates a scratch file shares one temp directory, so the
    /// cleanup assertion would otherwise see another test's file in flight.
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|poison| poison.into_inner())
    }

    fn leftovers() -> Vec<PathBuf> {
        let prefix = format!("forgeguard-artifact-{}-", process::id());
        let Ok(entries) = fs::read_dir(std::env::temp_dir()) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&prefix))
            })
            .collect()
    }

    #[test]
    fn round_trip_preserves_symbols() {
        let _guard = serial();
        let directory = tempfile::tempdir().unwrap();
        let store = store_with(directory.path(), 3, 4);
        let expected = store.all_symbols().unwrap();

        let destination = directory.path().join("out/graph.db.zst");
        let report = export(&store, &destination, BEST_LEVEL).unwrap();
        assert_eq!(report.path, destination);

        let imported = open_artifact(&destination).unwrap();
        let actual = imported.all_symbols().unwrap();
        assert_eq!(expected.len(), actual.len());
        for (left, right) in expected.iter().zip(actual.iter()) {
            assert_eq!(left.qualified, right.qualified);
            assert_eq!(left.signature, right.signature);
            assert_eq!(left.path, right.path);
            assert_eq!(left.start_line, right.start_line);
            assert_eq!(left.kind, right.kind);
        }
        assert_eq!(
            imported.file_entry(Path::new("src/file1.rs")).unwrap(),
            store.file_entry(Path::new("src/file1.rs")).unwrap()
        );
    }

    #[test]
    fn reads_a_plain_sqlite_artifact() {
        let directory = tempfile::tempdir().unwrap();
        let store = store_with(directory.path(), 2, 2);
        let legacy = directory.path().join("legacy/graph.db");
        store.export_to(&legacy).unwrap();

        let imported = open_artifact(&legacy).unwrap();
        assert_eq!(imported.counts().unwrap(), store.counts().unwrap());
    }

    #[test]
    fn rejects_an_unknown_format() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("garbage.bin");
        fs::write(&path, b"this is neither compressed nor a database").unwrap();

        let Err(failure) = open_artifact(&path) else {
            panic!("a non-artifact file must be rejected");
        };
        let error = failure.to_string();
        assert!(error.contains("zstd"), "{error}");
        assert!(error.contains("SQLite"), "{error}");
    }

    #[test]
    fn compression_shrinks_the_database() {
        let _guard = serial();
        let directory = tempfile::tempdir().unwrap();
        let store = store_with(directory.path(), 40, 10);
        assert_eq!(store.counts().unwrap().1, 400);

        let destination = directory.path().join("graph.db.zst");
        let report = export(&store, &destination, BEST_LEVEL).unwrap();
        assert!(
            report.compressed_bytes < report.raw_bytes,
            "{} >= {}",
            report.compressed_bytes,
            report.raw_bytes
        );
        assert!(report.ratio > 1.0);
        println!(
            "raw {} bytes, compressed {} bytes, ratio {:.1}:1",
            report.raw_bytes, report.compressed_bytes, report.ratio
        );
    }

    #[test]
    fn temporary_files_are_removed() {
        let _guard = serial();
        let directory = tempfile::tempdir().unwrap();
        let store = store_with(directory.path(), 2, 2);
        let destination = directory.path().join("graph.db.zst");
        export(&store, &destination, FAST_LEVEL).unwrap();
        assert_eq!(leftovers(), Vec::<PathBuf>::new());

        // A well-formed zstd frame wrapping something that is not a database:
        // the scratch file is created and the import then fails.
        let broken = directory.path().join("broken.db.zst");
        fs::write(
            &broken,
            zstd::encode_all(&b"not a database"[..], FAST_LEVEL).unwrap(),
        )
        .unwrap();
        assert!(open_artifact(&broken).is_err());
        assert_eq!(leftovers(), Vec::<PathBuf>::new());
    }

    #[test]
    fn resolve_prefers_the_compressed_artifact() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        assert_eq!(resolve(root), None);

        let legacy = root.join(LEGACY_ARTIFACT_FILE);
        fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        fs::write(&legacy, b"").unwrap();
        assert_eq!(resolve(root), Some(legacy));

        let compressed = root.join(ARTIFACT_FILE);
        fs::write(&compressed, b"").unwrap();
        assert_eq!(resolve(root), Some(compressed));
    }
}
