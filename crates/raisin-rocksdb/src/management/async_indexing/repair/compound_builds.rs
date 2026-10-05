//! The `compound_builds` repair: compound index builds that run by
//! themselves, per branch, in the background (plan Phase 13f, owner
//! decisions of 2026-10-05).
//!
//! It owns three kinds of work, all found from the DATA (`compound_detect`):
//!
//! - the BUILT-IN workspace indexes (`@__children_by_created_at`, on every
//!   workspace unless its config opts out) that are not `Ready` on this node —
//!   after the upgrade that ships them, on a new workspace, after a switch
//!   back on;
//! - indexes whose state record is an older FORMAT
//!   (`CompoundIndexState::is_format_upgrade`), while
//!   `RAISIN_COMPOUND_FORMAT_REBUILD` is not `0` — the format rebuild that was
//!   admin-only before;
//! - workspace indexes no longer declared (removed, or a built-in switched
//!   off): their state record and entries are DROPPED.
//!
//! A link also rebuilds every other declared index of the branch that is not
//! `Ready` (the safety net after a checkpoint ingest, which marks every record
//! `NotBuilt`); those do not make a branch pending by themselves — the
//! per-index build jobs of the sweep own them.
//!
//! Discipline, as for every automatic repair: the chain after start
//! (`auto_node_path::schedule_chain`, one branch at a time, the next link
//! queued when one ends), a targeted link on request (a workspace or schema
//! event, a cold drain), re-marked pending and restarted after a checkpoint
//! ingest; each build prechecks disk headroom (2x COMPOUND_INDEX) and refuses
//! unplaceable nodes before clearing, writes paced (`max_bytes_per_sec`) on a
//! blocking thread, and holds the keyspace lock so it queues behind (and then
//! skips) a per-index job building the same index. Resumable at INDEX
//! granularity: a built index is `Ready` and is skipped by the next run; an
//! interrupted one is rebuilt from scratch (the build is clear-then-rewrite).
//! Progress lives in the per-node state record (`repair_state\0compound_builds`),
//! its cursor naming the last finished `{workspace}\0{index}`.

use super::compound_detect::{branch_work, work_fingerprint, Work};
use super::{list_branches, repair_node_id, state_key, RepairKind, RepairOptions, RepairReport};
use crate::RocksDBStorage;
use raisin_error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

/// After the other automatic chains' first links.
pub const START_DELAY: Duration = Duration::from_secs(120);

/// What one link did on one branch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompoundBuildCounts {
    /// Indexes built (stamped `Ready`).
    pub built: u64,
    /// Indexes found `Ready` when their turn came (built meanwhile).
    pub already_ready: u64,
    /// Undeclared workspace indexes dropped.
    pub dropped: u64,
    /// Entries those drops deleted.
    pub dropped_entries: u64,
    /// Builds that failed (no headroom, unplaceable nodes, …); retried at the
    /// next start.
    pub failed: u64,
    /// Node-type indexes skipped: no type declares them any more.
    pub undeclared: u64,
    /// Builds refused because nodes have no value for the index's ORDER
    /// column (legacy versions without `created_at`/`updated_at`): an
    /// expected state, not a failure (plan Phase 13g). The
    /// `timestamp_backfill` repair resolves it and re-requests the link.
    #[serde(default)]
    pub refused: u64,
    /// The nodes those refusals counted.
    #[serde(default)]
    pub refused_missing_order_values: u64,
}

/// The status a link that ended with refused builds (and no failure) saves:
/// pending at the next start, and not re-linked by a targeted request while
/// the owed work is unchanged — `timestamp_backfill` re-requests it.
pub const REFUSED_STATUS: &str = "refused_missing_order_values";

/// Run the repair on one branch, or every branch of the repository.
pub(super) async fn run(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: Option<&str>,
    options: &RepairOptions,
) -> Result<Vec<RepairReport>> {
    let branches = match branch {
        Some(branch) => vec![branch.to_string()],
        None => list_branches(storage.db(), tenant_id, repo_id)?,
    };
    let mut reports = Vec::new();
    for branch in branches {
        reports.push(link(storage, tenant_id, repo_id, &branch, options).await?);
    }
    Ok(reports)
}

async fn link(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    options: &RepairOptions,
) -> Result<RepairReport> {
    let db = storage.db();
    let node_id = repair_node_id(storage);
    let key = state_key(
        tenant_id,
        repo_id,
        branch,
        RepairKind::CompoundBuilds.slug(),
        &node_id,
    );
    let work = branch_work(db, tenant_id, repo_id, branch, true)?;
    let mut report = RepairReport {
        branch: branch.to_string(),
        repair: RepairKind::CompoundBuilds.slug().to_string(),
        dry_run: options.dry_run,
        ..RepairReport::default()
    };
    if options.dry_run {
        report.compound.built = work
            .iter()
            .filter(|w| matches!(w, Work::Build { .. }))
            .count() as u64;
        report.compound.dropped = work.len() as u64 - report.compound.built;
        return Ok(report);
    }
    save(db, &key, "running", None, 0, None, None)?;
    let handler = crate::storage::create_compound_index_handler(storage, None);
    let mut first_error: Option<Error> = None;
    let mut done = 0u64;
    // Keys this link gave up on (failed, or a type index no type declares any
    // more): never retried within it.
    let mut given_up: HashSet<String> = HashSet::new();
    let mut work = work;
    let mut round = 1;
    let still_owed = loop {
        for item in &work {
            let (undeclared, refused) = (report.compound.undeclared, report.compound.refused);
            let ctx = (tenant_id, repo_id, branch);
            match super::compound_items::run_item(
                storage,
                &handler,
                ctx,
                item,
                options,
                &mut report,
            )
            .await
            {
                Err(e) => {
                    tracing::warn!(tenant_id, repo_id, branch, item = ?item, error = %e, "compound_builds: work item failed");
                    report.compound.failed += 1;
                    first_error.get_or_insert(e);
                    given_up.insert(item.key());
                }
                // Undeclared, or refused for missing order-column values:
                // re-checking it within this link would only refuse again.
                Ok(())
                    if report.compound.undeclared > undeclared
                        || report.compound.refused > refused =>
                {
                    given_up.insert(item.key());
                }
                Ok(()) => {}
            }
            done += 1;
            save(db, &key, "running", Some(&item.key()), done, None, None)?;
        }
        // Re-read the work: a request that found this link live queued
        // nothing (a workspace created or switched meanwhile), and a
        // checkpoint ingest meanwhile marked what this link already built.
        let again: Vec<Work> = branch_work(db, tenant_id, repo_id, branch, true)?
            .into_iter()
            .filter(|item| !given_up.contains(&item.key()))
            .collect();
        if again.is_empty() || round == RECHECK_ROUNDS {
            break again;
        }
        round += 1;
        work = again;
    };
    report.completed = first_error.is_none();
    // A failed link records the work the branch still owes, so a targeted
    // request does not re-run it until that changes (`auto_compound`).
    let refused = report.compound.refused > 0;
    // A refused link records the owed work as a failed one does: a targeted
    // request does not re-run it until that changes (`auto_compound`), and
    // the backfill that resolves it re-requests it explicitly.
    let failed_on = if report.completed && !refused {
        None
    } else {
        Some(work_fingerprint(&branch_work(
            db, tenant_id, repo_id, branch, false,
        )?))
    };
    // Work left after the last re-check keeps the branch pending (`queued`),
    // never `done` over work it did not do.
    let status = match (report.completed, refused, still_owed.is_empty()) {
        (false, _, _) => "failed",
        (true, true, _) => REFUSED_STATUS,
        (true, false, true) => "done",
        (true, false, false) => "queued",
    };
    let refused_nodes = refused.then_some(report.compound.refused_missing_order_values);
    save(db, &key, status, None, done, failed_on, refused_nodes)?;
    tracing::info!(
        tenant_id,
        repo_id,
        branch,
        built = report.compound.built,
        dropped = report.compound.dropped,
        failed = report.compound.failed,
        refused = report.compound.refused,
        refused_missing_order_values = report.compound.refused_missing_order_values,
        "compound_builds finished a branch"
    );
    match first_error {
        Some(e) => Err(e),
        None => Ok(report),
    }
}

/// How many times one link re-reads its work before leaving the rest to the
/// next run (`queued`).
const RECHECK_ROUNDS: usize = 3;

fn save(
    db: &rocksdb::DB,
    key: &[u8],
    status: &str,
    cursor: Option<&str>,
    written: u64,
    failed_on: Option<String>,
    refused_missing_order_values: Option<u64>,
) -> Result<()> {
    let state = super::RepairState {
        status: status.to_string(),
        pass: "indexes".to_string(),
        cursor: cursor.map(|c| hex::encode(c.as_bytes())),
        written,
        updated_at: chrono::Utc::now().to_rfc3339(),
        epoch: failed_on,
        refused_missing_order_values,
    };
    let bytes = serde_json::to_vec(&state)
        .map_err(|e| Error::storage(format!("repair state encode: {e}")))?;
    db.put_cf(crate::cf_handle(db, crate::cf::INDEX_STATUS)?, key, bytes)
        .map_err(|e| Error::storage(e.to_string()))
}
