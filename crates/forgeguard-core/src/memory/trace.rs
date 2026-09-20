//! Call-chain traversal.
//!
//! One breadth-first walk over the stored call edges answers "who reaches this"
//! and "what does this reach" to a bounded depth, which is the question that
//! otherwise costs an agent a chain of greps.

use std::{
    collections::{BTreeSet, VecDeque},
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::{record_query, store::Store, SymbolKind};

/// Depth is clamped: past five hops a call chain stops being evidence and starts
/// being the whole repository.
pub const MAX_DEPTH: usize = 5;
const MAX_NODES: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    /// Callers, transitively: who reaches this symbol.
    Inbound,
    /// Callees, transitively: what this symbol reaches.
    Outbound,
    Both,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceNode {
    pub qualified: String,
    pub path: PathBuf,
    pub start_line: usize,
    pub kind: SymbolKind,
    pub depth: usize,
    /// Edge direction that reached this node.
    pub direction: Direction,
    /// The symbol one hop closer to the origin.
    pub via: String,
    pub is_test: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceReport {
    pub origin: String,
    pub path: PathBuf,
    pub direction: Direction,
    pub depth: usize,
    pub nodes: Vec<TraceNode>,
    /// The node cap was hit; widen by tracing from a narrower origin.
    pub truncated: bool,
}

/// Breadth-first walk from the first symbol matching `query`.
pub fn trace_path(
    root: &Path,
    store: &Store,
    query: &str,
    direction: Direction,
    depth: usize,
) -> Result<Option<TraceReport>> {
    let started = Instant::now();
    let Some(origin) = store.find_symbols(query, 1)?.into_iter().next() else {
        record_query(root, started);
        return Ok(None);
    };
    let depth = depth.clamp(1, MAX_DEPTH);

    let mut report = TraceReport {
        origin: origin.qualified.clone(),
        path: origin.path.clone(),
        direction,
        depth,
        nodes: Vec::new(),
        truncated: false,
    };
    let mut visited = BTreeSet::from([origin.id]);
    let mut queue = VecDeque::from([(origin, 0_usize, direction)]);

    while let Some((current, current_depth, current_dir)) = queue.pop_front() {
        if current_depth == depth {
            continue;
        }
        // forgeguard: allow FG-ALG-001 -- each node is expanded once; the visited set and MAX_NODES bound the walk
        for (edge, neighbour) in neighbours(store, &current, current_dir)? {
            if !visited.insert(neighbour.id) {
                continue;
            }
            if report.nodes.len() >= MAX_NODES {
                report.truncated = true;
                record_query(root, started);
                return Ok(Some(report));
            }
            report.nodes.push(TraceNode {
                qualified: neighbour.qualified.clone(),
                path: neighbour.path.clone(),
                start_line: neighbour.start_line,
                kind: neighbour.kind,
                depth: current_depth + 1,
                direction: edge,
                via: current.qualified.clone(),
                is_test: neighbour.is_test,
            });
            queue.push_back((neighbour, current_depth + 1, edge));
        }
    }

    record_query(root, started);
    Ok(Some(report))
}

type Edge = (Direction, super::store::SymbolRow);

fn neighbours(
    store: &Store,
    row: &super::store::SymbolRow,
    direction: Direction,
) -> Result<Vec<Edge>> {
    let mut edges = Vec::new();
    if matches!(direction, Direction::Inbound | Direction::Both) {
        for caller in store.callers_of(&row.name, row.container.as_deref())? {
            edges.push((Direction::Inbound, caller));
        }
    }
    if matches!(direction, Direction::Outbound | Direction::Both) {
        for callee in store.callees_of(row.id)? {
            edges.push((Direction::Outbound, callee));
        }
    }
    Ok(edges)
}
