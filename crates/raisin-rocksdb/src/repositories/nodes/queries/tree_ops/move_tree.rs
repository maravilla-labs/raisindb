//! Tree move operation - moves a node and all its descendants
//!
//! Since nodes are stored as StorageNode (without path), moving a tree only
//! updates indexes - NO node blob rewrites needed. This is O(K) where K is
//! the number of index entries, vs O(N*blob_size) with embedded paths.

use super::super::super::helpers::{moved_descendant_path, TOMBSTONE};
use super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_storage::{
    BranchRepository, BranchScope, NodeRepository, RevisionRepository, StorageScope,
};
use rocksdb::WriteBatch;

impl NodeRepositoryImpl {
    /// Move node tree (node + all descendants) to a new location
    ///
    /// For each node in the tree:
    /// - Tombstone old PATH_INDEX
    /// - Write new PATH_INDEX
    /// - Write new NODE_PATH (node_id -> new path)
    ///
    /// Only for root node:
    /// - Update ORDERED_CHILDREN (parent changes)
    ///
    /// # Algorithm
    /// 1. Validate move operation
    /// 2. Scan all descendants using prefix scan
    /// 3. In ONE atomic WriteBatch: update all indexes
    /// 4. Update branch HEAD
    ///
    /// # Performance
    /// - ONE WriteBatch for entire tree (atomic)
    /// - ONE revision for all nodes
    /// - No blob writes - only index updates
    /// - Node IDs are preserved (unlike copy+delete)
    pub(in crate::repositories::nodes) async fn move_node_tree_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        id: &str,
        new_path: &str,
        operation_meta: Option<raisin_models::operations::OperationMeta>,
    ) -> Result<()> {
        tracing::info!(
            "move_node_tree: moving tree from id={} to new_path={} (optimized - index only)",
            id,
            new_path
        );

        // Get existing root node
        let root_node = self
            .get_impl(tenant_id, repo_id, branch, workspace, id, false)
            .await?
            .ok_or_else(|| raisin_error::Error::NotFound("Node not found".to_string()))?;

        let old_root_path = root_node.path.clone();

        // Validation 1: Cannot move root node
        self.validate_not_root_node(&old_root_path)?;

        // Extract target parent and new name from new_path
        let (target_parent_path, new_name) = new_path
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

        // Validation 2: Target parent must exist — EXCEPT the workspace root,
        // which is a legitimate destination that stores no node of its own.
        // `None` here means "the root"; see validate_parent_exists_opt.
        let target_parent_node = self
            .validate_parent_exists_opt(tenant_id, repo_id, branch, workspace, &target_parent_path)
            .await?;

        // Validation 3: Check workspace allows this node type
        let is_root_target = target_parent_path == "/";
        self.validate_workspace_allows_node_type(
            StorageScope::new(tenant_id, repo_id, branch, workspace),
            &root_node.node_type,
            is_root_target,
        )
        .await?;

        // Validation 4: Check if root node type is allowed under target parent's
        // schema. Skipped at the root: there is no parent node whose schema
        // could allow or refuse a child, and Validation 3 has already asked the
        // WORKSPACE — which is the authority on what may sit at its top level.
        if let Some(parent) = &target_parent_node {
            self.validate_parent_allows_child(
                BranchScope::new(tenant_id, repo_id, branch),
                &parent.node_type,
                &root_node.node_type,
            )
            .await?;
        }

        // Validation 5: No circular reference
        self.validate_no_circular_reference(&old_root_path, &target_parent_path)
            .await?;

        // Validation 6: Check for duplicate names in target location. Root-level
        // nodes are indexed with `parent_id = "/"` (see listing.rs), so the
        // uniqueness check reaches the workspace's top level with that literal.
        self.validate_unique_child_name(
            tenant_id,
            repo_id,
            branch,
            workspace,
            target_parent_node.as_ref().map_or("/", |p| p.id.as_str()),
            &new_name,
        )
        .await?;

        tracing::info!(
            "move_node_tree: source_path={}, target_path={}",
            old_root_path,
            new_path
        );

        // Collect all descendants (includes root at depth 0)
        let descendants =
            self.scan_descendants_ordered_impl(tenant_id, repo_id, branch, workspace, id, None)?;

        tracing::info!(
            "move_node_tree: found {} nodes to move (index-only updates)",
            descendants.len()
        );

        // Allocate single revision for entire tree move
        let revision = self.revision_repo.allocate_revision();

        // Prepare atomic WriteBatch
        let mut batch = WriteBatch::default();

        // Get column family handles
        let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;
        let cf_node_path = cf_handle(&self.db, cf::NODE_PATH)?;
        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;

        // Process root node's ORDERED_CHILDREN (parent changes)
        if let Some(old_parent_path) =
            old_root_path
                .rsplit_once('/')
                .map(|(p, _)| if p.is_empty() { "/" } else { p })
        {
            if let Some(old_parent_node) = self
                .get_by_path_impl(tenant_id, repo_id, branch, workspace, old_parent_path, None)
                .await?
            {
                // Tombstone old ordered children entry
                if let Some(old_label) = self.get_order_label_for_child(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &old_parent_node.id,
                    id,
                )? {
                    let old_ordered_key = keys::ordered_child_key_versioned(
                        tenant_id,
                        repo_id,
                        branch,
                        workspace,
                        &old_parent_node.id,
                        &old_label,
                        &revision,
                        id,
                    );
                    batch.put_cf(cf_ordered, old_ordered_key, TOMBSTONE);

                    // Invalidate cached metadata
                    let metadata_key = keys::last_child_metadata_key(
                        tenant_id,
                        repo_id,
                        branch,
                        workspace,
                        &old_parent_node.id,
                    );
                    batch.delete_cf(cf_ordered, metadata_key);
                }
            }
        }

        // Add root node to new parent's ORDERED_CHILDREN. At the workspace root
        // that index is keyed by the literal "/" — the same convention
        // add_impl/update_impl use for root-level nodes (see listing.rs).
        let new_parent_id = target_parent_node
            .as_ref()
            .map_or_else(|| "/".to_string(), |p| p.id.clone());

        let order_label = match self.get_order_label_for_child(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &new_parent_id,
            id,
        )? {
            // Already ordered under the target parent (re-move / rename in place).
            Some(existing) => existing,
            None => self.next_append_label(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &new_parent_id,
                &revision,
            )?,
        };
        let ordered_key = keys::ordered_child_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &new_parent_id,
            &order_label,
            &revision,
            id,
        );
        batch.put_cf(cf_ordered, ordered_key, new_name.as_bytes());

        // Update cached last-child metadata
        let metadata_key =
            keys::last_child_metadata_key(tenant_id, repo_id, branch, workspace, &new_parent_id);
        batch.put_cf(cf_ordered, metadata_key, order_label.as_bytes());

        // Process all nodes (root + descendants): update PATH_INDEX and NODE_PATH
        let mut moved_node_ids = Vec::new();
        // (node_id, old_path) for each moved node, for post-commit move events.
        let mut moved_pairs: Vec<(String, String)> = Vec::new();
        // Nodes the child-order index claimed are in this subtree but whose paths
        // say otherwise. They are LEFT WHERE THEY ARE (see
        // `moved_descendant_path`) and their stale ordering entries are healed
        // below, so the inconsistency does not follow the tree around forever.
        let mut orphaned: Vec<(String, String, usize)> = Vec::new();

        for (node, depth) in &descendants {
            // Calculate new path for this node
            let node_new_path = if *depth == 0 {
                new_path.to_string()
            } else {
                match moved_descendant_path(&node.path, &old_root_path, new_path) {
                    Some(path) => path,
                    None => {
                        tracing::warn!(
                            node_id = %node.id,
                            node_path = %node.path,
                            root_id = %id,
                            old_root_path = %old_root_path,
                            "move_node_tree: the child-order index lists this node under the \
                             moved subtree but its path is outside it — leaving it in place and \
                             dropping the stale ordering entry"
                        );
                        orphaned.push((node.id.clone(), node.path.clone(), *depth));
                        continue;
                    }
                }
            };

            moved_node_ids.push(node.id.clone());
            moved_pairs.push((node.id.clone(), node.path.clone()));

            // Tombstone old PATH_INDEX
            let old_path_key = keys::path_index_key_versioned(
                tenant_id, repo_id, branch, workspace, &node.path, &revision,
            );
            batch.put_cf(cf_path, old_path_key, TOMBSTONE);

            // Write new PATH_INDEX
            let new_path_key = keys::path_index_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &node_new_path,
                &revision,
            );
            batch.put_cf(cf_path, new_path_key, node.id.as_bytes());

            // Write new NODE_PATH (node_id -> new path)
            let node_path_key = keys::node_path_key_versioned(
                tenant_id, repo_id, branch, workspace, &node.id, &revision,
            );
            batch.put_cf(cf_node_path, node_path_key, node_new_path.as_bytes());

            // Re-key compound indexes.
            //
            // A move changes `__parent_path`, which is a compound-index COLUMN,
            // so an entry keyed on the old parent would keep matching
            // `CHILD_OF(old)` forever and never match `CHILD_OF(new)` — a
            // silently wrong result set with no error. Tombstone against the old
            // path, then write against the new one.
            //
            // Compound indexes are the one family NOT covered by
            // `add_node_indexes_to_batch` (they need an async NodeType load,
            // which that sync, replication-safe path cannot do), so every write
            // path has to remember them individually. This is the fifth such
            // site; the same omission has already shipped twice — once for
            // spatial (see indexing/mod.rs) and once for the SQL DML path (see
            // transaction/.../indexing.rs). `move_tree_compound_reindex_test`
            // pins it.
            self.add_compound_tombstones_to_batch(
                &mut batch, node, tenant_id, repo_id, branch, workspace,
            )?;

            let mut moved_node = node.clone();
            moved_node.path = node_new_path.clone();
            self.add_compound_indexes_to_batch(
                &mut batch,
                &moved_node,
                tenant_id,
                repo_id,
                branch,
                workspace,
                &revision,
            )
            .await?;
        }

        // HEAL the inconsistency that produced `orphaned`.
        //
        // A node reached at depth 1 got there through THIS root's
        // ORDERED_CHILDREN, so that entry is the stale one and this is the only
        // place with enough context to name it. Tombstoning it stops the node
        // being dragged along by every future move of the root — which is how a
        // stale link turned into an unreachable node.
        //
        // Deeper orphans are left alone: their entry belongs to some intermediate
        // parent this loop does not identify, and guessing would tombstone a
        // legitimate one. They are logged, they are not corrupted, and the same
        // repair happens when their real parent is moved.
        for (orphan_id, orphan_path, depth) in &orphaned {
            if *depth != 1 {
                continue;
            }
            if let Some(stale_label) =
                self.get_order_label_for_child(tenant_id, repo_id, branch, workspace, id, orphan_id)?
            {
                let stale_key = keys::ordered_child_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    id,
                    &stale_label,
                    &revision,
                    orphan_id,
                );
                batch.put_cf(cf_ordered, stale_key, TOMBSTONE);
                tracing::warn!(
                    node_id = %orphan_id,
                    node_path = %orphan_path,
                    root_id = %id,
                    "move_node_tree: dropped a stale child-order entry pointing outside the subtree"
                );
            }
        }

        // Most of a move is index-only — a descendant's stored blob stays valid
        // because `Node` holds its parent's NAME, not a path. But two kinds of node
        // do go stale and must be rewritten:
        //
        //   * the moved ROOT — new `name` (on rename), new `parent`, new `order_key`;
        //   * its DIRECT CHILDREN, but only when the root was RENAMED, since they
        //     store the root's old name in `parent`.
        //
        // Deeper descendants are untouched: their parent's name did not change.
        // Rewriting them would turn an O(index) move into an O(subtree) blob
        // rewrite for no benefit.
        let mut rewritten_nodes: Vec<Node> = Vec::new();
        for (node, depth) in &descendants {
            let node_new_path = if *depth == 0 {
                new_path.to_string()
            } else {
                // Same guard as the index loop: a node that is not really in this
                // subtree keeps its blob untouched along with its indexes.
                match moved_descendant_path(&node.path, &old_root_path, new_path) {
                    Some(path) => path,
                    None => continue,
                }
            };

            let updated_name = node_new_path
                .rsplit('/')
                .next()
                .unwrap_or(&node_new_path)
                .to_string();
            let updated_parent = Node::extract_parent_name_from_path(&node_new_path);
            let is_root = *depth == 0;

            // Skip any node whose identity fields are unchanged.
            if !is_root && node.name == updated_name && node.parent == updated_parent {
                continue;
            }

            let mut rewritten = node.clone();
            rewritten.path = node_new_path;
            rewritten.name = updated_name;
            rewritten.parent = updated_parent;
            rewritten.updated_at = Some(chrono::Utc::now());
            if is_root {
                rewritten.order_key = order_label.clone();
            }

            // The root lands under a new parent; descendants keep theirs.
            let parent_id_for_index = if is_root {
                Some(new_parent_id.clone())
            } else {
                None
            };
            self.add_node_indexes_to_batch_with_parent_id(
                &mut batch,
                &rewritten,
                tenant_id,
                repo_id,
                branch,
                workspace,
                &revision,
                parent_id_for_index,
            )?;
            rewritten_nodes.push(rewritten);
        }

        let root_node_for_replication = rewritten_nodes
            .iter()
            .find(|node| node.path == new_path)
            .cloned();

        // Atomic commit
        self.db
            .write(batch)
            .map_err(|e| raisin_error::Error::storage(format!("Atomic tree move failed: {}", e)))?;

        tracing::info!(
            "move_node_tree: wrote {} index updates + {} blob rewrites atomically",
            moved_node_ids.len() * 3,
            rewritten_nodes.len()
        );
        debug_assert!(
            root_node_for_replication.is_some(),
            "the traversal must always include the moved root at depth 0"
        );

        // Update branch HEAD
        self.branch_repo
            .update_head(tenant_id, repo_id, branch, revision)
            .await?;

        // The acting principal for this move. Resolved once, above the
        // replication capture, so the operation peers replay carries the SAME
        // actor the local move events below stamp -- a masterless cluster where
        // origin and replica disagree on who moved a node is simply wrong.
        let event_actor = operation_meta
            .as_ref()
            .map(|m| m.actor.clone())
            .unwrap_or_else(|| "system".to_string());

        // Capture move for replication as an ApplyRevision snapshot covering
        // every moved node (root + descendants) at its NEW path. A granular
        // MoveNode op only names the root, so peers never learned about
        // descendant path changes.
        if self.operation_capture.is_enabled() {
            let mut changes = Vec::with_capacity(moved_node_ids.len());
            for node_id in &moved_node_ids {
                match self
                    .get(
                        raisin_storage::StorageScope::new(tenant_id, repo_id, branch, workspace),
                        node_id,
                        None,
                    )
                    .await
                {
                    Ok(Some(moved)) => changes.push((
                        moved,
                        raisin_replication::operation::ReplicatedNodeChangeKind::Upsert,
                    )),
                    Ok(None) => tracing::warn!(
                        node_id = %node_id,
                        "Moved node missing when capturing replication snapshot"
                    ),
                    Err(e) => tracing::warn!(
                        node_id = %node_id,
                        error = %e,
                        "Failed to load moved node for replication snapshot"
                    ),
                }
            }
            self.capture_apply_revision_snapshot(
                tenant_id,
                repo_id,
                branch,
                workspace,
                changes,
                revision,
                crate::repositories::nodes::WriteAttribution {
                    actor: Some(&event_actor),
                    agent: operation_meta.as_ref().and_then(|m| m.agent.as_deref()),
                },
            )
            .await;
        }

        // Index node changes for revision tracking
        for node_id in &moved_node_ids {
            self.revision_repo
                .index_node_change(tenant_id, repo_id, &revision, node_id)
                .await?;
        }

        // Emit move events (root + descendants) so the subscribed job handler
        // reindexes fulltext and retargets references to the NEW paths. Done
        // after HEAD is updated so each node reads back at its new path.
        self.emit_move_node_events(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &revision,
            &moved_pairs,
            &event_actor,
        )
        .await;

        // Store operation metadata if provided
        if let Some(op_meta) = operation_meta {
            let rev_meta = raisin_storage::RevisionMeta {
                revision,
                parent: op_meta.parent_revision,
                merge_parent: None,
                branch: branch.to_string(),
                timestamp: op_meta.timestamp,
                actor: op_meta.actor.clone(),
                message: op_meta.message.clone(),
                is_system: op_meta.is_system,
                changed_nodes: vec![],
                changed_node_types: Vec::new(),
                changed_archetypes: Vec::new(),
                changed_element_types: Vec::new(),
                operation: Some(op_meta),
            };

            self.revision_repo
                .store_revision_meta(tenant_id, repo_id, rev_meta)
                .await?;
        }

        tracing::info!(
            "move_node_tree: complete - {} nodes moved (IDs preserved)",
            moved_node_ids.len()
        );
        Ok(())
    }
}
