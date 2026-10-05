//! Streaming, resumable, admin-triggered repairs of derived index data.
//!
//! Every repair here follows the plan's repair discipline:
//!
//! - **Cleanup, never the fix.** The readers already tolerate the damage; a
//!   repair only removes it. A checkpoint ingest from an unrepaired peer can
//!   bring it back, so the ingest hook re-enqueues the repairs.
//! - **Data-detected and idempotent.** Targets are found by inspecting the
//!   data, never from a "done" flag, so a re-run after a crash or an ingest is
//!   always correct and a run over clean data writes nothing.
//! - **Streaming and bounded.** One CF of one branch is iterated, and writes
//!   are committed in bounded batches (default 8 MB), each together with the
//!   per-node state record holding the resume cursor (`cursor.rs`).
//! - **Disk precheck.** A run refuses to start without free space of at least
//!   twice the affected CF on the data volume. A dry run reports what it would
//!   write and writes nothing.
//! - **Admin-triggered, never at boot.** Each node repairs its own data; the
//!   per-node state record lets an operator see which nodes have not. The one
//!   exception is the `node_path` backfill, which queues itself (ordinary
//!   background jobs, after boot, one branch at a time) on every branch where
//!   it has not completed — see `auto_node_path.rs` (plan Phase 10b) — and
//!   the chains built on it: `localized_names`, `property_index` and
//!   `block_overlay_tombstones` (`auto_block_overlays.rs`, plan Phase 11c)
//!   and `compound_builds` (`compound_builds.rs`, plan Phase 13f).
//!
//! [`run_repair`] is the entry point the job handler and the admin endpoint
//! call.
//!
//! Run-collapse GC (plan Phase 9) runs through here too, as the
//! `collapse_runs` kind: the same bounded writer, cursor, throttle, state
//! record and fan-out, but it DELETES (`management::history_gc::collapse`).
//! Every repair that inserts entries holds the `(branch, CF)` exclusion
//! against it while it runs (`management::cf_exclusion`).

mod auto_block_overlays;
mod auto_compound;
mod auto_node_path;
mod auto_property_index;
mod auto_targets;
mod block_overlay_tombstones;
mod branches;
mod compound_builds;
mod compound_detect;
mod compound_items;
mod cursor;
mod enqueue;
mod headroom;
mod kind;
mod node_path_backfill;
mod node_path_stage;
mod options;
mod ordered_children;
mod ordered_pass;
mod parent_entries;
mod path_tombstone;
mod property_index;
mod property_state;
mod requests;
mod translation_resync;
mod translation_resync_scan;

pub use auto_block_overlays::{
    auto_enabled as block_overlay_auto_enabled, pending_branches as pending_block_overlay_branches,
    schedule_after_start as schedule_block_overlay_tombstones, BLOCK_OVERLAY_AUTO_ENV,
};
pub use auto_compound::{
    enqueue_if_owed as enqueue_compound_builds_if_owed, request as request_compound_builds,
    restart_after_ingest as restart_compound_builds_after_ingest,
    schedule_after_start as schedule_compound_builds,
};
pub use auto_node_path::{
    auto_backfill_enabled, continue_chain, continue_node_path_backfill_chain,
    enqueue_pending_node_path_backfills, pending_node_path_branches, schedule_chain,
    schedule_node_path_backfill, start_chain, AUTO_BACKFILL_DELAY, AUTO_CHAIN_META,
    NODE_PATH_AUTO_BACKFILL_ENV,
};
pub use auto_property_index::{
    auto_rebuild_enabled as property_index_auto_rebuild_enabled, pending_property_index_branches,
    request_rebuild as request_property_index_rebuild,
    schedule_after_start as schedule_property_index_rebuild, PROPERTY_INDEX_AUTO_REBUILD_ENV,
};
pub use auto_targets::{after_link, enqueue_branch, record_link_outcome};
pub use block_overlay_tombstones::BlockOverlayCounts;
pub use compound_builds::{CompoundBuildCounts, START_DELAY as COMPOUND_BUILDS_START_DELAY};
pub use compound_detect::pending_branches as pending_compound_build_branches;
pub(crate) use cursor::BoundedWriter;
pub use cursor::{load_state, state_key, BatchReport, CommitHook, RepairState};
pub(crate) use enqueue::list_repositories;
pub use enqueue::{
    enqueue_index_repair, reenqueue_repairs_after_checkpoint, reenqueue_repairs_after_ingest,
};
pub use enqueue::{mark_repairs_pending, mark_repairs_pending_on};
pub use headroom::{check_headroom, check_headroom_assuming, check_output_headroom};
pub use kind::RepairKind;
pub use node_path_backfill::NodePathCounts;
pub use options::{RepairOptions, RepairReport};
pub(crate) use ordered_children::iterate_from;
pub use ordered_children::OrderedChildrenCounts;
pub use property_index::PropertyIndexCounts;
pub use property_state::{
    forget_branch_rebuild_state, invalidate_after_branch_copy, invalidate_property_index_rebuild,
    invalidate_rebuilds_for_ingest, property_index_rebuilt,
};
pub use requests::register_requester;
pub(crate) use requests::{debounced, registered_storage_for, registered_storages};
pub use translation_resync::TranslationResyncCounts;

use crate::{cf, keys};
pub(crate) use branches::{branch_head, list_branches};
use raisin_error::Result;
use rocksdb::DB;

/// Run `kind` over one branch, or every branch of the repository when
/// `branch` is `None` — forks included, since each holds its own copies.
pub async fn run_repair(
    storage: &crate::RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: Option<&str>,
    kind: RepairKind,
    mut options: RepairOptions,
) -> Result<Vec<RepairReport>> {
    if kind == RepairKind::CollapseRuns {
        if !storage.config().history_gc_collapse_runs && !collapse_enabled_by_env() {
            return Err(raisin_error::Error::Validation(format!(
                "collapse_runs is disabled: set `history_gc_collapse_runs` in the storage \
                 configuration or {COLLAPSE_RUNS_ENV}=1 (plan Phase 9; default off)"
            )));
        }
        // Any replication setting, not just operation capture: the
        // coordinator applies peers' ops from a node id and port alone.
        options.cluster_mode |= storage.config().replicates();
    }
    if kind == RepairKind::CompoundBuilds {
        // Async: each build awaits its keyspace lock and runs its passes on
        // a blocking thread (plan Phase 13f).
        return compound_builds::run(storage, tenant_id, repo_id, branch, &options).await;
    }
    if kind == RepairKind::ResyncTranslations {
        // Async: it captures replication ops as it goes.
        return translation_resync::resync_translations(
            storage, tenant_id, repo_id, branch, &options,
        )
        .await;
    }
    let db = storage.db().clone();
    let node_id = repair_node_id(storage);
    let (tenant_id, repo_id) = (tenant_id.to_string(), repo_id.to_string());
    let branch = branch.map(str::to_string);
    let dry_run = options.dry_run;
    let (t, r) = (tenant_id.clone(), repo_id.clone());
    let reports = tokio::task::spawn_blocking(move || {
        run_repair_blocking(
            &db,
            &tenant_id,
            &repo_id,
            branch.as_deref(),
            kind,
            &node_id,
            &options,
        )
    })
    .await
    .map_err(|e| raisin_error::Error::storage(format!("repair task failed: {e}")))??;
    if kind == RepairKind::PropertyIndexVerify && !dry_run {
        property_state::queue_rebuilds_for_misses(storage, &t, &r, &reports).await?;
    }
    Ok(reports)
}

/// The env var that enables `collapse_runs` (`1`/`true`/`on`/`yes`), besides
/// `RocksDBConfig::history_gc_collapse_runs`. Default off.
pub const COLLAPSE_RUNS_ENV: &str = "RAISIN_HISTORY_GC_COLLAPSE_RUNS";

fn collapse_enabled_by_env() -> bool {
    std::env::var(COLLAPSE_RUNS_ENV).is_ok_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "on" | "yes"
        )
    })
}

/// The cluster node id a repair's state record is kept under (`local` on a
/// single node).
pub fn repair_node_id(storage: &crate::RocksDBStorage) -> String {
    storage
        .config()
        .cluster_node_id
        .clone()
        .unwrap_or_else(|| "local".to_string())
}

/// The synchronous body of [`run_repair`].
pub fn run_repair_blocking(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: Option<&str>,
    kind: RepairKind,
    node_id: &str,
    options: &RepairOptions,
) -> Result<Vec<RepairReport>> {
    if kind == RepairKind::CollapseRuns {
        crate::management::history_gc::collapse::precheck_headroom(db, options)?;
    } else if options.check_headroom && !options.dry_run && kind.writes() {
        check_headroom_assuming(db, kind.column_family(), options.free_bytes_override)?;
    }
    let branches = match branch {
        Some(branch) => vec![branch.to_string()],
        None => list_branches(db, tenant_id, repo_id)?,
    };
    branches
        .iter()
        .map(|branch| repair_branch(db, tenant_id, repo_id, branch, kind, node_id, options))
        .collect()
}

fn repair_branch(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    kind: RepairKind,
    node_id: &str,
    options: &RepairOptions,
) -> Result<RepairReport> {
    let key = state_key(tenant_id, repo_id, branch, kind.slug(), node_id);
    let previous = load_state(db, tenant_id, repo_id, branch, kind.slug(), node_id)?;
    let resumed = previous
        .as_ref()
        .is_some_and(|s| s.status == "running" && s.cursor.is_some())
        && !options.dry_run;
    let state = if resumed {
        previous.unwrap_or_default()
    } else {
        RepairState::default()
    };
    // An inserting repair holds its `(branch, CF)` against run-collapse for
    // the whole branch (it writes below existing entries).
    let _inserting = (kind.inserts() && !options.dry_run).then(|| {
        crate::management::cf_exclusion::enter_inserter(
            db,
            tenant_id,
            repo_id,
            branch,
            kind.column_family(),
        )
    });
    let mut writer = cursor::BoundedWriter::new(
        db,
        key,
        state,
        options.batch_bytes,
        options.dry_run,
        options.stop_after_batches,
        options.max_bytes_per_sec,
        options.before_commit.clone(),
    );
    // A fresh `property_index` rebuild records the invalidation epoch it
    // starts under; a resumed one keeps the epoch of the run it continues.
    if kind == RepairKind::PropertyIndex && !resumed {
        writer.set_epoch(property_state::rebuild_epoch(
            db, tenant_id, repo_id, branch, node_id,
        )?);
    }
    let mut report = RepairReport {
        branch: branch.to_string(),
        repair: kind.slug().to_string(),
        dry_run: options.dry_run,
        resumed,
        ..RepairReport::default()
    };

    let completed = match kind {
        RepairKind::OrderedChildren => {
            let scope = ordered_children::Scope {
                tenant_id,
                repo_id,
                branch,
                head: branch_head(db, tenant_id, repo_id, branch)?,
            };
            let skip_nodes_pass = writer.state().pass == ordered_children::PASS_ORDERED;
            (skip_nodes_pass
                || ordered_children::nodes_pass(db, &scope, &mut writer, &mut report.ordered)?)
                && ordered_pass::ordered_pass(db, &scope, &mut writer, &mut report.ordered)?
        }
        RepairKind::PathTombstone => path_tombstone::path_pass(
            db,
            &keys::branch_prefix(tenant_id, repo_id, branch),
            &mut writer,
            &mut report.scanned,
        )?,
        RepairKind::NodePath => node_path_backfill::node_path_pass(
            db,
            &node_path_backfill::Scope {
                tenant_id,
                repo_id,
                branch,
            },
            &mut writer,
            &mut report.node_path,
        )?,
        RepairKind::PropertyIndex | RepairKind::PropertyIndexVerify => {
            property_index::property_index_pass(
                db,
                &property_index::Scope {
                    tenant_id,
                    repo_id,
                    branch,
                },
                &mut writer,
                &mut report.property_index,
                (kind == RepairKind::PropertyIndexVerify).then_some(options.sample_every),
            )?
        }
        RepairKind::ResyncTranslations | RepairKind::CompoundBuilds => {
            return Err(raisin_error::Error::Validation(format!(
                "{} runs through run_repair (it is async)",
                kind.slug()
            )))
        }
        RepairKind::LocalizedNames => crate::localized_name::rebuild::rebuild_branch(
            db,
            (tenant_id, repo_id, branch),
            &mut writer,
            resumed,
            &mut report.localized_names,
        )?,
        RepairKind::BlockOverlayTombstones => block_overlay_tombstones::block_overlay_pass(
            db,
            &block_overlay_tombstones::Scope {
                tenant_id,
                repo_id,
                branch,
                head: branch_head(db, tenant_id, repo_id, branch)?,
            },
            &mut writer,
            &mut report.block_overlays,
        )?,
        RepairKind::CollapseRuns => {
            crate::management::history_gc::collapse::collapse_branch_at_head(
                db,
                tenant_id,
                repo_id,
                branch,
                node_id,
                branch_head(db, tenant_id, repo_id, branch)?,
                &mut writer,
                &mut report.collapse,
                options,
            )?
        }
    };

    if completed && kind == RepairKind::PropertyIndex && !options.dry_run {
        // `done` only while no invalidation happened since the run started.
        if !property_state::commit_rebuild(db, &mut writer, tenant_id, repo_id, branch, node_id)? {
            report.writes = writer.report.clone();
            return Ok(report);
        }
    } else if completed {
        writer.commit("done")?;
        if kind == RepairKind::PropertyIndexVerify
            && report.property_index.missing > 0
            && !options.dry_run
        {
            // Fail closed at once: the delta writer does full puts on this
            // branch until the rebuild `run_repair` queues has completed.
            invalidate_property_index_rebuild(db, tenant_id, repo_id, branch, node_id)?;
        }
    }
    report.completed = completed;
    report.writes = writer.report.clone();
    tracing::info!(
        tenant_id,
        repo_id,
        branch,
        repair = kind.slug(),
        dry_run = options.dry_run,
        completed,
        written = report.writes.written,
        bytes = report.writes.bytes,
        "index repair finished a branch"
    );
    Ok(report)
}
