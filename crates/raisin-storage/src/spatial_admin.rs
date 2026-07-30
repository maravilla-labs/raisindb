// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The operator-facing surface of the spatial index: read its local state,
//! census the bytes that are physically there, and schedule a local rebuild.
//!
//! # Why this is a separate trait from [`crate::spatial::SpatialStateSource`]
//!
//! `SpatialStateSource` answers the one question the *query planner* asks on every
//! spatial predicate — "may I trust this index?" — and is deliberately kept to
//! that, because it sits in a hot path. This trait is the admin surface: it is
//! allowed to scan, and it is allowed to enqueue work.
//!
//! # Why a census exists at all
//!
//! `SHOW SPATIAL INDEX HEALTH` reading only the state record would be reporting
//! *intent about reality* rather than reality: the record says "built at
//! precisions X", but nothing in it proves a single key exists. The whole class of
//! bug this subsystem is being repaired for is exactly that gap — a planner that
//! believed an index it had never verified. [`SpatialEntryCensus`] counts the keys.
//!
//! # Cluster scope, stated out loud
//!
//! Everything here is **LOCAL to one node**. The spatial index is derived local
//! state, so a rebuild must be run per node. Cluster-wide fan-out happens through
//! *configuration*, because `WorkspaceConfig.spatial` is replicated: an
//! `ALTER SPATIAL INDEX` reaches every peer, and each peer then notices its own
//! `policy_hash` mismatch and schedules its own build.

use async_trait::async_trait;
use raisin_error::Result;

use crate::spatial::SpatialIndexState;

/// A physical count of what is actually in the spatial index column family for
/// one (workspace, property).
///
/// "Live" means *newest revision wins*: within a cell the entries for one node are
/// ordered newest-first, so the first entry seen for a `(cell, node)` pair decides
/// whether that pair is live or tombstoned. That is the same resolution rule the
/// query scan applies, so a census can never disagree with a query about which
/// entries count.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct SpatialEntryCensus {
    /// The geometry property these counts describe.
    pub property: String,
    /// Distinct `(cell, node)` pairs whose newest entry is live.
    pub live_entries: u64,
    /// Distinct `(cell, node)` pairs whose newest entry is a tombstone.
    pub tombstoned_entries: u64,
    /// Distinct node ids with at least one live entry.
    pub distinct_nodes: u64,
    /// Live entry count per geohash precision, coarsest precision first.
    ///
    /// This is the observable that proves a reindex actually moved data: after
    /// `ALTER SPATIAL INDEX … SET PRECISIONS` plus a rebuild, the precisions
    /// present here must be the new set.
    pub live_per_precision: Vec<(usize, u64)>,
    /// Every distinct `policy_hash` stamped into a live entry, ascending.
    ///
    /// More than one value means entries produced under two different policies
    /// coexist — normal mid-rebuild, and a signal that a rebuild is outstanding
    /// otherwise.
    pub policy_hashes: Vec<u64>,
    /// Live entries still carrying the pre-`SpatialEntry` `"{lon},{lat}"` value.
    pub legacy_entries: u64,
    /// Raw keys visited, including superseded revisions.
    pub raw_keys_scanned: u64,
}

/// Administrative operations on one node's local spatial index.
#[async_trait]
pub trait SpatialIndexAdmin: Send + Sync {
    /// Every local index-state record in the workspace, keyed by property name.
    fn states(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
    ) -> Result<Vec<(String, SpatialIndexState)>>;

    /// Count the index entries physically present.
    ///
    /// `property = None` censuses every geometry property that has entries.
    fn census(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property: Option<&str>,
    ) -> Result<Vec<SpatialEntryCensus>>;

    /// Schedule a LOCAL spatial index build.
    ///
    /// Returns the job id. Enqueue is idempotent via the job's dedup key, so a
    /// duplicate request while one is queued or running collapses onto it.
    async fn queue_build(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property: Option<&str>,
        rebuild: bool,
    ) -> Result<String>;
}
