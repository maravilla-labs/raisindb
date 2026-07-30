//! The shared, synchronous secondary-index batch writer.
//!
//! # Why this module exists
//!
//! Before it, the logic that writes a node's secondary indexes existed in FOUR
//! hand-maintained copies:
//!
//! | path | file | spatial? |
//! |---|---|---|
//! | transaction (SQL DML + `NodeService`) | `transaction/context/nodes/create/indexing.rs` | yes |
//! | repository (`storage.nodes().add`) | `repositories/nodes/crud/indexing/mod.rs` | **no** |
//! | replication apply | `replication/application/index_writers.rs` | **no** |
//! | replication snapshot (now deleted) | `replication/application/replication_core.rs` | **no** |
//!
//! Nothing structurally guaranteed parity, so a new index type got wired into one
//! path and forgotten on the others. That is exactly how the compound-index bug
//! documented at `transaction/.../indexing.rs` happened ("the transaction write
//! path historically did NOT maintain compound indexes"), and it is how spatial
//! indexing ended up **absent from the replication apply path entirely** — making
//! a geometry written on node1 spatially queryable only on node1, in a masterless
//! cluster, with the planner turning that into zero rows instead of an error.
//!
//! [`spatial`] is now the ONLY code permitted to write or tombstone spatial index
//! entries. Every write path delegates here. A fifth path that forgets spatial is
//! caught by `tests/all/index_parity_test.rs`, which writes one fixed node through
//! every live path and asserts the resulting key sets are byte-identical.
//!
//! # Why spatial goes inline rather than event -> job
//!
//! Fulltext indexing uses an event -> job hop because Tantivy is a separate store
//! that cannot join a RocksDB `WriteBatch`. The spatial index IS a RocksDB column
//! family, so it joins the caller's batch and commits atomically with the record.
//! That is strictly stronger: there is no window in which a node holds the record
//! but not its index entry, and no dependence on the job system being healthy.

pub mod spatial;
pub mod spatial_policy;

pub use spatial::{
    write_node_spatial_indexes, write_spatial_property, SpatialIndexTargets, SPATIAL_TOMBSTONE,
};
pub use spatial_policy::NodeSpatialPolicies;

/// The scope every index key is prefixed with.
///
/// Bundled into one struct because the five-argument
/// `(tenant, repo, branch, workspace)` tail was being passed positionally through
/// a dozen call sites, and a transposed pair there is a silent cross-tenant index
/// write.
#[derive(Debug, Clone, Copy)]
pub struct IndexCtx<'a> {
    pub tenant_id: &'a str,
    pub repo_id: &'a str,
    pub branch: &'a str,
    pub workspace: &'a str,
}

impl<'a> IndexCtx<'a> {
    pub fn new(tenant_id: &'a str, repo_id: &'a str, branch: &'a str, workspace: &'a str) -> Self {
        Self {
            tenant_id,
            repo_id,
            branch,
            workspace,
        }
    }
}
