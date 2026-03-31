//! `put_node` - Create or update a node in the transaction
//!
//! Handles both creates and updates by checking for existing nodes.

use raisin_error::Result;
use raisin_models::nodes::Node;

use crate::transaction::RocksDBTransaction;

use super::super::{
    cache, indexing, metadata, ordering, references, storage, tracking, validation,
};
use super::rls;

/// Create or update a node in the transaction
///
/// This method handles both creates and updates:
/// - If the node doesn't exist, validates as CREATE
/// - If the node exists, validates as UPDATE
pub async fn put_node(tx: &RocksDBTransaction, workspace: &str, node: &Node) -> Result<()> {
    // 1. Normalize parent field from path before saving
    let mut normalized_node = metadata::normalize_parent(node);

    // 2. Resolve path-based references (converts paths to UUIDs, populates raisin:path)
    references::resolve_references(tx, &mut normalized_node.properties, workspace).await?;

    // 3. Extract metadata (tenant, repo, branch)
    let (tenant_id, repo_id, branch) = metadata::extract_metadata(tx)?;

    // 4. Check if this is a create or update operation
    let existing_node = super::super::super::read::get_node(tx, workspace, &node.id).await?;

    tracing::info!(
        node_id = %normalized_node.id,
        path = %normalized_node.path,
        workspace = workspace,
        is_new = existing_node.is_none(),
        "TRANSACTION: put_node called"
    );

    // 4a. Check RLS permission
    rls::check_put_permission(tx, &normalized_node, existing_node.as_ref(), workspace)?;

    // 5. Validate based on operation type
    if existing_node.is_none() {
        tracing::info!(
            node_id = %normalized_node.id,
            node_type = %normalized_node.node_type,
            "TRANSACTION: Detected NEW NODE - will track create operation"
        );
        validation::validate_create(
            tx,
            &tenant_id,
            &repo_id,
            &branch,
            workspace,
            &normalized_node,
            true, // validate parent
        )
        .await?;
    } else {
        validation::validate_update_with_existing(
            existing_node.as_ref().unwrap(),
            &normalized_node,
        )?;
    }

    // 5a. Schema validation against NodeType/Archetype/ElementType
    if tx.is_validate_schema_enabled() {
        let validator = tx.create_validator();
        validator.validate_node(workspace, &normalized_node).await?;
    }

    // 5b. Check unique property constraints
    validation::check_unique_constraints(
        tx,
        &tenant_id,
        &repo_id,
        &branch,
        workspace,
        &normalized_node,
    )
    .await?;

    // 6. Detect changes
    let (parent_changed, path_changed, old_parent, old_path) =
        validation::detect_changes(existing_node.as_ref(), &normalized_node);

    // 7. Get or allocate the single transaction HLC
    let revision = tx.get_or_allocate_transaction_revision()?;

    tracing::info!(
        "TXN put_node: node_id={}, old_path={:?}, new_path={}, path_changed={}, revision={}",
        normalized_node.id,
        old_path,
        normalized_node.path,
        path_changed,
        revision
    );

    // 7. Update read cache for read-your-writes semantics
    cache::update_read_cache(
        tx,
        workspace,
        &normalized_node,
        old_path.as_deref().filter(|_| path_changed),
    )?;

    // 8. Write node to batch
    let node_key = storage::write_node_to_batch(
        tx,
        &tenant_id,
        &repo_id,
        &branch,
        workspace,
        &normalized_node,
        &revision,
    )?;
    tx.record_write(node_key)?;

    // 9. Write path index
    storage::write_path_index(
        tx,
        &tenant_id,
        &repo_id,
        &branch,
        workspace,
        &normalized_node.path,
        &normalized_node.id,
        &revision,
        old_path.as_deref().filter(|_| path_changed),
    )?;

    // 10. Index all properties
    indexing::index_node_properties(
        tx,
        &tenant_id,
        &repo_id,
        &branch,
        workspace,
        &normalized_node,
        &revision,
    )?;

    // 11. Index references
    indexing::index_node_references(
        tx,
        &tenant_id,
        &repo_id,
        &branch,
        workspace,
        &normalized_node,
        &revision,
    )?;

    // 11a. Handle unique index updates
    if let Some(ref old_node) = existing_node {
        indexing::tombstone_unique_properties(
            tx, &tenant_id, &repo_id, &branch, workspace, old_node, &revision,
        )
        .await?;
    }
    indexing::index_unique_properties(
        tx,
        &tenant_id,
        &repo_id,
        &branch,
        workspace,
        &normalized_node,
        &revision,
    )
    .await?;

    // 12. Handle ORDERED_CHILDREN index
    let parent_id = ordering::lookup_parent_id(
        tx,
        &tenant_id,
        &repo_id,
        &branch,
        workspace,
        &normalized_node,
    )
    .await?;

    // 12a. If parent changed, tombstone old ordering entry
    if parent_changed {
        if let Some(_old_p) = old_parent.as_ref() {
            let existing = existing_node.as_ref().ok_or_else(|| {
                raisin_error::Error::storage(
                    "Internal error: parent_changed is true but existing_node is None",
                )
            })?;

            if let Some(old_parent_id) = ordering::lookup_old_parent_id(
                tx, &tenant_id, &repo_id, &branch, workspace, existing,
            )
            .await?
            {
                ordering::tombstone_old_ordering(
                    tx,
                    &tenant_id,
                    &repo_id,
                    &branch,
                    workspace,
                    &old_parent_id,
                    &normalized_node.id,
                    &revision,
                )?;
            }
        }
    }

    // 12b. Add or update ordering entry
    if let Some(parent_id_val) = parent_id {
        let (order_label, is_new_node) = ordering::add_ordered_child(
            tx,
            &tenant_id,
            &repo_id,
            &branch,
            workspace,
            &parent_id_val,
            &normalized_node,
            &revision,
        )?;

        normalized_node.order_key = order_label.clone();

        // 13. Track changes
        if is_new_node {
            tracking::track_create(tx, workspace, &normalized_node, revision)?;
        } else if let Some(ref old_node) = existing_node {
            tracking::track_update(tx, workspace, old_node, &normalized_node, revision)?;

            if parent_changed {
                tracking::track_move(
                    tx,
                    workspace,
                    &normalized_node.id,
                    old_parent,
                    normalized_node.parent.clone(),
                    Some(order_label),
                    revision,
                )?;
            }
        }
    } else {
        tracking::track_orphaned(
            tx,
            workspace,
            existing_node.as_ref(),
            &normalized_node,
            revision,
        )?;
    }

    Ok(())
}
