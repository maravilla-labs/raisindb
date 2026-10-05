//! Stage 2: history GC must not change any answer at a retained revision.
//!
//! A history is driven as in stage 1, a random `main` snapshot is pinned with
//! a tag, and `run_history_gc` runs with a random `keep_revisions` cutoff.
//! Retention keeps every version newer than the cutoff, the newest version at
//! or before it, and the versions a tag pins — so every snapshot at or above
//! its branch's cutoff, and the pinned one, must still answer exactly as the
//! model says. Snapshots below the cutoff are no longer asserted: GC is
//! allowed to forget them.

use super::driver::Run;
use super::env::{MAIN, REPO, TENANT};
use super::model::Snapshot;
use raisin_hlc::HLC;
use raisin_rocksdb::management::history_gc::{run_history_gc, GcOptions, HistoryRetention};
use raisin_storage::{Storage, TagRepository};
use std::collections::HashMap;
use std::time::Duration;

/// The tag pin and the per-branch cutoffs of one GC run.
pub struct GcOutcome {
    pub pinned: HLC,
    pub cutoffs: HashMap<String, HLC>,
    pub versions_deleted: u64,
}

impl GcOutcome {
    /// Whether GC promised to keep `s` answerable.
    pub fn retains(&self, s: &Snapshot) -> bool {
        if s.branch == MAIN && s.head == self.pinned {
            return true;
        }
        match self.cutoffs.get(&s.branch) {
            Some(cutoff) => s.head >= *cutoff,
            None => true,
        }
    }
}

/// Pin the `pin`-th `main` snapshot with a tag, then collect with
/// `keep_revisions = keep`.
pub async fn collect(run: &Run, pin: usize, keep: u64) -> GcOutcome {
    let mains: Vec<&Snapshot> = run.snaps.iter().filter(|s| s.branch == MAIN).collect();
    let pinned = mains[pin % mains.len()].head;
    run.env
        .storage
        .tags()
        .create_tag(TENANT, REPO, "oracle-pin", &pinned, "oracle", None, false)
        .await
        .expect("tag");
    let options = GcOptions {
        retention_override: Some(HistoryRetention {
            keep_days: None,
            keep_revisions: Some(keep),
        }),
        tenant: Some(TENANT.to_string()),
        repo: Some(REPO.to_string()),
        min_age: Duration::ZERO,
        collect_orphaned_blobs: false,
        bound_job_results: false,
        sweep_unreferenced_blobs: false,
        compact: false,
        ..GcOptions::default()
    };
    let report = run_history_gc(&run.env.storage, &options).expect("history gc");
    let cutoffs = report
        .branches
        .iter()
        .filter(|b| b.tenant == TENANT && b.repo == REPO)
        .filter_map(|b| {
            let cutoff = b.cutoff.as_ref()?.parse::<HLC>().ok()?;
            Some((b.branch.clone(), cutoff))
        })
        .collect();
    GcOutcome {
        pinned,
        cutoffs,
        versions_deleted: report.versions_deleted,
    }
}
