//! Registry of indexed repositories.
//!
//! Each repository owns its graph inside its own `.forgeguard/cache/memory`, so
//! the registry only records where those graphs are. That keeps one agent able
//! to ask "what have I indexed" without a shared database that two checkouts of
//! the same repository would fight over.

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::{now_seconds, store::Store, MEMORY_DIR};

const REGISTRY_FILE: &str = ".forgeguard/memory-projects.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectEntry {
    /// Directory name of the repository root: what a caller names in a query.
    pub name: String,
    pub root: PathBuf,
    pub files: usize,
    pub symbols: usize,
    pub last_indexed: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Registry {
    #[serde(default)]
    projects: Vec<ProjectEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexStatus {
    pub root: PathBuf,
    pub indexed: bool,
    pub files: usize,
    pub symbols: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_commit: Option<String>,
    /// The index was built against a different commit than the one checked out.
    pub stale: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_indexed: Option<u64>,
    pub database: PathBuf,
}

/// Record (or refresh) a repository in the registry after an index run.
pub fn register_project(root: &Path, files: usize, symbols: usize) -> Result<()> {
    let Some(path) = registry_path() else {
        return Ok(());
    };
    let mut registry = load(&path);
    let root = root.to_path_buf();
    let entry = ProjectEntry {
        name: project_name(&root),
        root: root.clone(),
        files,
        symbols,
        last_indexed: now_seconds(),
    };
    match registry
        .projects
        .iter_mut()
        .find(|project| project.root == root)
    {
        Some(existing) => *existing = entry,
        None => registry.projects.push(entry),
    }
    registry
        .projects
        .sort_by(|left, right| left.root.cmp(&right.root));
    save(&path, &registry)
}

/// Registered repositories whose graph still exists on disk.
pub fn list_projects() -> Result<Vec<ProjectEntry>> {
    let Some(path) = registry_path() else {
        return Ok(Vec::new());
    };
    Ok(load(&path)
        .projects
        .into_iter()
        .filter(|project| project.root.join(MEMORY_DIR).is_dir())
        .collect())
}

/// Drop a repository's graph and its registry entry. Source is never touched.
pub fn delete_project(root: &Path) -> Result<bool> {
    let directory = root.join(MEMORY_DIR);
    let removed = directory.is_dir();
    if removed {
        fs::remove_dir_all(&directory)
            .with_context(|| format!("failed to remove {}", directory.display()))?;
    }
    if let Some(path) = registry_path() {
        let mut registry = load(&path);
        registry.projects.retain(|project| project.root != root);
        save(&path, &registry)?;
    }
    Ok(removed)
}

/// Whether a repository has a usable graph, and whether it matches HEAD.
pub fn index_status(root: &Path) -> Result<IndexStatus> {
    let database = root.join(super::DATABASE_FILE);
    if !database.is_file() {
        return Ok(IndexStatus {
            root: root.to_path_buf(),
            indexed: false,
            files: 0,
            symbols: 0,
            commit: None,
            head_commit: crate::git::head_commit(root).unwrap_or(None),
            stale: true,
            last_indexed: None,
            database,
        });
    }
    let store = Store::open(root)?;
    let (files, symbols) = store.counts()?;
    let commit = store.meta("commit")?;
    let head_commit = crate::git::head_commit(root).unwrap_or(None);
    Ok(IndexStatus {
        root: root.to_path_buf(),
        indexed: files > 0,
        files,
        symbols,
        stale: match (&commit, &head_commit) {
            (Some(indexed), Some(head)) => indexed != head,
            // Without Git there is no commit to compare, so freshness is decided
            // per file at index time, not here.
            _ => false,
        },
        last_indexed: store
            .meta("last_indexed")?
            .and_then(|value| value.parse().ok()),
        commit,
        head_commit,
        database,
    })
}

fn project_name(root: &Path) -> String {
    root.file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("repository")
        .to_owned()
}

fn registry_path() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(|home| PathBuf::from(home).join(REGISTRY_FILE))
}

fn load(path: &Path) -> Registry {
    // forgeguard: allow FG-SEC-007 -- the user's home directory joined with a crate constant
    fs::read_to_string(path)
        .ok()
        .and_then(|source| serde_json::from_str(&source).ok())
        .unwrap_or_default()
}

fn save(path: &Path, registry: &Registry) -> Result<()> {
    let directory = path.parent().context("registry path has no parent")?;
    fs::create_dir_all(directory)
        .with_context(|| format!("failed to create {}", directory.display()))?;
    fs::write(path, serde_json::to_vec_pretty(registry)?)
        .with_context(|| format!("failed to write {}", path.display()))
}
