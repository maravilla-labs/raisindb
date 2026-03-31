//! Relation operations for transactions
//!
//! This module implements transactional relation operations with:
//! - Forward, reverse, and global index updates
//! - WriteBatch integration for atomic commits
//! - ChangeTracker integration for CRDT replication
//! - MVCC tombstone semantics for deletions
//!
//! # Operations
//!
//! - `add_relation`: Add a relationship between two nodes
//! - `remove_relation`: Remove all relationships between two nodes
//!
//! # CRDT Semantics
//!
//! Relations use Last-Write-Wins (LWW) CRDT semantics based on the composite key
//! (source_id, target_id, relation_type). Only one relation of a given type can exist
//! between two nodes. Concurrent updates are resolved using HLC timestamps.

use raisin_error::{Error, Result};
use raisin_models::nodes::{FullRelation, RelationRef};

use crate::transaction::RocksDBTransaction;
use crate::{cf_handle, keys};

/// Add a relationship from source node to target node within the transaction
///
/// Creates versioned entries in three indexes:
/// - Forward index: Outgoing relationships FROM source TO target
/// - Reverse index: Incoming relationships TO target FROM source
/// - Global index: Cross-workspace query support for Cypher
///
/// The relation is tracked via ChangeTracker for CRDT replication.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `source_workspace` - Workspace containing the source node
/// * `source_node_id` - ID of the source node
/// * `source_node_type` - Node type of the source (e.g., "raisin:Page")
/// * `relation` - RelationRef containing target details (type, workspace, id)
///
/// # Returns
///
/// Ok(()) on success, Error on storage failure
pub async fn add_relation(
    tx: &RocksDBTransaction,
    source_workspace: &str,
    source_node_id: &str,
    source_node_type: &str,
    relation: RelationRef,
) -> Result<()> {
    // 1. Get metadata
    let (tenant_id, repo_id, branch) = {
        let meta = tx
            .metadata
            .lock()
            .map_err(|e| Error::storage(format!("Failed to lock metadata: {}", e)))?;

        let branch = meta.branch.as_ref().ok_or_else(|| {
            Error::storage("Branch not set in transaction. Call set_branch() first.".to_string())
        })?;

        (
            meta.tenant_id.to_string(),
            meta.repo_id.to_string(),
            branch.to_string(),
        )
    };

    // 2. Get or allocate the single transaction HLC (all operations in tx share same revision)
    let revision = tx.get_or_allocate_transaction_revision()?;

    tracing::warn!(
        "🔍 RELATION DEBUG: add_relation called, allocated revision={}",
        revision
    );

    // 2. Serialize relation data for indexes
    let forward_relation_bytes = rmp_serde::to_vec(&relation)
        .map_err(|e| Error::storage(format!("Failed to serialize relation: {}", e)))?;

    // Create reverse RelationRef (stores source info as if it were the target)
    // This allows incoming relationship queries to know the source node's type
    let reverse_relation = RelationRef::new(
        source_node_id.to_string(),   // target = source (from reverse perspective)
        source_workspace.to_string(), // workspace of source
        source_node_type.to_string(), // target_node_type = source's type
        relation.relation_type.clone(), // same relation type
        relation.weight,              // same weight
    );
    let reverse_relation_bytes = rmp_serde::to_vec(&reverse_relation)
        .map_err(|e| Error::storage(format!("Failed to serialize reverse relation: {}", e)))?;

    // Create FullRelation for global index (includes both source and target info WITH node types)
    let full_relation = FullRelation::from_source_and_ref(
        source_node_id.to_string(),
        source_workspace.to_string(),
        source_node_type.to_string(),
        &relation,
    );
    let full_relation_bytes = rmp_serde::to_vec(&full_relation)
        .map_err(|e| Error::storage(format!("Failed to serialize full relation: {}", e)))?;

    // 3. Create versioned keys for all three indexes
    let forward_key = keys::relation_forward_key_versioned(
        &tenant_id,
        &repo_id,
        &branch,
        source_workspace,
        source_node_id,
        &relation.relation_type,
        &revision,
        &relation.target,
    );

    let reverse_key = keys::relation_reverse_key_versioned(
        &tenant_id,
        &repo_id,
        &branch,
        &relation.workspace,
        &relation.target,
        &relation.relation_type,
        &revision,
        source_node_id,
    );

    let global_key = keys::relation_global_key_versioned(
        &tenant_id,
        &repo_id,
        &branch,
        &relation.relation_type,
        &revision,
        source_workspace,
        source_node_id,
        &relation.workspace,
        &relation.target,
    );

    // 4. Get column family handle
    let cf_relation = cf_handle(&tx.db, crate::cf::RELATION_INDEX)?;

    // 5. Write all three indexes to WriteBatch atomically
    {
        let mut batch = tx
            .batch
            .lock()
            .map_err(|e| Error::storage(format!("Failed to lock batch: {}", e)))?;

        batch.put_cf(cf_relation, &forward_key, &forward_relation_bytes);
        batch.put_cf(cf_relation, &reverse_key, &reverse_relation_bytes);
        batch.put_cf(cf_relation, &global_key, &full_relation_bytes);
    }

    // 6. Track relation addition for CRDT replication
    {
        let mut tracker = tx
            .change_tracker
            .lock()
            .map_err(|e| Error::storage(format!("Failed to lock change_tracker: {}", e)))?;

        tracker.track_relation_add(
            source_node_id.to_string(),
            source_workspace.to_string(),
            revision,
            relation.relation_type.clone(),
            relation.target.clone(),
            relation.workspace.clone(),
            relation.clone(),
        );
    }

    Ok(())
}

/// Remove all relationships between source and target nodes within the transaction
///
/// Scans the forward index to find all relations from source to target, then writes
/// tombstones to all three indexes (forward, reverse, global) for each relation found.
///
/// Since the method doesn't specify relation_type, this removes ALL relations
/// between the two nodes.
///
/// Each removed relation is tracked via ChangeTracker for CRDT replication.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `source_workspace` - Workspace containing the source node
/// * `source_node_id` - ID of the source node
/// * `target_workspace` - Workspace containing the target node
/// * `target_node_id` - ID of the target node
///
/// # Returns
///
/// Ok(true) if relations were found and removed, Ok(false) if no relations found
pub async fn remove_relation(
    tx: &RocksDBTransaction,
    source_workspace: &str,
    source_node_id: &str,
    target_workspace: &str,
    target_node_id: &str,
) -> Result<bool> {
    // 1. Get metadata
    let (tenant_id, repo_id, branch) = {
        let meta = tx
            .metadata
            .lock()
            .map_err(|e| Error::storage(format!("Failed to lock metadata: {}", e)))?;

        let branch = meta.branch.as_ref().ok_or_else(|| {
            Error::storage("Branch not set in transaction. Call set_branch() first.".to_string())
        })?;

        (
            meta.tenant_id.to_string(),
            meta.repo_id.to_string(),
            branch.to_string(),
        )
    };

    // 2. Get or allocate the single transaction HLC (all operations in tx share same revision)
    let revision = tx.get_or_allocate_transaction_revision()?;

    // 2. Get column family handle
    let cf_relation = cf_handle(&tx.db, crate::cf::RELATION_INDEX)?;

    // 3. Scan forward index to find all relations from source to target
    let prefix = keys::relation_forward_prefix(
        &tenant_id,
        &repo_id,
        &branch,
        source_workspace,
        source_node_id,
    );

    let mut found_any = false;
    let mut relations_to_remove = Vec::new();

    // Scan to find all relations from source to target
    let iter = tx.db.prefix_iterator_cf(cf_relation, &prefix);
    for item in iter {
        let (key, value) =
            item.map_err(|e| Error::storage(format!("Failed to iterate relations: {}", e)))?;

        if !key.starts_with(&prefix) {
            break;
        }

        // Skip tombstones
        if value.as_ref() == b"T" {
            continue;
        }

        // Deserialize to check if this relation points to our target
        let relation: RelationRef = rmp_serde::from_slice(&value)
            .map_err(|e| Error::storage(format!("Failed to deserialize relation: {}", e)))?;

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

    // 4. Write tombstones for all matching relations
    {
        let mut batch = tx
            .batch
            .lock()
            .map_err(|e| Error::storage(format!("Failed to lock batch: {}", e)))?;

        for relation_type in &relations_to_remove {
            // Create tombstone keys for all three indexes
            let forward_key = keys::relation_forward_key_versioned(
                &tenant_id,
                &repo_id,
                &branch,
                source_workspace,
                source_node_id,
                relation_type,
                &revision,
                target_node_id,
            );

            let reverse_key = keys::relation_reverse_key_versioned(
                &tenant_id,
                &repo_id,
                &branch,
                target_workspace,
                target_node_id,
                relation_type,
                &revision,
                source_node_id,
            );

            let global_key = keys::relation_global_key_versioned(
                &tenant_id,
                &repo_id,
                &branch,
                relation_type,
                &revision,
                source_workspace,
                source_node_id,
                target_workspace,
                target_node_id,
            );

            // Write tombstones to all three indexes
            batch.put_cf(cf_relation, &forward_key, b"T");
            batch.put_cf(cf_relation, &reverse_key, b"T");
            batch.put_cf(cf_relation, &global_key, b"T");
        }
    }

    // 5. Track relation removals for CRDT replication
    {
        let mut tracker = tx
            .change_tracker
            .lock()
            .map_err(|e| Error::storage(format!("Failed to lock change_tracker: {}", e)))?;

        eprintln!(
            "🗑️  REMOVE_RELATION: Tracking {} relation removals for replication",
            relations_to_remove.len()
        );

        for relation_type in relations_to_remove {
            eprintln!(
                "🗑️  Tracking RemoveRelation: {} --[{}]--> {}",
                source_node_id, relation_type, target_node_id
            );
            tracker.track_relation_remove(
                source_node_id.to_string(),
                source_workspace.to_string(),
                revision,
                relation_type.clone(),
                target_node_id.to_string(),
                target_workspace.to_string(),
            );
        }
    }

    Ok(found_any)
}
