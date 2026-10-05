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
    // ...as THIS transaction left each of them (see `move_view`).
    let descendants = super::move_view::transaction_view(tx, workspace, descendants)?;

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

    // 5. Get the target parent's id (it must exist in committed state) — EXCEPT
    // the WORKSPACE ROOT, which is a legitimate destination that stores no node
    // of its own. Root-level nodes are indexed with `parent_id = "/"` (see
    // listing.rs), so that literal IS the parent id here and every ordered-children
    // key below lands in the right place.
    let target_parent_id = if target_parent_path == "/" {
        "/".to_string()
    } else {
        super::read::get_node_by_path(tx, workspace, &target_parent_path)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!(
                    "Target parent '{}' not found",
                    target_parent_path
                ))
            })?
            .id
    };

    // A DIFFERENT node already at the destination is a CONFLICT, not a move.
    //
    // Nothing below checks this. The batch tombstones the old PATH_INDEX key and
    // writes `new_path -> node.id`, and PATH_INDEX holds exactly one mapping per
    // (path, revision) with the id in the VALUE — so moving onto an occupied path
    // silently overwrites the occupant's mapping and strands it: blob and every
    // other index entry still live, reachable by a table scan and by nothing else.
    // (The `get_order_label_for_child` probe further down looks like this check
    // and is not: it asks whether THIS node already has a slot under the new
    // parent.)
    //
    // That matters most where moves are automatic. The package installer adopts a
    // node found at the legacy `properties.name` path by moving it onto the
    // path-derived one, so a tenant holding nodes at BOTH paths silently loses one
    // — and once a path is unresolvable, `get_node_by_path` reports "nothing here"
    // and the next install CREATES a duplicate, which drifts the index further.
    // Measured on a live tenant: one package's content reached four copies.
    //
    // Refuse instead. A caller that means "replace" can delete first and say so.
    if let Some(occupant) = super::read::get_node_by_path(tx, workspace, new_path).await? {
        if occupant.id != source_node.id {
            return Err(raisin_error::Error::Conflict(format!(
                "Cannot move '{}' to '{}': node '{}' already occupies that path",
                old_root_path, new_path, occupant.id
            )));
        }
    }

    // Get old parent info BEFORE locking batch (to avoid holding non-Send lock across await)
    //
    // The workspace root has no stored node; its ORDERED_CHILDREN are keyed by
    // the literal "/", as `target_parent_id` is above. Looking "/" up as a node
    // found nothing, so a node moved OUT of the root kept its root entry and
    // went on being listed there.
    let old_parent_id = if let Some(source_parent_path) = source_node
        .path
        .rsplit_once('/')
        .map(|(p, _)| if p.is_empty() { "/" } else { p })
    {
        if source_parent_path == "/" {
            Some("/".to_string())
        } else if let Ok(Some(old_parent)) =
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

    // 6. Get or allocate transaction revision — before the label, which
    // carries it as its `::HLC` suffix like every other minted label.
    let revision = tx.get_or_allocate_transaction_revision()?;

    // 5b. Compute new order label BEFORE locking batch. `appended` says whether
    // it was minted at the end of the new parent (only then is it the parent's
    // new LAST label).
    let (new_order_label, appended) = {
        // Check if child already exists in new parent (shouldn't, but handle gracefully)
        let existing = node_repo.get_order_label_for_child(
            &tenant_id,
            &repo_id,
            &branch,
            workspace,
            &target_parent_id,
            node_id,
        )?;
        if let Some(label) = existing {
            (label, false)
        } else {
            // Append through the transaction's minter: this transaction's own
            // appends under the new parent first, the database after. Reading
            // the database alone minted the SAME label as a create into that
            // parent earlier (or later) in this transaction.
            let label = super::create::next_append_label_tx(
                tx,
                &tenant_id,
                &repo_id,
                &branch,
                workspace,
                &target_parent_id,
                &revision,
            )?;
            super::create::record_appended_label_tx(tx, workspace, &target_parent_id, &label)?;
            (label, true)
        }
    };

    // `(old path, node as moved)` per moved node, for the read cache (step 9)
    // and change tracking.
    let mut moved: Vec<(String, Node)> = Vec::new();

    // `(old, new)` records rewritten below, re-indexed once the batch lock is
    // released (the index writers take it themselves).
    let mut rewrites: Vec<(Node, Node)> = Vec::new();
    // `(old, moved)` for EVERY moved node: `__parent_path` is a compound-index
    // column, re-keyed below exactly as the repository move does.
    let mut rekeys: Vec<(Node, Node)> = Vec::new();
    // The listed version of each node re-keyed WITHOUT a record rewrite.
    let mut rekey_only: Vec<Node> = Vec::new();

    // 7. Lock batch and write all path updates. Scoped: the guard is not
    // `Send`, and the index maintenance below awaits.
    {
        let mut batch = tx
            .batch
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

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
            &target_parent_id,
            &new_order_label,
            &revision,
            node_id,
        );
        batch.put_cf(cf_ordered, new_ordered_key, new_name.as_bytes());

        // Update new parent's cached last-child metadata — only when the node was
        // appended. A rename keeps its label, which is usually NOT the last one;
        // caching it as LAST made the next append land in the middle.
        if appended {
            let new_metadata_key = keys::last_child_metadata_key(
                &tenant_id,
                &repo_id,
                &branch,
                workspace,
                &target_parent_id,
            );
            batch.put_cf(cf_ordered, new_metadata_key, new_order_label.as_bytes());
        }

        // 8. For each node (root + descendants): update paths
        for subject in &descendants {
            let (node, depth) = (&subject.node, &subject.depth);
            // Calculate new path for this node
            let node_new_path = if *depth == 0 {
                // Root node gets the new_path exactly
                new_path.to_string()
            } else {
                // Descendant nodes: replace old root prefix with new root prefix.
                // A node the child-order index claims is here but whose path says
                // otherwise is LEFT WHERE IT IS — see `moved_descendant_path` for
                // what the old `unwrap_or(&node.path)` did to it instead.
                match crate::repositories::nodes::helpers::moved_descendant_path(
                    &node.path,
                    &old_root_path,
                    new_path,
                ) {
                    Some(path) => path,
                    None => {
                        tracing::warn!(
                            node_id = %node.id,
                            node_path = %node.path,
                            old_root_path = %old_root_path,
                            "TXN move_node_tree: node is listed under the moved subtree but its path \
                             is outside it — leaving it in place"
                        );
                        continue;
                    }
                }
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

            let mut as_moved = node.clone();
            as_moved.path = node_new_path.clone();

            // A move is mostly index-only, because `Node` stores its parent's NAME
            // rather than a path. Three kinds of node still need their record
            // rewritten:
            //
            //   * the moved ROOT — new `name` (on rename), new `parent`, new
            //     `order_key`;
            //   * its DIRECT CHILDREN, but only on a RENAME, since they hold the
            //     root's old name in `parent`;
            //   * any node this transaction already WROTE (or moved): its record
            //     at this revision names the pre-move path, and a record must
            //     name one path. Rewritten from the staged node, so the write's
            //     own changes survive.
            //
            // Without the first two a transactional rename left `node.name`
            // reporting the old name forever, even though every path had been
            // updated around it.
            let updated_name = node_new_path
                .rsplit('/')
                .next()
                .unwrap_or(&node_new_path)
                .to_string();
            let updated_parent = Node::extract_parent_name_from_path(&node_new_path);
            let is_root = *depth == 0;
            let renamed = node.name != updated_name || node.parent != updated_parent;

            if is_root || renamed || subject.touched {
                as_moved.name = updated_name;
                as_moved.parent = updated_parent;
                if is_root || renamed {
                    as_moved.updated_at = Some(chrono::Utc::now());
                }
                if is_root {
                    as_moved.order_key = new_order_label.clone();
                }
                let parent_id = if is_root {
                    Some(target_parent_id.clone()).filter(|p| p != "/")
                } else {
                    subject.parent_id.clone()
                };

                // Through the one record writer (the NODE_PATH entry it
                // writes is the one written just above).
                crate::repositories::nodes::crud::indexing::node_record::write_node_record(
                    &tx.db, &mut batch, &tenant_id, &repo_id, &branch, workspace, &as_moved,
                    parent_id, &revision,
                )?;
                if is_root || renamed {
                    rewrites.push((node.clone(), as_moved.clone()));
                }
            } else {
                // Re-keyed from the LISTED version without a record rewrite:
                // checked at commit against the stored one.
                rekey_only.push(node.clone());
            }
            rekeys.push((node.clone(), as_moved.clone()));
            moved.push((node.path.clone(), as_moved));
        }
    }

    // 8b. A rewritten record carries a fresh `updated_at` (and on a rename a
    // new `name`): its property index must follow, exactly as `put_node`
    // maintains it — tombstone what the old record indexed, index the new
    // one. Without this, `ORDER BY updated_at` dropped every moved node (the
    // reader meets the stale entry first and rejects it on the re-check) and
    // `name = ...` kept answering with the old name.
    // 8c. Re-key compound-index entries. The transaction move never did, so
    // a node moved through it kept matching `CHILD_OF(old parent)` in a typed
    // folder listing and never matched the new one (the repository move has
    // done this since `move_tree_compound_reindex_test`). Derived from the old
    // record, so no workspace scan per moved node.
    for (old, moved) in &rekeys {
        super::create::indexing::write_compound_indexes(
            tx,
            &tenant_id,
            &repo_id,
            &branch,
            workspace,
            crate::indexing::Baseline::Full(Some(old)),
            moved,
            &revision,
        )
        .await?;
    }
    for (old, new) in &rewrites {
        // A re-stamp: full put against the record it replaces.
        super::create::indexing::index_node_properties(
            tx,
            &tenant_id,
            &repo_id,
            &branch,
            workspace,
            new,
            &revision,
            crate::repositories::nodes::PropertyWrite::full(Some(old)),
        )?;
    }

    // 8d. Each re-stamped record's property and compound writes were derived
    // from the committed subtree this move listed; the commit re-derives them
    // under the node's commit lock against what is stored then (plan Phase
    // 7b — `always`: a listing has no per-node "before the read" to record).
    {
        let ctx = crate::indexing::IndexCtx::new(&tenant_id, &repo_id, &branch, workspace);
        let mut cache = tx
            .read_cache
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        for (_, new) in &rewrites {
            cache
                .delta_checks
                .entry((workspace.to_string(), new.id.clone()))
                .or_insert_with(|| {
                    crate::indexing::StagedDeltaCheck::always(&ctx, &new.id, &revision)
                });
        }
        // A descendant re-keyed but not rewritten: its compound re-key was
        // derived from the version this move listed (an update may commit
        // before this transaction does).
        for listed in &rekey_only {
            cache
                .delta_checks
                .entry((workspace.to_string(), listed.id.clone()))
                .or_insert_with(|| {
                    crate::indexing::StagedDeltaCheck::rekey(&ctx, listed, &revision)
                });
        }
    }

    // 9. Update read cache for read-your-writes semantics: every moved node,
    // with its new path, where a later `get_node` in this transaction finds it.
    super::move_view::record_moves(tx, workspace, &moved)?;

    // 10. Track changes for event emission during commit
    {
        let mut changed = tx
            .changed_nodes
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

        for (old_path, node) in &moved {
            // A move changes the path, never a property: an untouched node
            // reports an empty changed-property list; one already written
            // earlier in this transaction keeps what that write knew.
            let changed_properties = match changed.get(&node.id) {
                Some(prev) => prev.changed_properties.clone(),
                None => Some(Vec::new()),
            };
            // Track as "Modified" operation (path changed)
            changed.insert(
                node.id.clone(),
                NodeChange {
                    workspace: workspace.to_string(),
                    revision,
                    operation: ChangeOperation::Modified,
                    path: Some(old_path.clone()), // Store path before move for event matching
                    node_type: Some(node.node_type.clone()),
                    changed_properties,
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
            Some(target_parent_id.clone()),
            Some(new_order_label.clone()),
        );
        // Every descendant's path changed too: replicate each, as the
        // repository move does, or a peer keeps them at their old paths.
        for (_, node) in &moved {
            if node.id != source_node.id {
                tracker.track_path_change(
                    node.id.clone(),
                    workspace.to_string(),
                    revision,
                    node.path.clone(),
                );
            }
        }
    }

    tracing::info!(
        "TXN move_node_tree: wrote {} path updates to transaction batch (single revision)",
        moved.len() * 3 // PATH_INDEX tombstone + new PATH_INDEX + NODE_PATH
    );

    Ok(())
}
