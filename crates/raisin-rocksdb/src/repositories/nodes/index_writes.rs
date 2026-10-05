//! `index.skip_unchanged` (plan Phases 7 and 7b): whether the write paths may
//! skip re-putting index entries a node's predecessor already holds.
//!
//! **Default ON (Phase 7b), and still gated per branch.** The flag
//! (`RocksDBConfig::index_skip_unchanged`, `[storage] index_skip_unchanged`,
//! `RAISIN_INDEX_SKIP_UNCHANGED`) says the operator wants it; a branch's
//! `property_index` rebuild state record being `done` under THIS node's id
//! says it is safe there — the rebuild is what fills the holes (replica
//! membership entries above all) that full re-puts used to heal by accident.
//! With the flag on, that rebuild queues itself per branch in the background
//! (`repair::auto_property_index`). Until it is done every write on the
//! branch is the full put it always was. Turning the flag off is always a safe
//! rollback: the full writer produces a superset.
//!
//! What made default-on safe is the node commit step (`indexing::node_lock`):
//! two overlapping writers of one node can commit out of revision order, and
//! each commit now re-validates its staged delta against the versions stored
//! at that moment, under a per-node mutex every funnel takes.
//!
//! `RAISIN_INDEX_SKIP_UNCHANGED=0|false|off|no` turns it off and `1|true|on|
//! yes` on, overriding the config either way; unset, the config decides.
//! A checkpoint ingest, or a `property_index_verify` that finds a miss, puts
//! the branch back to full puts until it is rebuilt.

use super::NodeRepositoryImpl;
use crate::indexing::{IndexCtx, OwnedBaseline};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;

/// The env var that turns `index.skip_unchanged` on (`1`/`true`/`on`/`yes`).
pub const SKIP_UNCHANGED_ENV: &str = "RAISIN_INDEX_SKIP_UNCHANGED";

pub(crate) struct IndexWritePolicy {
    skip_unchanged: AtomicBool,
    /// The cluster node id the rebuild state is kept under (`local` on a
    /// single node) — the same id `repair::repair_node_id` uses.
    node_id: RwLock<String>,
}

/// `RAISIN_INDEX_SKIP_UNCHANGED`, when set to something it understands.
fn env_override() -> Option<bool> {
    let value = std::env::var(SKIP_UNCHANGED_ENV).ok()?;
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" => Some(true),
        "0" | "false" | "off" | "no" => Some(false),
        _ => None,
    }
}

impl IndexWritePolicy {
    pub(crate) fn from_env() -> Self {
        Self {
            skip_unchanged: AtomicBool::new(env_override().unwrap_or(true)),
            node_id: RwLock::new("local".to_string()),
        }
    }
}

impl NodeRepositoryImpl {
    /// Configure skip-unchanged writes: `enabled` from the config (overridden
    /// by `RAISIN_INDEX_SKIP_UNCHANGED` when set), `node_id` from
    /// `RocksDBConfig::cluster_node_id`.
    pub fn configure_index_writes(&self, enabled: bool, node_id: Option<&str>) {
        self.index_writes
            .skip_unchanged
            .store(env_override().unwrap_or(enabled), Ordering::Relaxed);
        if let Ok(mut id) = self.index_writes.node_id.write() {
            *id = node_id.unwrap_or("local").to_string();
        }
    }

    /// Turn `index.skip_unchanged` on or off at runtime (tests, rollback).
    pub fn set_index_skip_unchanged(&self, enabled: bool) {
        self.index_writes
            .skip_unchanged
            .store(enabled, Ordering::Relaxed);
    }

    /// Whether `index.skip_unchanged` is set.
    pub fn index_skip_unchanged(&self) -> bool {
        self.index_writes.skip_unchanged.load(Ordering::Relaxed)
    }

    /// Whether a write on `branch` may skip unchanged entries: the flag is on,
    /// no merge into the branch is in progress (`merge_window`), AND this node
    /// has rebuilt the branch's property index.
    pub(crate) fn skip_unchanged_permitted(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> bool {
        if !self.index_skip_unchanged()
            || super::merge_window_open(&self.db, tenant_id, repo_id, branch)
        {
            return false;
        }
        let node_id = self
            .index_writes
            .node_id
            .read()
            .map(|id| id.clone())
            .unwrap_or_else(|_| "local".to_string());
        crate::management::async_indexing::repair::property_index_rebuilt(
            &self.db, tenant_id, repo_id, branch, &node_id,
        )
    }

    /// The baseline of a write that never skips (cross-branch stage):
    /// `Full(fallback)`, or `OutOfOrder` when versions above `revision` are
    /// already stored (`indexing::resolve_baseline` with the gate closed).
    pub(crate) fn full_baseline(
        &self,
        ctx: &IndexCtx<'_>,
        node_id: &str,
        revision: &HLC,
        fallback: Option<&Node>,
    ) -> Result<OwnedBaseline> {
        crate::indexing::resolve_baseline(&self.db, ctx, node_id, revision, fallback, false)
    }

    /// THE baseline for an ordinary (fresh-revision) write of `node_id` at
    /// `revision`: `Full(fallback)` unless skipping is permitted, else what
    /// the stored versions prove (`indexing::resolve_baseline`).
    pub(crate) fn delta_baseline(
        &self,
        ctx: &IndexCtx<'_>,
        node_id: &str,
        revision: &HLC,
        fallback: Option<&Node>,
    ) -> Result<OwnedBaseline> {
        let permitted = self.skip_unchanged_permitted(ctx.tenant_id, ctx.repo_id, ctx.branch);
        crate::indexing::resolve_baseline(&self.db, ctx, node_id, revision, fallback, permitted)
    }
}
