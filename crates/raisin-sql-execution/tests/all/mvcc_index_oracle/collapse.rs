//! Stage 2b (plan Phase 9): run-collapse GC must not change any answer at ANY
//! revision — `collapse_preserves_every_index_read_at_every_revision`.
//!
//! A history is driven as in stage 1, optionally put through retention GC (as
//! in stage 2, a tag pin plus a random `keep_revisions`), then the repairs
//! collapse requires run on every branch, then `collapse_runs` runs on every
//! branch with the watermark at HEAD. Unlike retention GC, collapse forgets
//! nothing: every snapshot retention kept must still answer exactly as the
//! model says, at its own revision, through every template — so every reader
//! (property, pseudo-property, REFERENCES, ordering, compound, UNIQUE-backed
//! writes afterwards) is checked over collapsed history.

use super::driver::Run;
use super::env::{REPO, TENANT};
use raisin_rocksdb::management::async_indexing::repair::{
    repair_node_id, run_repair, run_repair_blocking, RepairKind, RepairOptions,
};

fn options() -> RepairOptions {
    RepairOptions {
        check_headroom: false,
        max_bytes_per_sec: 0,
        collapse_min_age: std::time::Duration::ZERO,
        ..RepairOptions::default()
    }
}

/// Run the prerequisites and collapse every branch. Returns the versions
/// deleted. Panics when a branch is refused or left incomplete.
pub async fn collapse(run: &Run) -> u64 {
    let storage = &run.env.storage;
    for kind in [RepairKind::OrderedChildren, RepairKind::NodePath] {
        let reports = run_repair(storage, TENANT, REPO, None, kind, options())
            .await
            .expect("prerequisite repair");
        assert!(reports.iter().all(|r| r.completed), "{reports:?}");
    }
    // The blocking body: the oracle's storage is configured without the
    // admin flag, which only gates `run_repair`.
    let reports = run_repair_blocking(
        storage.db(),
        TENANT,
        REPO,
        None,
        RepairKind::CollapseRuns,
        &repair_node_id(storage),
        &options(),
    )
    .expect("collapse");
    for r in &reports {
        assert!(
            r.completed,
            "collapse incomplete on {}: {:?}",
            r.branch, r.collapse
        );
    }
    reports.iter().map(|r| r.writes.written).sum()
}
