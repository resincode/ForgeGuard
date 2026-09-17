//! Background auto-sync.
//!
//! Polling, not filesystem events: indexing already skips a file on a stat
//! match, so a tick over an unchanged tree costs one `stat` per file and no
//! parse. That buys the same freshness as an event watcher without a new
//! dependency or per-platform event plumbing.
// ponytail: polling loop; swap in a filesystem-event crate only if a tick ever
// shows up as measurable cost on a large repository.

use std::{
    path::Path,
    time::{Duration, Instant},
};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::{index_repository, now_seconds, refresh_changed, IndexOptions, IndexReport};
use crate::config::ScanConfig;

#[derive(Debug, Clone)]
pub struct WatchOptions {
    pub interval: Duration,
    /// `None` runs until the process is interrupted.
    pub iterations: Option<usize>,
    /// Git revision to diff against; `None` compares the working tree to HEAD.
    pub base: Option<String>,
}

impl Default for WatchOptions {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(5),
            iterations: None,
            base: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchTick {
    pub at: u64,
    pub changed: bool,
    #[serde(flatten)]
    pub report: IndexReport,
}

/// Re-index on an interval, reporting only the ticks that changed something.
/// The callback decides what to print, so the core stays free of output policy.
pub fn watch_repository(
    root: &Path,
    config: &ScanConfig,
    options: &WatchOptions,
    mut on_tick: impl FnMut(&WatchTick),
) -> Result<()> {
    let mut remaining = options.iterations;
    loop {
        let started = Instant::now();
        let report = sync_once(root, config, options.base.as_deref())?;
        let tick = WatchTick {
            at: now_seconds(),
            changed: report.parsed > 0 || report.removed > 0,
            report,
        };
        on_tick(&tick);

        if let Some(left) = remaining.as_mut() {
            *left = left.saturating_sub(1);
            if *left == 0 {
                return Ok(());
            }
        }
        // Subtract the work already done so the interval is a period, not a gap.
        let elapsed = started.elapsed();
        if elapsed < options.interval {
            std::thread::sleep(options.interval - elapsed);
        }
    }
}

/// One sync. Git scopes the work when it is available; without it the walk
/// falls back to stat checks over the whole tree.
pub fn sync_once(root: &Path, config: &ScanConfig, base: Option<&str>) -> Result<IndexReport> {
    match refresh_changed(root, config, base) {
        Ok(report) => Ok(report),
        Err(_) => index_repository(root, config, &IndexOptions::default()),
    }
}
