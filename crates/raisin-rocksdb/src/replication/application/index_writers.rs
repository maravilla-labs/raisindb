//! Index writing helpers for node properties and relations
//!
//! This module provides utilities for writing property indexes, reference indexes,
//! and relation indexes to RocksDB during replication.

use crate::{keys, repositories::hash_property_value};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use rocksdb::WriteBatch;

/// Write all property indexes for a node to a batch
///
/// This includes:
/// - Custom properties with hashed values
/// - System fields (__node_type, __name, __archetype, etc.)
/// - Timestamp fields (__created_at, __updated_at)
pub fn write_property_indexes(
    batch: &mut WriteBatch,
    cf_property: &rocksdb::ColumnFamily,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node: &Node,
    revision: &HLC,
) {
    let is_published = node.published_at.is_some();

    // Index custom properties with hashed values
    for (prop_name, prop_value) in &node.properties {
        let value_hash = hash_property_value(prop_value);
        let prop_key = keys::property_index_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            prop_name,
            &value_hash,
            revision,
            &node.id,
            is_published,
        );
        batch.put_cf(cf_property, prop_key, node.id.as_bytes());
    }

    // Helper closure for writing system fields
    let mut write_field = |field: &str, value: &str| {
        if value.is_empty() {
            return;
        }
        let key = keys::property_index_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            field,
            value,
            revision,
            &node.id,
            is_published,
        );
        batch.put_cf(cf_property, key, node.id.as_bytes());
    };

    // Write system field indexes
    write_field("__node_type", &node.node_type);
    write_field("__name", &node.name);
    if let Some(ref archetype) = node.archetype {
        write_field("__archetype", archetype);
    }
    if let Some(ref created_by) = node.created_by {
        write_field("__created_by", created_by);
    }
    if let Some(ref updated_by) = node.updated_by {
        write_field("__updated_by", updated_by);
    }
    // Write timestamp fields using microsecond precision
    if let Some(created_at) = node.created_at {
        let key = keys::property_index_key_versioned_timestamp(
            tenant_id,
            repo_id,
            branch,
            workspace,
            "__created_at",
            created_at.timestamp_micros(),
            revision,
            &node.id,
            is_published,
        );
        batch.put_cf(cf_property, key, node.id.as_bytes());
    }
    if let Some(updated_at) = node.updated_at {
        let key = keys::property_index_key_versioned_timestamp(
            tenant_id,
            repo_id,
            branch,
            workspace,
            "__updated_at",
            updated_at.timestamp_micros(),
            revision,
            &node.id,
            is_published,
        );
        batch.put_cf(cf_property, key, node.id.as_bytes());
    }
}

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
pub fn write_all_node_indexes(
    batch: &mut WriteBatch,
    cfs: &ReplicationIndexCfs<'_>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node: &Node,
    revision: &HLC,
    policies: &crate::indexing::NodeSpatialPolicies,
) -> Result<()> {
    write_property_indexes(
        batch,
        cfs.property,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node,
        revision,
    );

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
