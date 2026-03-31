//! `add_node` - Optimized path for creating new nodes in a transaction
//!
//! Unlike `put_node`, this only handles CREATE (no update path).

use raisin_error::Result;
use raisin_models::nodes::Node;

use crate::transaction::RocksDBTransaction;

use super::super::{
    cache, indexing, metadata, ordering, references, storage, tracking, validation,
};
use super::rls;

/// Create a new node in the transaction (optimized for new nodes)
///
/// This is an optimized version of `put_node` for new nodes only.
/// It validates as CREATE and skips existence checks.
///
/// # Fast Path
///
/// Unlike `put_node`, this method:
/// - Only validates as CREATE (no existence check)
/// - Appends to end of ordered children (no existence check)
/// - Always tracks as Added operation
///
/// # Read-Your-Writes
///
/// When creating initial_structure children, the parent node may have been
/// created earlier in this same transaction and only exists in the write batch.
/// We check the transaction's read cache first for read-your-writes semantics.
pub async fn add_node(tx: &RocksDBTransaction, workspace: &str, node: &Node) -> Result<()> {
    // 1. Normalize parent field from path before saving
    let mut normalized_node = metadata::normalize_parent(node);

    // 2. Resolve path-based references
    references::resolve_references(tx, &mut normalized_node.properties, workspace).await?;

    // 3. Extract metadata (tenant, repo, branch)
    let (tenant_id, repo_id, branch) = metadata::extract_metadata(tx)?;

    // 3a. Check CREATE permission
    rls::check_create_permission(tx, &normalized_node, workspace)?;

    // 4. Check for path conflict in transaction cache (read-your-writes)
    let cached_existing =
        super::super::super::read::get_node_by_path(tx, workspace, &normalized_node.path).await?;
    if let Some(existing_node) = cached_existing {
        tracing::warn!(
            "ADD_NODE: Path conflict detected! Path '{}' already exists with id='{}', refusing to create duplicate with id='{}'",
            normalized_node.path,
            existing_node.id,
            normalized_node.id
        );
        return Err(raisin_error::Error::Conflict(format!(
            "Node with path '{}' already exists (id={})",
            normalized_node.path, existing_node.id
        )));
    }

    // 5. Validate as new node
    validation::validate_create(
        tx,
        &tenant_id,
        &repo_id,
        &branch,
        workspace,
        &normalized_node,
        false, // skip parent validation
    )
    .await?;

    // 5a. Schema validation
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

    // 6. Get or allocate the single transaction HLC
    let revision = tx.get_or_allocate_transaction_revision()?;

    tracing::info!(
        "TXN add_node: node_id={}, path={}, revision={}",
        normalized_node.id,
        normalized_node.path,
        revision
    );

    // 6. Update read cache for read-your-writes semantics
    cache::update_read_cache(tx, workspace, &normalized_node, None)?;

    // 7. Write node to batch
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

    // 8. Write path index (no tombstone for add_node)
    storage::write_path_index(
        tx,
        &tenant_id,
        &repo_id,
        &branch,
        workspace,
        &normalized_node.path,
        &normalized_node.id,
        &revision,
        None,
    )?;

    // 9. Index all properties
    indexing::index_node_properties(
        tx,
        &tenant_id,
        &repo_id,
        &branch,
        workspace,
        &normalized_node,
        &revision,
    )?;

    // 10. Index references
    indexing::index_node_references(
        tx,
        &tenant_id,
        &repo_id,
        &branch,
        workspace,
        &normalized_node,
        &revision,
    )?;

    // 10a. Index unique properties
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

    // 11. Add ORDERED_CHILDREN index entry (FAST PATH)
    let parent_id = ordering::lookup_parent_id(
        tx,
        &tenant_id,
        &repo_id,
        &branch,
        workspace,
        &normalized_node,
    )
    .await?;

    if let Some(parent_id_val) = parent_id {
        let order_label = ordering::add_ordered_child_fast(
            tx,
            &tenant_id,
            &repo_id,
            &branch,
            workspace,
            &parent_id_val,
            &normalized_node,
            &revision,
        )?;

        normalized_node.order_key = order_label;
    }

    // 12. Track creation
    tracking::track_create(tx, workspace, &normalized_node, revision)?;

    Ok(())
}
