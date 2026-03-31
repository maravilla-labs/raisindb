//! CRUD operations for relations
//!
//! This module implements add_relation and remove_relation operations,
//! managing forward, reverse, and global indexes.

use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use raisin_models::nodes::{FullRelation, RelationRef};
use rocksdb::DB;
use std::sync::Arc;

use crate::keys::{
    relation_forward_key_versioned, relation_forward_prefix, relation_global_key_versioned,
    relation_reverse_key_versioned,
};

use super::helpers::{
    deserialize_relation_ref, get_relation_cf, is_tombstone, serialize_full_relation,
    serialize_relation_ref, TOMBSTONE,
};

/// Add a relation to all three indexes (forward, reverse, global)
pub(super) async fn add_relation(
    db: &Arc<DB>,
    revision: &HLC,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    source_workspace: &str,
    source_node_id: &str,
    source_node_type: &str,
    relation: &RelationRef,
) -> Result<()> {
    // Serialize the relation for forward index (stores target info)
    let forward_relation_bytes = serialize_relation_ref(relation)?;

    // Create reverse RelationRef (stores source info as if it were the target)
    // This allows incoming relationship queries to know the source node's type
    let reverse_relation = RelationRef::new(
        source_node_id.to_string(),   // target = source (from reverse perspective)
        source_workspace.to_string(), // workspace of source
        source_node_type.to_string(), // target_node_type = source's type
        relation.relation_type.clone(), // same relation type
        relation.weight,              // same weight
    );
    let reverse_relation_bytes = serialize_relation_ref(&reverse_relation)?;

    // Create FullRelation for global index (includes both source and target info WITH node types)
    let full_relation = FullRelation::from_source_and_ref(
        source_node_id.to_string(),
        source_workspace.to_string(),
        source_node_type.to_string(),
        relation,
    );
    let full_relation_bytes = serialize_full_relation(&full_relation)?;

    // Create forward index key (outgoing relationship FROM source TO target)
    let forward_key = relation_forward_key_versioned(
        tenant_id,
        repo_id,
        branch,
        source_workspace,
        source_node_id,
        &relation.relation_type,
        revision,
        &relation.target,
    );

    // Create reverse index key (incoming relationship TO target FROM source)
    let reverse_key = relation_reverse_key_versioned(
        tenant_id,
        repo_id,
        branch,
        &relation.workspace,
        &relation.target,
        &relation.relation_type,
        revision,
        source_node_id,
    );

    // Create global index key (for cross-workspace Cypher queries)
    let global_key = relation_global_key_versioned(
        tenant_id,
        repo_id,
        branch,
        &relation.relation_type,
        revision,
        source_workspace,
        source_node_id,
        &relation.workspace,
        &relation.target,
    );

    // Get relation column family handle
    let cf_relation = get_relation_cf(db)?;

    // Write all three indexes atomically to the RELATION_INDEX column family
    db.put_cf(cf_relation, &forward_key, &forward_relation_bytes)
        .map_err(|e| Error::storage(format!("Failed to write forward relation index: {}", e)))?;

    db.put_cf(cf_relation, &reverse_key, &reverse_relation_bytes)
        .map_err(|e| Error::storage(format!("Failed to write reverse relation index: {}", e)))?;

    db.put_cf(cf_relation, &global_key, &full_relation_bytes)
        .map_err(|e| Error::storage(format!("Failed to write global relation index: {}", e)))?;

    Ok(())
}

/// Remove relation(s) between source and target nodes
///
/// Since the method doesn't specify relation_type, this removes ALL relations
/// between source and target by writing tombstones.
pub(super) async fn remove_relation(
    db: &Arc<DB>,
    revision: &HLC,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    source_workspace: &str,
    source_node_id: &str,
    target_workspace: &str,
    target_node_id: &str,
) -> Result<bool> {
    // Get relation column family handle
    let cf_relation = get_relation_cf(db)?;

    // Use prefix scan to find all relations from source to target
    let prefix =
        relation_forward_prefix(tenant_id, repo_id, branch, source_workspace, source_node_id);

    let mut found_any = false;
    let mut relations_to_remove = Vec::new();

    // Scan to find all relations from source to target
    let iter = db.prefix_iterator_cf(cf_relation, &prefix);
    for item in iter {
        let (key, value) =
            item.map_err(|e| Error::storage(format!("Failed to iterate relations: {}", e)))?;

        if !key.starts_with(&prefix) {
            break;
        }

        // Skip tombstones
        if is_tombstone(&value) {
            continue;
        }

        // Deserialize to check if this relation points to our target
        let relation = deserialize_relation_ref(&value)?;

        // Check if this relation points to the target node in the target workspace
        if relation.target == target_node_id && relation.workspace == target_workspace {
            // Parse key to extract relation_type
            let key_parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
            if key_parts.len() >= 7 {
                let relation_type = String::from_utf8_lossy(key_parts[6]).to_string();
                relations_to_remove.push(relation_type);
                found_any = true;
            }
        }
    }

    // Write tombstones for all matching relations
    for relation_type in relations_to_remove {
        write_relation_tombstones(
            db,
            revision,
            tenant_id,
            repo_id,
            branch,
            source_workspace,
            source_node_id,
            target_workspace,
            target_node_id,
            &relation_type,
        )?;
    }

    Ok(found_any)
}

/// Write tombstones to all three indexes for a specific relation
pub(super) fn write_relation_tombstones(
    db: &DB,
    revision: &HLC,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    source_workspace: &str,
    source_node_id: &str,
    target_workspace: &str,
    target_node_id: &str,
    relation_type: &str,
) -> Result<()> {
    let forward_key = relation_forward_key_versioned(
        tenant_id,
        repo_id,
        branch,
        source_workspace,
        source_node_id,
        relation_type,
        revision,
        target_node_id,
    );

    let reverse_key = relation_reverse_key_versioned(
        tenant_id,
        repo_id,
        branch,
        target_workspace,
        target_node_id,
        relation_type,
        revision,
        source_node_id,
    );

    let global_key = relation_global_key_versioned(
        tenant_id,
        repo_id,
        branch,
        relation_type,
        revision,
        source_workspace,
        source_node_id,
        target_workspace,
        target_node_id,
    );

    let cf_relation = get_relation_cf(db)?;

    // Write tombstones to all three indexes in RELATION_INDEX column family
    db.put_cf(cf_relation, &forward_key, TOMBSTONE)
        .map_err(|e| Error::storage(format!("Failed to delete forward relation: {}", e)))?;

    db.put_cf(cf_relation, &reverse_key, TOMBSTONE)
        .map_err(|e| Error::storage(format!("Failed to delete reverse relation: {}", e)))?;

    db.put_cf(cf_relation, &global_key, TOMBSTONE)
        .map_err(|e| Error::storage(format!("Failed to delete global relation: {}", e)))?;

    Ok(())
}
