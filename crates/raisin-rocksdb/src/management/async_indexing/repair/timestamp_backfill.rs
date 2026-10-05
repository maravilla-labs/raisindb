//! The `timestamp_backfill` repair (plan Phase 13g, owner request of
//! 2026-10-05): give every live node whose NEWEST version has no `created_at`
//! / `updated_at` the timestamps its history implies, so the compound indexes
//! whose ORDER column is a system timestamp — the built-in
//! `@__children_by_created_at` on every workspace above all — can be built.
//!
//! - **What it writes.** `created_at` = the physical time of the node's FIRST
//!   stored revision on the branch (the oldest retained one when history GC
//!   removed the first); `updated_at` = the newest revision's. Only a missing
//!   field is set; no other field or property changes.
//! - **Through the one write funnel** (`NodeRepositoryImpl::backfill_timestamps`
//!   → `update_impl_as`): rewritten IN PLACE at the read version's revision
//!   (so a racing or not-yet-applied edit, which is always above it, wins),
//!   conditionally (a version written since the read: nothing written, the
//!   next run retries), as the system actor, every derived index —
//!   property, compound (type, workspace and built-in), unique, reference,
//!   spatial, localized name — maintained by the writers every edit uses, and
//!   one `ApplyRevision` captured for replication, so a replica receives the
//!   rewritten version like any write. The repository layer publishes no node
//!   event, so no trigger, webhook or subscription fires for legacy content.
//! - **Idempotent, data-detected.** The scan picks nodes from the data; the
//!   funnel re-reads the node and writes nothing when it has both fields (or
//!   was deleted meanwhile). Deleted nodes are skipped.
//! - **Streaming, bounded, resumable, paced.** Chunks of at most
//!   [`CHUNK_NODES`] nodes (and `batch_bytes` of newest versions) read on a
//!   blocking thread; the per-node state record
//!   (`repair_state\0timestamp_backfill`) holds the last finished node's key
//!   after every chunk; writes are paced to `max_bytes_per_sec` (estimated
//!   from the blobs rewritten); each chunk checks the volume can take its
//!   output first.
//! - When a branch completes having written something, its `compound_builds`
//!   link is re-requested, so the indexes refused for these nodes build
//!   without waiting for a restart.
//! - A `queued` mark written over the state record DURING a run (a checkpoint
//!   ingest) wins: the run scans the branch again from the start instead of
//!   saving `done` over it.

use super::branches::list_branches;
use super::cursor::{load_state, state_key, RepairState};
use super::timestamp_scan::{scan_chunk, Candidate};
use super::{check_output_headroom, repair_node_id, RepairKind, RepairOptions, RepairReport};
use crate::RocksDBStorage;
use raisin_error::{Error, Result};
use serde::{Deserialize, Serialize};

/// Nodes read per chunk (between commits of the cursor).
pub(super) const CHUNK_NODES: usize = 256;

/// Free space a chunk must leave on the data volume after its writes.
const FREE_FLOOR: u64 = 256 * 1024 * 1024;

/// Index and revision bytes a rewritten version writes per blob byte (a
/// rough multiplier for pacing and the headroom check).
const WRITE_AMPLIFICATION: u64 = 4;

const PASS: &str = "timestamps";

/// What one run did on one branch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimestampBackfillCounts {
    /// Live nodes seen (newest version not a delete).
    pub nodes: u64,
    /// Of those: newest version without `created_at` and/or `updated_at`.
    pub missing: u64,
    /// Versions written (in a dry run: would write).
    pub backfilled: u64,
    /// Found complete (or deleted) when the funnel re-read them, or written
    /// since its read (the next run retries those): nothing written.
    pub unchanged: u64,
    /// Newest versions that could not be decoded (skipped, logged).
    pub undecodable: u64,
    /// Writes the funnel refused (logged; the branch saves `failed` and is
    /// retried at the next start).
    pub failed: u64,
}

/// Run the backfill on one branch, or every branch of the repository.
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
        reports.push(backfill_branch(storage, tenant_id, repo_id, &branch, options).await?);
    }
    Ok(reports)
}

async fn backfill_branch(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    options: &RepairOptions,
) -> Result<RepairReport> {
    let db = storage.db().clone();
    let node_id = repair_node_id(storage);
    let slug = RepairKind::TimestampBackfill.slug();
    let key = state_key(tenant_id, repo_id, branch, slug, &node_id);
    let previous = load_state(&db, tenant_id, repo_id, branch, slug, &node_id)?;
    let resumed = !options.dry_run
        && previous
            .as_ref()
            .is_some_and(|s| s.status == "running" && s.cursor.is_some());
    let seen_at_start = previous.as_ref().map(|s| s.updated_at.clone());
    let mut state = match previous.filter(|_| resumed) {
        Some(state) => state,
        None => RepairState {
            pass: PASS.to_string(),
            ..RepairState::default()
        },
    };
    let mut report = RepairReport {
        branch: branch.to_string(),
        repair: slug.to_string(),
        dry_run: options.dry_run,
        resumed,
        ..RepairReport::default()
    };
    let mut after: Option<Vec<u8>> = state.cursor.as_deref().and_then(|h| hex::decode(h).ok());
    // The record as this run last saw it: a `queued` mark written over it
    // since (a checkpoint ingest's `mark_repairs_pending`) means NODES the run
    // already passed may hold new legacy versions — the mark wins and the
    // scan starts over, instead of this run saving `done` over it.
    let mut last_seen = seen_at_start;
    let started = std::time::Instant::now();
    let mut paced_bytes: u64 = 0;
    let mut chunks = 0usize;
    let completed = loop {
        let scope = (
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
        );
        let (db_scan, from) = (db.clone(), after.clone());
        let bounds = (CHUNK_NODES, options.batch_bytes as u64);
        let chunk = tokio::task::spawn_blocking(move || {
            scan_chunk(&db_scan, &scope, from.as_deref(), bounds)
        })
        .await
        .map_err(|e| Error::storage(format!("timestamp backfill scan failed: {e}")))??;
        let Some(last) = chunk.last_group.clone() else {
            if !options.dry_run && marked_since(&db, &key, &mut last_seen)? {
                after = None;
                continue;
            }
            break true;
        };
        let counts = &mut report.timestamps;
        counts.nodes += chunk.live;
        counts.undecodable += chunk.undecodable;
        counts.missing += chunk.candidates.len() as u64;
        let estimate: u64 = chunk
            .candidates
            .iter()
            .map(|c| c.blob_bytes * WRITE_AMPLIFICATION)
            .sum();
        if options.dry_run {
            counts.backfilled += chunk.candidates.len() as u64;
        } else {
            if options.check_headroom && estimate > 0 {
                check_output_headroom(&db, estimate, FREE_FLOOR, options.free_bytes_override)?;
            }
            for candidate in &chunk.candidates {
                write_one(storage, (tenant_id, repo_id, branch), candidate, counts).await;
            }
            if marked_since(&db, &key, &mut last_seen)? {
                after = None;
                continue;
            }
            state.status = "running".to_string();
            state.cursor = Some(hex::encode(&last));
            state.written = counts.backfilled;
            save(&db, &key, &mut state)?;
            last_seen = Some(state.updated_at.clone());
        }
        report.writes.written += chunk.candidates.len() as u64;
        report.writes.bytes += estimate;
        report.writes.batches += 1;
        chunks += 1;
        paced_bytes += estimate + chunk.bytes_read;
        pace(options.max_bytes_per_sec, started, paced_bytes).await;
        if options
            .stop_after_batches
            .is_some_and(|limit| chunks >= limit)
        {
            break false;
        }
        after = Some(last);
    };
    report.completed = completed;
    if completed && !options.dry_run {
        let failed = report.timestamps.failed > 0;
        state.status = if failed { "failed" } else { "done" }.to_string();
        state.cursor = None;
        state.written = report.timestamps.backfilled;
        save(&db, &key, &mut state)?;
        // The indexes refused for these nodes can be built now: ask for the
        // branch's `compound_builds` link past the refusal's owed-work
        // fingerprint. Only when the run wrote something — a run that changed
        // no node changes no build's outcome, and must not re-run a link
        // that FAILED (no headroom, unplaceable nodes) on every start.
        if report.timestamps.backfilled > 0 {
            if let Err(e) =
                super::auto_compound::request_after_backfill(storage, tenant_id, repo_id, branch)
                    .await
            {
                tracing::warn!(tenant_id, repo_id, branch, error = %e, "timestamp_backfill: could not re-request compound builds");
            }
        }
    }
    tracing::info!(
        tenant_id,
        repo_id,
        branch,
        dry_run = options.dry_run,
        completed,
        nodes = report.timestamps.nodes,
        missing = report.timestamps.missing,
        backfilled = report.timestamps.backfilled,
        failed = report.timestamps.failed,
        "timestamp_backfill finished a branch"
    );
    Ok(report)
}

/// One node through the write funnel. A refused write is logged and counted,
/// never fatal: the other nodes still get theirs.
async fn write_one(
    storage: &RocksDBStorage,
    (tenant_id, repo_id, branch): (&str, &str, &str),
    candidate: &Candidate,
    counts: &mut TimestampBackfillCounts,
) {
    match storage
        .nodes_impl()
        .backfill_timestamps(
            tenant_id,
            repo_id,
            branch,
            &candidate.workspace,
            &candidate.node_id,
            candidate.created_at,
            candidate.updated_at,
        )
        .await
    {
        Ok(true) => counts.backfilled += 1,
        Ok(false) => counts.unchanged += 1,
        Err(e) => {
            counts.failed += 1;
            tracing::warn!(
                tenant_id,
                repo_id,
                branch,
                workspace = %candidate.workspace,
                node_id = %candidate.node_id,
                error = %e,
                "timestamp_backfill: the write funnel refused a node"
            );
        }
    }
}

/// Whether the state record was marked `queued` by someone else since this
/// run last saw it (`last_seen`, updated to what it reads).
fn marked_since(db: &rocksdb::DB, key: &[u8], last_seen: &mut Option<String>) -> Result<bool> {
    let cf = crate::cf_handle(db, crate::cf::INDEX_STATUS)?;
    let stored = db
        .get_cf(cf, key)
        .map_err(|e| Error::storage(e.to_string()))?
        .and_then(|bytes| serde_json::from_slice::<RepairState>(&bytes).ok());
    let Some(stored) = stored else {
        return Ok(false);
    };
    let marked = stored.status == "queued" && last_seen.as_deref() != Some(&stored.updated_at);
    if marked {
        tracing::info!(
            "timestamp_backfill: the branch was marked pending during the run; scanning it again"
        );
        *last_seen = Some(stored.updated_at);
    }
    Ok(marked)
}

fn save(db: &rocksdb::DB, key: &[u8], state: &mut RepairState) -> Result<()> {
    state.pass = PASS.to_string();
    state.updated_at = chrono::Utc::now().to_rfc3339();
    let bytes = serde_json::to_vec(state)
        .map_err(|e| Error::storage(format!("repair state encode: {e}")))?;
    db.put_cf(crate::cf_handle(db, crate::cf::INDEX_STATUS)?, key, bytes)
        .map_err(|e| Error::storage(e.to_string()))
}

/// Hold the average rate at `max_bytes_per_sec` (0: unlimited), asleep on
/// the runtime — never a blocked worker thread.
async fn pace(max_bytes_per_sec: u64, started: std::time::Instant, bytes: u64) {
    if max_bytes_per_sec == 0 {
        return;
    }
    let due = std::time::Duration::from_secs_f64(bytes as f64 / max_bytes_per_sec as f64);
    let elapsed = started.elapsed();
    if due > elapsed {
        tokio::time::sleep(due - elapsed).await;
    }
}
