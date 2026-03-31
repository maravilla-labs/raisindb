//! Query operations for relations
//!
//! This module implements relation query operations:
//! - get_outgoing_relations: Get all relations FROM a node
//! - get_incoming_relations: Get all relations TO a node
//! - get_relations_by_type: Get relations filtered by target node type

use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use raisin_models::nodes::RelationRef;
use rocksdb::DB;
use std::collections::HashSet;
use std::sync::Arc;

use crate::keys::{relation_forward_prefix, relation_reverse_prefix};

use super::helpers::{
    deserialize_relation_ref, get_relation_cf, is_tombstone, parse_forward_key, parse_reverse_key,
};

/// Get all outgoing relations from a node
pub(super) async fn get_outgoing_relations(
    db: &Arc<DB>,
    max_revision: &HLC,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
) -> Result<Vec<RelationRef>> {
    // Get relation column family handle
    let cf_relation = get_relation_cf(db)?;

    // Get prefix for all outgoing relations from this node
    let prefix = relation_forward_prefix(tenant_id, repo_id, branch, workspace, node_id);

    let mut relations = Vec::new();
    let mut seen_targets = HashSet::new();

    tracing::debug!(
        "🔍 get_outgoing_relations: node_id={}, max_rev={}",
        node_id,
        max_revision
    );

    // Scan all relations with this prefix
    let iter = db.prefix_iterator_cf(cf_relation, &prefix);
    for item in iter {
        let (key, value) =
            item.map_err(|e| Error::storage(format!("Failed to iterate relations: {}", e)))?;

        // Check if key still matches prefix
        if !key.starts_with(&prefix) {
            break;
        }

        // Parse key components
        let components = parse_forward_key(&key)?;

        tracing::debug!(
            "  Found relation: type={}, target={}, rev={}, value_len={}, is_tombstone={}",
            components.relation_type,
            components.target_id,
            components.revision,
            value.len(),
            is_tombstone(&value)
        );

        // Skip if revision is newer than max_revision
        if &components.revision > max_revision {
            tracing::debug!(
                "  SKIP: revision {:?} > max_rev {:?}",
                components.revision,
                max_revision
            );
            continue;
        }

        // Create unique key for this target to detect duplicates (only take newest)
        // Use relation_type + target to distinguish different relation types to same node
        let target_key = format!("{}:{}", components.relation_type, components.target_id);

        // Skip if we've already seen this target (we only want the newest revision)
        if seen_targets.contains(&target_key) {
            tracing::debug!("  SKIP: already seen {}", target_key);
            continue;
        }

        // Check for tombstones - if this is a tombstone, mark as seen and skip
        // This ensures that older non-tombstoned versions of the same relation are not returned
        if is_tombstone(&value) {
            tracing::debug!("  TOMBSTONE: marking {} as deleted", target_key);
            seen_targets.insert(target_key); // Mark as deleted so we skip older versions
            continue;
        }

        // Deserialize the relation
        let relation = deserialize_relation_ref(&value)?;

        relations.push(relation);
        seen_targets.insert(target_key);
    }

    Ok(relations)
}

/// Get all incoming relations to a node
pub(super) async fn get_incoming_relations(
    db: &Arc<DB>,
    max_revision: &HLC,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
) -> Result<Vec<(String, String, RelationRef)>> {
    // Get relation column family handle
    let cf_relation = get_relation_cf(db)?;

    // Get prefix for all incoming relations to this node
    let prefix = relation_reverse_prefix(tenant_id, repo_id, branch, workspace, node_id);

    let mut relations = Vec::new();
    let mut seen_sources = HashSet::new();

    // Scan all relations with this prefix
    let iter = db.prefix_iterator_cf(cf_relation, &prefix);
    for item in iter {
        let (key, value) =
            item.map_err(|e| Error::storage(format!("Failed to iterate relations: {}", e)))?;

        // Check if key still matches prefix
        if !key.starts_with(&prefix) {
            break;
        }

        // Parse key components
        let components = parse_reverse_key(&key)?;

        // Skip if revision is newer than max_revision
        if &components.revision > max_revision {
            continue;
        }

        // Create unique key for this source to detect duplicates (only take newest)
        let source_key = format!("{}:{}", components.relation_type, components.source_id);

        // Skip if we've already seen this source (we only want the newest revision)
        if seen_sources.contains(&source_key) {
            continue;
        }

        // Check for tombstones - if this is a tombstone, mark as seen and skip
        if is_tombstone(&value) {
            seen_sources.insert(source_key); // Mark as deleted so we skip older versions
            continue;
        }

        // Deserialize the relation (note: this is stored from the source node's perspective,
        // so the workspace in RelationRef is the target workspace, which is the current node's workspace)
        let relation = deserialize_relation_ref(&value)?;

        // For incoming relations, we need to return the source workspace and source ID
        // The source workspace is not in the RelationRef (which stores target info),
        // so we need to infer it. Since we're querying from a specific workspace/branch,
        // the source must be in the same workspace context.
        // TODO: This might need revision if cross-workspace relations are supported
        let source_workspace = workspace.to_string();

        relations.push((source_workspace, components.source_id, relation));
        seen_sources.insert(source_key);
    }

    Ok(relations)
}

/// Get outgoing relations filtered by target node type
pub(super) async fn get_relations_by_type(
    db: &Arc<DB>,
    max_revision: &HLC,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    target_node_type: &str,
) -> Result<Vec<RelationRef>> {
    // Get all outgoing relations and filter by type
    let all_relations = get_outgoing_relations(
        db,
        max_revision,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
    )
    .await?;

    // Filter by target node type
    let filtered = all_relations
        .into_iter()
        .filter(|rel| rel.relation_type == target_node_type)
        .collect();

    Ok(filtered)
}
