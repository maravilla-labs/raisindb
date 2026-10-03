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
//!   per-node state record lets an operator see which nodes have not.
//!
//! [`run_repair`] is the entry point the job handler and the admin endpoint
//! call.

mod cursor;
mod enqueue;
mod ordered_children;
mod ordered_pass;
mod parent_entries;
mod path_tombstone;

pub use cursor::{check_headroom, load_state, state_key, BatchReport, RepairState};
pub use enqueue::{
    enqueue_index_repair, reenqueue_repairs_after_checkpoint, reenqueue_repairs_after_ingest,
};
pub use ordered_children::OrderedChildrenCounts;

use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::DB;
use serde::{Deserialize, Serialize};

/// Which repair to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepairKind {
    /// Tombstone ORDERED_CHILDREN entries deletes left live (plan item 2.1).
    OrderedChildren,
    /// Rewrite merge's legacy `\0` PATH_INDEX tombstones as `T` (item 2.2).
    PathTombstone,
}

impl RepairKind {
    /// Stable name, used in the state record key and the job type.
    pub fn slug(&self) -> &'static str {
        match self {
            Self::OrderedChildren => "ordered_children",
            Self::PathTombstone => "path_tombstone",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        match slug {
            "ordered_children" => Some(Self::OrderedChildren),
            "path_tombstone" => Some(Self::PathTombstone),
            _ => None,
        }
    }

    /// The CF the repair writes, which the disk precheck sizes.
    fn column_family(&self) -> &'static str {
        match self {
            Self::OrderedChildren => cf::ORDERED_CHILDREN,
            Self::PathTombstone => cf::PATH_INDEX,
        }
    }
}

/// How to run a repair.
#[derive(Debug, Clone)]
pub struct RepairOptions {
    /// Report what would be written; write nothing.
    pub dry_run: bool,
    /// Commit when a batch reaches this many bytes.
    pub batch_bytes: usize,
    /// Refuse to start without 2x the CF size free on the data volume.
    pub check_headroom: bool,
    /// Stop as if crashed after this many committed batches (tests).
    pub stop_after_batches: Option<usize>,
    /// Average write rate cap between batches, in bytes per second; 0 means
    /// unlimited. Keeps a repair from saturating the disk a live node serves
    /// from.
    pub max_bytes_per_sec: u64,
}

impl Default for RepairOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            batch_bytes: 8 * 1024 * 1024,
            check_headroom: true,
            stop_after_batches: None,
            max_bytes_per_sec: 32 * 1024 * 1024,
        }
    }
}

/// What one repair did on one branch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepairReport {
    pub branch: String,
    pub repair: String,
    pub dry_run: bool,
    /// The run picked up from a persisted cursor.
    pub resumed: bool,
    /// The run reached the end (false: stopped early, resumable).
    pub completed: bool,
    /// Writes queued/committed, batches and bytes.
    pub writes: BatchReport,
    /// PATH_INDEX entries scanned (path repair).
    pub scanned: u64,
    /// ORDERED_CHILDREN repair counts.
    pub ordered: OrderedChildrenCounts,
}

/// Run `kind` over one branch, or every branch of the repository when
/// `branch` is `None` — forks included, since each holds its own copies.
pub async fn run_repair(
    storage: &crate::RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: Option<&str>,
    kind: RepairKind,
    options: RepairOptions,
) -> Result<Vec<RepairReport>> {
    let db = storage.db().clone();
    let node_id = storage
        .config()
        .cluster_node_id
        .clone()
        .unwrap_or_else(|| "local".to_string());
    let (tenant_id, repo_id) = (tenant_id.to_string(), repo_id.to_string());
    let branch = branch.map(str::to_string);
    tokio::task::spawn_blocking(move || {
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
    .map_err(|e| raisin_error::Error::storage(format!("repair task failed: {e}")))?
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
    if options.check_headroom && !options.dry_run {
        check_headroom(db, kind.column_family())?;
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
    let mut writer = cursor::BoundedWriter::new(
        db,
        key,
        state,
        options.batch_bytes,
        options.dry_run,
        options.stop_after_batches,
        options.max_bytes_per_sec,
    );
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
    };

    if completed {
        writer.commit("done")?;
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

/// Every branch of a repository, from the BRANCHES records.
fn list_branches(db: &DB, tenant_id: &str, repo_id: &str) -> Result<Vec<String>> {
    let prefix = keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("branches")
        .build_prefix();
    let cf = cf_handle(db, cf::BRANCHES)?;
    let mut branches = Vec::new();
    for item in crate::prefix_scan(db, cf, &prefix) {
        let (key, _) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        if let Ok(name) = std::str::from_utf8(&key[prefix.len()..]) {
            if !name.is_empty() && !name.contains('\0') {
                branches.push(name.to_string());
            }
        }
    }
    Ok(branches)
}

/// The branch HEAD, or `None` when the record is missing.
fn branch_head(db: &DB, tenant_id: &str, repo_id: &str, branch: &str) -> Result<Option<HLC>> {
    let cf = cf_handle(db, cf::BRANCHES)?;
    let bytes = db
        .get_cf(cf, keys::branch_key(tenant_id, repo_id, branch))
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
    match bytes {
        Some(bytes) => rmp_serde::from_slice::<raisin_context::Branch>(&bytes)
            .map(|b| Some(b.head))
            .map_err(|e| raisin_error::Error::storage(format!("Branch decode error: {e}"))),
        None => Ok(None),
    }
}
