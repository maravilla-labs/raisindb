//! Property and reference indexing for node creation
//!
//! This module handles indexing of node properties and references to enable
//! efficient querying and backlink lookups.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;

use crate::repositories::hash_property_value;
use crate::transaction::types::extract_references;
use crate::transaction::RocksDBTransaction;
use crate::{cf, cf_handle, keys};

/// Index all properties for a node (including pseudo-properties)
///
/// Creates property indexes for:
/// - Custom properties from node.properties
/// - Pseudo-properties: __node_type, __name, __archetype, __created_by, __updated_by, __created_at, __updated_at
///
/// These indexes enable efficient querying by property values.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `tenant_id` - The tenant ID
/// * `repo_id` - The repository ID
/// * `branch` - The branch name
/// * `workspace` - The workspace name
/// * `node` - The node whose properties to index
/// * `revision` - The HLC revision for versioning
///
/// # Errors
///
/// Returns error if lock is poisoned
pub(super) fn index_node_properties(
    tx: &RocksDBTransaction,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node: &Node,
    revision: &HLC,
) -> Result<()> {
    let cf_property = cf_handle(&tx.db, cf::PROPERTY_INDEX)?;
    let is_published = node.published_at.is_some();

    let mut batch = tx
        .batch
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

    // Index custom properties
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

    // Index node_type as pseudo-property
    let node_type_key = keys::property_index_key_versioned(
        tenant_id,
        repo_id,
        branch,
        workspace,
        "__node_type",
        &node.node_type,
        revision,
        &node.id,
        is_published,
    );
    batch.put_cf(cf_property, node_type_key, node.id.as_bytes());

    // Index name if not empty
    if !node.name.is_empty() {
        let name_key = keys::property_index_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            "__name",
            &node.name,
            revision,
            &node.id,
            is_published,
        );
        batch.put_cf(cf_property, name_key, node.id.as_bytes());
    }

    // Index archetype if present
    if let Some(ref archetype) = node.archetype {
        if !archetype.is_empty() {
            let archetype_key = keys::property_index_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                "__archetype",
                archetype,
                revision,
                &node.id,
                is_published,
            );
            batch.put_cf(cf_property, archetype_key, node.id.as_bytes());
        }
    }

    // Index created_by if present
    if let Some(ref created_by) = node.created_by {
        if !created_by.is_empty() {
            let created_by_key = keys::property_index_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                "__created_by",
                created_by,
                revision,
                &node.id,
                is_published,
            );
            batch.put_cf(cf_property, created_by_key, node.id.as_bytes());
        }
    }

    // Index updated_by if present
    if let Some(ref updated_by) = node.updated_by {
        if !updated_by.is_empty() {
            let updated_by_key = keys::property_index_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                "__updated_by",
                updated_by,
                revision,
                &node.id,
                is_published,
            );
            batch.put_cf(cf_property, updated_by_key, node.id.as_bytes());
        }
    }

    // Index created_at if present (using i64 microseconds for efficient sorting)
    if let Some(created_at) = node.created_at {
        let created_at_key = keys::property_index_key_versioned_timestamp(
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
        batch.put_cf(cf_property, created_at_key, node.id.as_bytes());
    }

    // Index updated_at if present (using i64 microseconds for efficient sorting)
    if let Some(updated_at) = node.updated_at {
        let updated_at_key = keys::property_index_key_versioned_timestamp(
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
        batch.put_cf(cf_property, updated_at_key, node.id.as_bytes());
    }

    // Index geometry properties in the spatial index (within same batch for atomicity)
    for (prop_name, prop_value) in &node.properties {
        if let PropertyValue::Geometry(geojson) = prop_value {
            tx.storage.spatial_index.index_geometry_to_batch(
                &mut batch,
                tenant_id,
                repo_id,
                branch,
                workspace,
                &node.id,
                prop_name,
                geojson,
                revision,
            )?;
        }
    }

    Ok(())
}

/// Tombstone old spatial index entries for a node's geometry properties.
///
/// Called before re-indexing during updates to prevent stale geohash entries.
pub(super) fn tombstone_spatial_properties(
    tx: &RocksDBTransaction,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    old_node: &Node,
    revision: &HLC,
) -> Result<()> {
    let mut batch = tx
        .batch
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

    for (prop_name, prop_value) in &old_node.properties {
        if matches!(prop_value, PropertyValue::Geometry(_)) {
            tx.storage.spatial_index.unindex_geometry_to_batch(
                &mut batch,
                tenant_id,
                repo_id,
                branch,
                workspace,
                &old_node.id,
                prop_name,
                revision,
            )?;
        }
    }

    Ok(())
}

/// Index unique properties for a node
///
/// Writes unique index entries for all properties marked as `unique: true` in the NodeType.
/// These indexes enable O(1) conflict detection for unique constraint enforcement.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `tenant_id` - The tenant ID
/// * `repo_id` - The repository ID
/// * `branch` - The branch name
/// * `workspace` - The workspace name
/// * `node` - The node whose unique properties to index
/// * `revision` - The HLC revision for versioning
///
/// # Errors
///
/// Returns error if lock is poisoned or NodeType loading fails
pub(super) async fn index_unique_properties(
    tx: &RocksDBTransaction,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node: &Node,
    revision: &HLC,
) -> Result<()> {
    use crate::repositories::UniqueIndexManager;
    use raisin_storage::NodeTypeRepository;

    // Get NodeType to check for unique properties (async - done before locking batch)
    let node_type = match tx
        .node_repo
        .node_type_repo
        .get(
            raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
            &node.node_type,
            None,
        )
        .await?
    {
        Some(nt) => nt,
        None => return Ok(()), // No NodeType = no unique indexes
    };

    // Get properties that have unique: true
    let unique_properties = match node_type.properties {
        Some(ref props) => props
            .iter()
            .filter_map(|p| {
                if p.unique.unwrap_or(false) {
                    p.name.clone()
                } else {
                    None
                }
            })
            .collect::<Vec<_>>(),
        None => return Ok(()), // No properties = no unique indexes
    };

    if unique_properties.is_empty() {
        return Ok(());
    }

    // Now lock the batch for synchronous writes
    let mut batch = tx
        .batch
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

    let unique_manager = UniqueIndexManager::new(tx.db.clone());

    // Add index entries for each unique property with a value
    for prop_name in unique_properties {
        if let Some(prop_value) = node.properties.get(&prop_name) {
            let value_hash = hash_property_value(prop_value);

            unique_manager.add_unique_index_to_batch(
                &mut batch,
                tenant_id,
                repo_id,
                branch,
                workspace,
                &node.node_type,
                &prop_name,
                &value_hash,
                revision,
                &node.id,
            )?;
        }
    }

    Ok(())
}

/// Write tombstones for unique index entries
///
/// Writes tombstones for unique index entries when a node is deleted or unique property value changes.
/// This releases the unique value so it can be used by other nodes.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `tenant_id` - The tenant ID
/// * `repo_id` - The repository ID
/// * `branch` - The branch name
/// * `workspace` - The workspace name
/// * `node` - The node whose unique properties to tombstone (with OLD property values)
/// * `revision` - The HLC revision for versioning
///
/// # Errors
///
/// Returns error if lock is poisoned or NodeType loading fails
pub(super) async fn tombstone_unique_properties(
    tx: &RocksDBTransaction,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node: &Node,
    revision: &HLC,
) -> Result<()> {
    use crate::repositories::UniqueIndexManager;
    use raisin_storage::NodeTypeRepository;

    // Get NodeType to check for unique properties (async - done before locking batch)
    let node_type = match tx
        .node_repo
        .node_type_repo
        .get(
            raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
            &node.node_type,
            None,
        )
        .await?
    {
        Some(nt) => nt,
        None => return Ok(()), // No NodeType = no unique indexes to tombstone
    };

    // Get properties that have unique: true
    let unique_properties = match node_type.properties {
        Some(ref props) => props
            .iter()
            .filter_map(|p| {
                if p.unique.unwrap_or(false) {
                    p.name.clone()
                } else {
                    None
                }
            })
            .collect::<Vec<_>>(),
        None => return Ok(()),
    };

    if unique_properties.is_empty() {
        return Ok(());
    }

    // Now lock the batch for synchronous writes
    let mut batch = tx
        .batch
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

    let unique_manager = UniqueIndexManager::new(tx.db.clone());

    // Add tombstones for each unique property with a value
    for prop_name in unique_properties {
        if let Some(prop_value) = node.properties.get(&prop_name) {
            let value_hash = hash_property_value(prop_value);

            unique_manager.add_unique_tombstone_to_batch(
                &mut batch,
                tenant_id,
                repo_id,
                branch,
                workspace,
                &node.node_type,
                &prop_name,
                &value_hash,
                revision,
            )?;
        }
    }

    Ok(())
}

/// Index references for a node
///
/// Creates both forward and reverse reference indexes:
/// - Forward: source_node_id + property_path -> reference
/// - Reverse: target_workspace + target_path -> source_node_id + property_path
///
/// These indexes enable efficient reference queries and backlink lookups.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `tenant_id` - The tenant ID
/// * `repo_id` - The repository ID
/// * `branch` - The branch name
/// * `workspace` - The workspace name
/// * `node` - The node whose references to index
/// * `revision` - The HLC revision for versioning
///
/// # Errors
///
/// Returns error if:
/// - Lock is poisoned
/// - Serialization fails
pub(super) fn index_node_references(
    tx: &RocksDBTransaction,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node: &Node,
    revision: &HLC,
) -> Result<()> {
    let cf_reference = cf_handle(&tx.db, cf::REFERENCE_INDEX)?;
    let is_published = node.published_at.is_some();
    let refs = extract_references(&node.properties);

    let mut batch = tx
        .batch
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

    for (property_path, reference) in refs {
        // Forward index
        let forward_key = keys::reference_forward_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &node.id,
            &property_path,
            revision,
            is_published,
        );
        let ref_value = rmp_serde::to_vec(&reference)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;
        batch.put_cf(cf_reference, forward_key, ref_value);

        // Reverse index
        let reverse_key = keys::reference_reverse_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &reference.workspace,
            &reference.path,
            &node.id,
            &property_path,
            revision,
            is_published,
        );
        batch.put_cf(cf_reference, reverse_key, b"");
    }

    Ok(())
}
