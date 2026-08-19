//! Node move operations
//!
//! This module implements transaction-aware node move operations.
//! All changes (parent + descendants) are written to a single transaction batch
//! with a single revision, enabling proper event emission on commit.

use raisin_error::Result;
use raisin_models::tree::ChangeOperation;

use crate::tombstones::TOMBSTONE;
use crate::transaction::change_types::NodeChange;
use crate::transaction::RocksDBTransaction;
use crate::{cf, cf_handle, keys};
use raisin_models::nodes::Node;

/// Move an entire node tree to a new parent (transaction-aware)
///
/// This implementation:
/// - Reads all descendants using the storage layer
/// - Writes all path updates to the transaction's batch with a single revision
/// - Tracks changes for event emission via transaction commit
/// - Works with revision contexts (not just HEAD)
///
/// All changes are atomic and visible through the transaction's read cache.
pub async fn move_node_tree(
    tx: &RocksDBTransaction,
    workspace: &str,
    node_id: &str,
    new_path: &str,
) -> Result<()> {
    // 1. Get transaction metadata
    let (tenant_id, repo_id, branch) = {
        let meta = tx
            .metadata
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        (
            meta.tenant_id.clone(),
            meta.repo_id.clone(),
            meta.branch.clone().ok_or_else(|| {
                raisin_error::Error::Validation("Branch not set in transaction".into())
            })?,
        )
    };

    // 2. Get source node (target must exist in committed state for parent change)
    let source_node = super::read::get_node(tx, workspace, node_id)
        .await?
        .ok_or_else(|| raisin_error::Error::NotFound(format!("Node {} not found", node_id)))?;
    let old_root_path = source_node.path.clone();

    // 3. Use storage layer to read all descendants (synchronous, no batch conflicts)
    let node_repo = tx.node_repo.as_ref();
    let descendants = node_repo.scan_descendants_ordered_impl(
        &tenant_id, &repo_id, &branch, workspace, node_id,
        None, // Use committed state for reading
    )?;

    tracing::info!(
        "TXN move_node_tree: moving {} nodes from '{}' to '{}'",
        descendants.len(),
        old_root_path,
        new_path
    );

    // 4. Parse target parent and new name from new_path
    let (target_parent_path, _new_name) = new_path
        .rsplit_once('/')
        .map(|(parent, name)| {
            let parent_path = if parent.is_empty() {
                "/".to_string()
            } else {
                parent.to_string()
            };
            (parent_path, name.to_string())
        })
        .unwrap_or_else(|| ("/".to_string(), new_path.to_string()));

    // 5. Get target parent node (must exist in committed state)
    let target_parent = super::read::get_node_by_path(tx, workspace, &target_parent_path)
        .await?
        .ok_or_else(|| {
            raisin_error::Error::NotFound(format!(
                "Target parent '{}' not found",
                target_parent_path
            ))
        })?;

    // Get old parent info BEFORE locking batch (to avoid holding non-Send lock across await)
    let old_parent_id = if let Some(source_parent_path) = source_node
        .path
        .rsplit_once('/')
        .map(|(p, _)| if p.is_empty() { "/" } else { p })
    {
        if let Ok(Some(old_parent)) =
            super::read::get_node_by_path(tx, workspace, source_parent_path).await
        {
            Some(old_parent.id.clone())
        } else {
            None
        }
    } else {
        None
    };

    // 5a. Get old order label BEFORE locking batch (uses storage layer)
    let old_order_label = if let Some(ref old_parent) = old_parent_id {
        node_repo.get_order_label_for_child(
            &tenant_id, &repo_id, &branch, workspace, old_parent, node_id,
        )?
    } else {
        None
    };

    // 5b. Compute new order label BEFORE locking batch
    let new_order_label = {
        // Check if child already exists in new parent (shouldn't, but handle gracefully)
        let existing = node_repo.get_order_label_for_child(
            &tenant_id,
            &repo_id,
            &branch,
            workspace,
            &target_parent.id,
            node_id,
        )?;
        if let Some(label) = existing {
            label
        } else {
            // Get last order label and increment
            let last = node_repo.get_last_order_label(
                &tenant_id,
                &repo_id,
                &branch,
                workspace,
                &target_parent.id,
            )?;
            if let Some(ref l) = last {
                crate::fractional_index::inc(l).unwrap_or_else(|_| crate::fractional_index::first())
            } else {
                crate::fractional_index::first()
            }
        }
    };

    // 6. Get or allocate transaction revision
    let revision = tx.get_or_allocate_transaction_revision()?;

    // 7. Lock batch and write all path updates
    let mut batch = tx
        .batch
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

    let cf_nodes = cf_handle(&tx.db, cf::NODES)?;
    let cf_path = cf_handle(&tx.db, cf::PATH_INDEX)?;
    let cf_node_path = cf_handle(&tx.db, cf::NODE_PATH)?;
    let cf_ordered = cf_handle(&tx.db, cf::ORDERED_CHILDREN)?;

    // 7a. Tombstone old ORDERED_CHILDREN entry (remove from old parent)
    if let (Some(ref old_parent), Some(ref old_label)) = (&old_parent_id, &old_order_label) {
        let old_ordered_key = keys::ordered_child_key_versioned(
            &tenant_id, &repo_id, &branch, workspace, old_parent, old_label, &revision, node_id,
        );
        batch.put_cf(cf_ordered, old_ordered_key, TOMBSTONE);

        // Invalidate old parent's cached metadata
        let old_metadata_key =
            keys::last_child_metadata_key(&tenant_id, &repo_id, &branch, workspace, old_parent);
        batch.delete_cf(cf_ordered, old_metadata_key);
    }

    // 7b. Add new ORDERED_CHILDREN entry (add to new parent)
    let new_name = new_path
        .rsplit_once('/')
        .map(|(_, n)| n)
        .unwrap_or(new_path);
    let new_ordered_key = keys::ordered_child_key_versioned(
        &tenant_id,
        &repo_id,
        &branch,
        workspace,
        &target_parent.id,
        &new_order_label,
        &revision,
        node_id,
    );
    batch.put_cf(cf_ordered, new_ordered_key, new_name.as_bytes());

    // Update new parent's cached last-child metadata
    let new_metadata_key =
        keys::last_child_metadata_key(&tenant_id, &repo_id, &branch, workspace, &target_parent.id);
    batch.put_cf(cf_ordered, new_metadata_key, new_order_label.as_bytes());

    // Track moved node IDs for change tracking
    let mut moved_node_ids = Vec::new();

    // (old_path, new_path, node_id) per moved node, so the read cache can be
    // updated in step 9 — both halves of the move, not just the vacated path.
    let mut moved_paths: Vec<(String, String, String)> = Vec::new();

    // 8. For each node (root + descendants): update paths
    for (node, depth) in &descendants {
        moved_node_ids.push(node.id.clone());

        // Calculate new path for this node
        let node_new_path = if *depth == 0 {
            // Root node gets the new_path exactly
            new_path.to_string()
        } else {
            // Descendant nodes: replace old root prefix with new root prefix
            let relative = node
                .path
                .strip_prefix(&format!("{}/", old_root_path))
                .unwrap_or(&node.path);
            format!("{}/{}", new_path, relative)
        };

        tracing::debug!(
            "TXN move_node_tree: updating node path: {} → {}",
            node.path,
            node_new_path
        );

        // Tombstone old PATH_INDEX
        let old_path_key = keys::path_index_key_versioned(
            &tenant_id, &repo_id, &branch, workspace, &node.path, &revision,
        );
        batch.put_cf(cf_path, old_path_key, TOMBSTONE);

        // Write new PATH_INDEX
        let new_path_key = keys::path_index_key_versioned(
            &tenant_id,
            &repo_id,
            &branch,
            workspace,
            &node_new_path,
            &revision,
        );
        batch.put_cf(cf_path, new_path_key, node.id.as_bytes());

        // Write new NODE_PATH
        let node_path_key = keys::node_path_key_versioned(
            &tenant_id, &repo_id, &branch, workspace, &node.id, &revision,
        );
        batch.put_cf(cf_node_path, node_path_key, node_new_path.as_bytes());

        moved_paths.push((node.path.clone(), node_new_path.clone(), node.id.clone()));

        // A move is mostly index-only, because `Node` stores its parent's NAME
        // rather than a path. Two kinds of node still go stale and need their
        // blob rewritten:
        //
        //   * the moved ROOT — new `name` (on rename), new `parent`, new
        //     `order_key`;
        //   * its DIRECT CHILDREN, but only on a RENAME, since they hold the
        //     root's old name in `parent`.
        //
        // Without this a transactional rename left `node.name` reporting the old
        // name forever, even though every path had been updated around it.
        let updated_name = node_new_path
            .rsplit('/')
            .next()
            .unwrap_or(&node_new_path)
            .to_string();
        let updated_parent = Node::extract_parent_name_from_path(&node_new_path);
        let is_root = *depth == 0;

        if is_root || node.name != updated_name || node.parent != updated_parent {
            let mut rewritten = node.clone();
            rewritten.path = node_new_path.clone();
            rewritten.name = updated_name;
            rewritten.parent = updated_parent;
            rewritten.updated_at = Some(chrono::Utc::now());
            if is_root {
                rewritten.order_key = new_order_label.clone();
            }

            let node_key = keys::node_key_versioned(
                &tenant_id,
                &repo_id,
                &branch,
                workspace,
                &rewritten.id,
                &revision,
            );
            let node_value = rmp_serde::to_vec_named(&rewritten)
                .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;
            batch.put_cf(cf_nodes, node_key, node_value);
        }
    }

    // 9. Update read cache for read-your-writes semantics
    {
        let mut cache = tx
            .read_cache
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

        // Vacate the old paths first, THEN claim the new ones. Doing it in one
        // pass would let a node moved onto a sibling's vacated path be erased
        // again by that sibling's removal.
        for (old_path, _, _) in &moved_paths {
            cache
                .paths
                .insert((workspace.to_string(), old_path.clone()), None);
        }

        // Register the new paths. Without this the moved node was unreachable
        // by path for the REST OF THE TRANSACTION — the batch had already
        // written the new PATH_INDEX entry, but an in-transaction read still
        // resolved through the cache and saw nothing. A caller that moved a
        // node and then upserted it at its new path therefore took the CREATE
        // branch and minted a duplicate, which for a node type with a
        // `unique: true` property failed the whole write.
        for (_, new_path, node_id) in &moved_paths {
            cache.paths.insert(
                (workspace.to_string(), new_path.clone()),
                Some(node_id.clone()),
            );
        }
    }

    // 10. Track changes for event emission during commit
    {
        let mut changed = tx
            .changed_nodes
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

        for (node, _) in &descendants {
            // Track as "Modified" operation (path changed)
            changed.insert(
                node.id.clone(),
                NodeChange {
                    workspace: workspace.to_string(),
                    revision,
                    operation: ChangeOperation::Modified,
                    path: Some(node.path.clone()), // Store path before move for event matching
                    node_type: Some(node.node_type.clone()),
                },
            );
        }
    }

    // Track move operations for replication (source node only)
    {
        let mut tracker = tx
            .change_tracker
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

        tracker.track_move(
            source_node.id.clone(),
            workspace.to_string(),
            revision,
            old_parent_id,
            Some(target_parent.id.clone()),
            None, // Order label not computed in transaction
        );
    }

    tracing::info!(
        "TXN move_node_tree: wrote {} path updates to transaction batch (single revision)",
        moved_node_ids.len() * 3 // PATH_INDEX tombstone + new PATH_INDEX + NODE_PATH
    );

    Ok(())
}
