//! Bulk deletion operations for relations
//!
//! This module implements remove_all_relations_for_node which removes
//! all outgoing and incoming relations for a specific node.

use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use rocksdb::DB;
use std::sync::Arc;

use crate::keys::{relation_forward_prefix, relation_reverse_prefix};

use super::crud::write_relation_tombstones;
use super::helpers::{deserialize_relation_ref, get_relation_cf, is_tombstone};

/// Remove all relations (both outgoing and incoming) for a node
pub(super) async fn remove_all_relations_for_node(
    db: &Arc<DB>,
    revision: &HLC,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
) -> Result<()> {
    // Get relation column family handle
    let cf_relation = get_relation_cf(db)?;

    // Step 1: Remove all outgoing relationships FROM this node
    let outgoing_to_remove =
        collect_outgoing_relations(db, tenant_id, repo_id, branch, workspace, node_id)?;

    // Create tombstones for outgoing relations and their reverse indexes
    for (relation_type, target_workspace, target_id) in outgoing_to_remove {
        write_relation_tombstones(
            db,
            revision,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            &target_workspace,
            &target_id,
            &relation_type,
        )?;
    }

    // Step 2: Remove all incoming relationships TO this node
    let incoming_to_remove =
        collect_incoming_relations(db, tenant_id, repo_id, branch, workspace, node_id)?;

    // Create tombstones for incoming relations and their forward indexes
    for (relation_type, source_workspace, source_id) in incoming_to_remove {
        write_relation_tombstones(
            db,
            revision,
            tenant_id,
            repo_id,
            branch,
            &source_workspace,
            &source_id,
            workspace,
            node_id,
            &relation_type,
        )?;
    }

    Ok(())
}

/// Collect information about all outgoing relations from a node
fn collect_outgoing_relations(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
) -> Result<Vec<(String, String, String)>> {
    let cf_relation = get_relation_cf(db)?;
    let outgoing_prefix = relation_forward_prefix(tenant_id, repo_id, branch, workspace, node_id);
    let mut outgoing_to_remove = Vec::new();

    let iter = db.prefix_iterator_cf(cf_relation, &outgoing_prefix);
    for item in iter {
        let (key, value) = item
            .map_err(|e| Error::storage(format!("Failed to iterate outgoing relations: {}", e)))?;

        if !key.starts_with(&outgoing_prefix) {
            break;
        }

        // Skip if already tombstone
        if is_tombstone(&value) {
            continue;
        }

        // Parse key to get relation_type and target info
        // Key structure: {tenant}\0{repo}\0{branch}\0{workspace}\0rel\0{source_node_id}\0{relation_type}\0{~revision}\0{target_node_id}
        let key_parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
        if key_parts.len() >= 9 {
            let relation_type = String::from_utf8_lossy(key_parts[6]).to_string();
            let target_id = String::from_utf8_lossy(key_parts[8]).to_string();

            // Deserialize to get target workspace
            let relation = deserialize_relation_ref(&value)?;

            outgoing_to_remove.push((relation_type, relation.workspace, target_id));
        }
    }

    Ok(outgoing_to_remove)
}

/// Collect information about all incoming relations to a node
fn collect_incoming_relations(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
) -> Result<Vec<(String, String, String)>> {
    let cf_relation = get_relation_cf(db)?;
    let incoming_prefix = relation_reverse_prefix(tenant_id, repo_id, branch, workspace, node_id);
    let mut incoming_to_remove = Vec::new();

    let iter = db.prefix_iterator_cf(cf_relation, &incoming_prefix);
    for item in iter {
        let (key, value) = item
            .map_err(|e| Error::storage(format!("Failed to iterate incoming relations: {}", e)))?;

        if !key.starts_with(&incoming_prefix) {
            break;
        }

        // Skip if already tombstone
        if is_tombstone(&value) {
            continue;
        }

        // Parse key to get relation_type and source info
        // Key structure: {tenant}\0{repo}\0{branch}\0{workspace}\0rel_rev\0{target_node_id}\0{relation_type}\0{~revision}\0{source_node_id}
        let key_parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
        if key_parts.len() >= 9 {
            let relation_type = String::from_utf8_lossy(key_parts[6]).to_string();
            let source_id = String::from_utf8_lossy(key_parts[8]).to_string();

            // For incoming relations, source_workspace is the same as current workspace
            // (assuming relations are within same workspace - cross-workspace is TODO)
            incoming_to_remove.push((relation_type, workspace.to_string(), source_id));
        }
    }

    Ok(incoming_to_remove)
}
