//! Index writing helpers for node properties and relations
//!
//! This module provides utilities for writing property indexes (through the
//! one delta writer), reference indexes, and relation indexes to RocksDB
//! during replication.

use crate::keys;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::WriteBatch;

/// Write reference indexes for a node to a batch
///
/// For each reference property, writes both forward and reverse indexes
pub fn write_reference_indexes(
    batch: &mut WriteBatch,
    cf_reference: &rocksdb::ColumnFamily,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node: &Node,
    revision: &HLC,
) {
    // Shared writer: nested references (arrays/objects/element content) with
    // the canonical dot-format property path — key format MUST match the
    // local write paths or replicated entries can never be tombstoned.
    if let Err(e) = crate::repositories::add_reference_index_entries(
        batch,
        cf_reference,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node,
        revision,
    ) {
        tracing::warn!(
            node_id = %node.id,
            error = %e,
            "Failed to write reference indexes for replicated node"
        );
    }
}

/// Write relation indexes for a node to a batch
///
/// For each relation, writes both forward and reverse indexes
pub fn write_relation_indexes(
    batch: &mut WriteBatch,
    cf_relation: &rocksdb::ColumnFamily,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node: &Node,
    revision: &HLC,
) -> Result<()> {
    for relation in &node.relations {
        let relation_bytes = rmp_serde::to_vec(&relation).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to serialize relation: {}", e))
        })?;

        // Forward relation: source -> target
        let fwd_key = keys::relation_forward_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &node.id,
            &relation.relation_type,
            revision,
            &relation.target,
        );
        batch.put_cf(cf_relation, fwd_key, &relation_bytes);

        // Reverse relation: target -> source
        let rev_key = keys::relation_reverse_key_versioned(
            tenant_id,
            repo_id,
            branch,
            &relation.workspace,
            &relation.target,
            &relation.relation_type,
            revision,
            &node.id,
        );
        batch.put_cf(cf_relation, rev_key, &relation_bytes);
    }

    Ok(())
}

/// Column families the replication index writers stage into.
///
/// Bundled so adding an index type is a struct field rather than another
/// positional argument at every call site — the shape that let SPATIAL_INDEX be
/// forgotten here in the first place.
pub struct ReplicationIndexCfs<'a> {
    pub property: &'a rocksdb::ColumnFamily,
    pub reference: &'a rocksdb::ColumnFamily,
    pub relation: &'a rocksdb::ColumnFamily,
    pub spatial: &'a rocksdb::ColumnFamily,
}

/// Write all secondary indexes for a replicated node (properties, references,
/// relations, **spatial**).
///
/// # The cluster bug this closes
///
/// RaisinDB is a masterless multi-master CRDT cluster: each instance builds its
/// own local indexes as replicated records arrive. Property, reference and relation
/// indexes were written here; **spatial was not written at all** — `grep -rn spatial
/// crates/raisin-rocksdb/src/replication/` matched one doc comment and zero code.
///
/// The production consequence: a geometry written on node1 was spatially queryable
/// ONLY on node1. Peers converged the node record correctly but held zero spatial
/// entries for it, so the same `ST_DWITHIN` returned different answers depending on
/// which node answered — and because the planner's `has_spatial_index()` was a
/// hardcoded `true` and the predicate was stripped from the residual filter, the
/// wrong answer was **zero rows, silently**, rather than an error.
///
/// Spatial goes inline in this batch rather than through an event -> job hop (the
/// route fulltext takes) because the spatial index IS a RocksDB column family and
/// can therefore join the caller's `WriteBatch`. That is strictly stronger: no
/// window where a peer holds the record but not its index entry, and no dependence
/// on the job system being healthy.
///
/// `policies` must be resolved by the caller (see
/// [`crate::indexing::NodeSpatialPolicies::from_local_state`]) because policy
/// resolution reads schema records, which is async, and this path is sync.
///
/// The PROPERTY_INDEX half is the ONE writer (`indexing::property_delta`),
/// never a skipping one here: `baseline` is `Full(replaced)` (the version this
/// one supersedes, whose values it no longer carries are tombstoned) — the
/// apply path keeps full puts until oracle stage 3 proves the delta under
/// permuted two-origin delivery (plan Phase 7 item 2) — or, when versions
/// ABOVE `revision` are already stored, `OutOfOrder`: a cluster node is both
/// an origin and a replica, so a successor may be a LOCAL write that skipped
/// unchanged entries, and only re-asserting it keeps this write's tombstones
/// from masking them. Membership (IS_A / HAS_MIXIN) is written like on the
/// origin — this writer used to omit it, so `IS_A(...)` was empty on every
/// replica. `in_place`: the revision is one a version is already stored at
/// (`versionable=false`).
#[allow(clippy::too_many_arguments)]
pub fn write_all_node_indexes(
    batch: &mut WriteBatch,
    db: &rocksdb::DB,
    cfs: &ReplicationIndexCfs<'_>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node: &Node,
    revision: &HLC,
    policies: &crate::indexing::NodeSpatialPolicies,
    baseline: crate::indexing::Baseline<'_>,
    in_place: bool,
) -> Result<()> {
    debug_assert!(!matches!(
        baseline,
        crate::indexing::Baseline::Predecessor(_)
    ));
    crate::indexing::write_property_index_delta(
        batch,
        crate::indexing::PropertyIndexTarget {
            db,
            cf: cfs.property,
        },
        &crate::indexing::IndexCtx::new(tenant_id, repo_id, branch, workspace),
        baseline,
        node,
        revision,
        // A replicated in-place write lands at its revision: the above-R
        // group lookup is the local writers' opt-in (see `property_delta::
        // in_place`), and the apply path has no skip-unchanged flag.
        if in_place {
            crate::indexing::InPlace::Reused(None)
        } else {
            crate::indexing::InPlace::No
        },
    )?;

    write_reference_indexes(
        batch,
        cfs.reference,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node,
        revision,
    );

    write_relation_indexes(
        batch,
        cfs.relation,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node,
        revision,
    )?;

    // Delegates to the ONE shared spatial writer, so this path cannot drift from
    // the transaction and repository paths again.
    crate::indexing::write_node_spatial_indexes(
        batch,
        &crate::indexing::SpatialIndexTargets {
            spatial_index: cfs.spatial,
        },
        &crate::indexing::IndexCtx::new(tenant_id, repo_id, branch, workspace),
        node,
        revision,
        policies,
    )?;

    Ok(())
}
