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
    let is_published = node.published_at.is_some();

    for (prop_path, prop_value) in &node.properties {
        if let PropertyValue::Reference(ref_data) = prop_value {
            // Forward reference: node -> referenced node
            let fwd_key = keys::reference_forward_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &node.id,
                prop_path,
                revision,
                is_published,
            );
            batch.put_cf(cf_reference, fwd_key, ref_data.id.as_bytes());

            // Reverse reference: referenced node -> this node
            let rev_key = keys::reference_reverse_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &ref_data.workspace,
                &ref_data.path,
                &node.id,
                prop_path,
                revision,
                is_published,
            );
            batch.put_cf(cf_reference, rev_key, node.id.as_bytes());
        }
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

/// Write all indexes for a node (properties, references, relations)
///
/// This is a convenience function that calls all three index writers
pub fn write_all_node_indexes(
    batch: &mut WriteBatch,
    cf_property: &rocksdb::ColumnFamily,
    cf_reference: &rocksdb::ColumnFamily,
    cf_relation: &rocksdb::ColumnFamily,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node: &Node,
    revision: &HLC,
) -> Result<()> {
    write_property_indexes(
        batch,
        cf_property,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node,
        revision,
    );

    write_reference_indexes(
        batch,
        cf_reference,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node,
        revision,
    );

    write_relation_indexes(
        batch,
        cf_relation,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node,
        revision,
    )?;

    Ok(())
}
