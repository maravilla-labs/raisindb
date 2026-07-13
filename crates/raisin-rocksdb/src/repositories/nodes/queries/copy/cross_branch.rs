//! Cross-branch node-set copy (branch promotion primitive).
//!
//! Copies a set of nodes — optionally with all descendants — from a source
//! branch onto a target branch in ONE atomic WriteBatch, **preserving node
//! ids** so repeated promotions update the same target nodes. Optionally
//! prunes target-branch nodes that no longer exist in the copied source set
//! (`delete_missing`), turning the operation into a one-way branch sync.
//!
//! Index maintenance mirrors the single-branch write paths:
//! - new/changed nodes go through `add_node_to_batch_with_parent_id`
//!   (node blob + PATH / NODE_PATH / PROPERTY / REFERENCE / RELATION /
//!   ORDERED_CHILDREN entries at the copy revision),
//! - pre-existing target nodes get old-value compound/unique tombstones
//!   first (the `update_impl` stale-index pattern), plus stale PATH and
//!   ORDERED_CHILDREN tombstones when the node moved on the source,
//! - pruned nodes get the full `delete_impl` tombstone set.

use super::super::super::helpers::{hash_property_value, TOMBSTONE};
use super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_models::translations::TranslationMeta;
use raisin_models::tree::ChangeOperation;
use raisin_storage::{
    BranchRepository, CrossBranchCopySummary, CrossBranchNodeChange, NodeChangeInfo,
    RevisionRepository,
};
use rocksdb::WriteBatch;
use std::collections::{HashMap, HashSet};

/// One source node staged for copying, with its resolved parent ids.
struct CopyEntry {
    node: Node,
    /// Parent id on the source branch (for reading the source order label)
    src_parent_id: String,
    /// Parent id on the target branch ("/" for root-level nodes; ids are
    /// preserved, so for non-root depths this equals the source parent id)
    dst_parent_id: String,
}

/// A resolved copy root: the source node plus its parent on both branches.
struct RootContext {
    node: Node,
    src_parent_id: String,
    dst_parent_id: String,
}

impl NodeRepositoryImpl {
    /// Copy a node set across branches — see
    /// [`raisin_storage::NodeRepository::copy_nodes_across_branches`] for the
    /// contract. All writes happen in a single WriteBatch under one revision;
    /// the target branch HEAD advances to that revision atomically.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::repositories::nodes) async fn copy_nodes_across_branches_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        source_branch: &str,
        target_branch: &str,
        workspace: &str,
        roots: &[String],
        recursive: bool,
        delete_missing: bool,
        operation_meta: Option<raisin_models::operations::OperationMeta>,
    ) -> Result<CrossBranchCopySummary> {
        // ========== VALIDATION ==========
        if roots.is_empty() {
            return Err(raisin_error::Error::Validation(
                "At least one root path is required".to_string(),
            ));
        }
        if source_branch == target_branch {
            return Err(raisin_error::Error::Validation(
                "Source and target branch must differ".to_string(),
            ));
        }

        self.branch_repo
            .get_branch(tenant_id, repo_id, source_branch)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!(
                    "Source branch '{}' not found",
                    source_branch
                ))
            })?;

        let target = self
            .branch_repo
            .get_branch(tenant_id, repo_id, target_branch)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!(
                    "Target branch '{}' not found",
                    target_branch
                ))
            })?;

        // Mirror merge_branches: never write onto a protected branch.
        if target.protected {
            return Err(raisin_error::Error::Forbidden(format!(
                "Cannot copy into protected branch '{}'",
                target_branch
            )));
        }

        // Resolve each root on the source branch and its parent on BOTH
        // branches. A missing target parent fails the whole operation early —
        // never write a dangling subtree.
        let mut root_ctxs: Vec<RootContext> = Vec::with_capacity(roots.len());
        for root_path in roots {
            let node = self
                .get_by_path_impl(
                    tenant_id,
                    repo_id,
                    source_branch,
                    workspace,
                    root_path,
                    None,
                )
                .await?
                .ok_or_else(|| {
                    raisin_error::Error::NotFound(format!(
                        "Source node '{}' not found on branch '{}'",
                        root_path, source_branch
                    ))
                })?;

            let parent_path = parent_path_of(&node.path);

            let src_parent_id = self
                .resolve_parent_id_opt(tenant_id, repo_id, source_branch, workspace, &parent_path)
                .await?
                .ok_or_else(|| {
                    raisin_error::Error::internal(format!(
                        "Source parent '{}' unresolvable on branch '{}'",
                        parent_path, source_branch
                    ))
                })?;

            let dst_parent_id = self
                .resolve_parent_id_opt(tenant_id, repo_id, target_branch, workspace, &parent_path)
                .await?
                .ok_or_else(|| {
                    raisin_error::Error::Validation(format!(
                        "Target parent '{}' does not exist on branch '{}'",
                        parent_path, target_branch
                    ))
                })?;

            root_ctxs.push(RootContext {
                node,
                src_parent_id,
                dst_parent_id,
            });
        }

        // ========== STEP 1: single revision for the whole operation ==========
        let revision = self.revision_repo.allocate_revision();

        // ========== STEP 2: collect the source node set (BFS, parents first) ==========
        let mut entries: Vec<CopyEntry> = Vec::new();
        let mut src_ids: HashSet<String> = HashSet::new();
        let mut path_to_id: HashMap<String, String> = HashMap::new();

        for rc in &root_ctxs {
            let set: Vec<(Node, usize)> = if recursive {
                self.scan_descendants_ordered_impl(
                    tenant_id,
                    repo_id,
                    source_branch,
                    workspace,
                    &rc.node.id,
                    None,
                )?
            } else {
                vec![(rc.node.clone(), 0)]
            };

            for (node, depth) in set {
                path_to_id.insert(node.path.clone(), node.id.clone());
                // Overlapping roots may visit a node twice — copy it once.
                if !src_ids.insert(node.id.clone()) {
                    continue;
                }

                let (src_parent_id, dst_parent_id) = if depth == 0 {
                    (rc.src_parent_id.clone(), rc.dst_parent_id.clone())
                } else {
                    // Parents-first BFS guarantees the parent path was
                    // already collected; ids are preserved, so the parent id
                    // is the same on both branches.
                    let parent_path = parent_path_of(&node.path);
                    let pid = path_to_id
                        .get(parent_path.as_str())
                        .cloned()
                        .ok_or_else(|| {
                            raisin_error::Error::internal(format!(
                                "Parent '{}' not collected before child '{}'",
                                parent_path, node.path
                            ))
                        })?;
                    (pid.clone(), pid)
                };

                entries.push(CopyEntry {
                    node,
                    src_parent_id,
                    dst_parent_id,
                });
            }
        }

        // ========== STEP 3: build the single WriteBatch ==========
        let mut batch = WriteBatch::default();
        let now = chrono::Utc::now();

        let mut changes: Vec<CrossBranchNodeChange> = Vec::new();
        let mut change_infos: Vec<NodeChangeInfo> = Vec::new();
        // (node as written, target parent id, order label) for replication capture
        let mut nodes_for_replication: Vec<(Node, String, String)> = Vec::new();
        // Highest order label written per target parent (metadata cache upkeep).
        let mut max_label_per_parent: HashMap<String, String> = HashMap::new();

        let (meta_actor, meta_message, meta_is_system) = if let Some(meta) = operation_meta.as_ref()
        {
            (meta.actor.clone(), meta.message.clone(), meta.is_system)
        } else {
            (
                "system".to_string(),
                format!(
                    "Copy {} node set(s) from branch '{}' to '{}'",
                    roots.len(),
                    source_branch,
                    target_branch
                ),
                true,
            )
        };

        for entry in &entries {
            let src_node = &entry.node;

            // Same id on the target branch -> Added vs Modified.
            let old_dst = self
                .get_impl(
                    tenant_id,
                    repo_id,
                    target_branch,
                    workspace,
                    &src_node.id,
                    false,
                )
                .await?;
            let operation = if old_dst.is_some() {
                ChangeOperation::Modified
            } else {
                ChangeOperation::Added
            };

            let mut node = src_node.clone();
            node.parent = Node::extract_parent_name_from_path(&node.path);
            node.has_children = None; // computed field, never stored
            node.children = vec![];
            if operation == ChangeOperation::Modified {
                // Keep creation metadata, stamp the modification.
                node.updated_at = Some(now);
            }

            // Child order: replay the source fractional label onto the target.
            // (Label collisions with independent target children are possible —
            // same caveat as branch merge; reads dedup and the ordering is
            // healed lazily on the next reorder.)
            let order_label = match self.get_order_label_for_child(
                tenant_id,
                repo_id,
                source_branch,
                workspace,
                &entry.src_parent_id,
                &node.id,
            )? {
                Some(label) => label,
                None => {
                    // No source ordering entry (unusual) — append at the end
                    // of the target parent instead.
                    match self.get_last_order_label(
                        tenant_id,
                        repo_id,
                        target_branch,
                        workspace,
                        &entry.dst_parent_id,
                    )? {
                        Some(last) => crate::fractional_index::inc(&last)
                            .unwrap_or_else(|_| crate::fractional_index::first()),
                        None => crate::fractional_index::first(),
                    }
                }
            };

            // Node blob + path/node_path/property/reference/relation/ordered
            // index entries, all at the shared revision.
            self.add_node_to_batch_with_parent_id(
                &mut batch,
                &node,
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                &revision,
                Some(&order_label),
                Some(&entry.dst_parent_id),
            )?;

            // Compound/unique indexes: tombstone the OLD target values first,
            // then write the new entries — without the tombstones a changed
            // column value would leave the stale old-value entry live (the
            // exact bug class fixed in update_impl).
            if let Some(old) = &old_dst {
                self.add_compound_tombstones_to_batch(
                    &mut batch,
                    old,
                    tenant_id,
                    repo_id,
                    target_branch,
                    workspace,
                )?;
                self.add_unique_tombstones_to_batch(
                    &mut batch,
                    old,
                    tenant_id,
                    repo_id,
                    target_branch,
                    workspace,
                    &revision,
                )
                .await?;
            }
            self.add_compound_indexes_to_batch(
                &mut batch,
                &node,
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                &revision,
            )
            .await?;
            self.add_unique_indexes_to_batch(
                &mut batch,
                &node,
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                &revision,
            )
            .await?;

            // If the node moved/renamed on the source since the last copy,
            // its old target path and old ordered-children slot are stale.
            if let Some(old) = &old_dst {
                self.tombstone_stale_placement(
                    &mut batch,
                    old,
                    &node,
                    &entry.dst_parent_id,
                    &order_label,
                    tenant_id,
                    repo_id,
                    target_branch,
                    workspace,
                    &revision,
                )
                .await?;
            }

            // Carry translations (node-level and block-level) to the target
            // branch under the SAME node id.
            self.copy_translations_to_batch(
                &mut batch,
                &node.id,
                tenant_id,
                repo_id,
                source_branch,
                target_branch,
                workspace,
                &revision,
                now,
                &meta_actor,
                &meta_message,
                meta_is_system,
                operation,
                &mut change_infos,
            )?;

            // Track the max label per parent for the last-child metadata cache.
            let slot = max_label_per_parent
                .entry(entry.dst_parent_id.clone())
                .or_insert_with(|| order_label.clone());
            if order_label > *slot {
                *slot = order_label.clone();
            }

            changes.push(CrossBranchNodeChange {
                node_id: node.id.clone(),
                path: node.path.clone(),
                node_type: node.node_type.clone(),
                operation,
            });
            change_infos.push(NodeChangeInfo {
                node_id: node.id.clone(),
                workspace: workspace.to_string(),
                operation,
                translation_locale: None,
            });
            nodes_for_replication.push((node, entry.dst_parent_id.clone(), order_label));
        }

        let copied = entries.len();

        // Last-child metadata cache: only advance it — writing a smaller
        // label would make the next append mint an already-used label.
        {
            let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;
            for (parent_id, label) in &max_label_per_parent {
                let current = self.get_last_order_label(
                    tenant_id,
                    repo_id,
                    target_branch,
                    workspace,
                    parent_id,
                )?;
                if current
                    .as_deref()
                    .map(|c| label.as_str() > c)
                    .unwrap_or(true)
                {
                    let metadata_key = keys::last_child_metadata_key(
                        tenant_id,
                        repo_id,
                        target_branch,
                        workspace,
                        parent_id,
                    );
                    batch.put_cf(cf_ordered, metadata_key, label.as_bytes());
                }
            }
        }

        // ========== STEP 4: delete_missing — prune target-only nodes ==========
        let mut deleted_ids: HashSet<String> = HashSet::new();
        if delete_missing {
            for rc in &root_ctxs {
                // The pre-copy target tree under the same root id (the batch
                // is not committed yet, so this sees the previous state).
                if self
                    .get_impl(
                        tenant_id,
                        repo_id,
                        target_branch,
                        workspace,
                        &rc.node.id,
                        false,
                    )
                    .await?
                    .is_none()
                {
                    continue;
                }
                let dst_set = self.scan_descendants_ordered_impl(
                    tenant_id,
                    repo_id,
                    target_branch,
                    workspace,
                    &rc.node.id,
                    None,
                )?;
                for (dst_node, _) in dst_set {
                    if src_ids.contains(&dst_node.id) || !deleted_ids.insert(dst_node.id.clone()) {
                        continue;
                    }
                    self.add_cross_branch_prune_to_batch(
                        &mut batch,
                        &dst_node,
                        tenant_id,
                        repo_id,
                        target_branch,
                        workspace,
                        &revision,
                    )
                    .await?;
                    changes.push(CrossBranchNodeChange {
                        node_id: dst_node.id.clone(),
                        path: dst_node.path.clone(),
                        node_type: dst_node.node_type.clone(),
                        operation: ChangeOperation::Deleted,
                    });
                    change_infos.push(NodeChangeInfo {
                        node_id: dst_node.id.clone(),
                        workspace: workspace.to_string(),
                        operation: ChangeOperation::Deleted,
                        translation_locale: None,
                    });
                }
            }
        }
        let deleted = deleted_ids.len();

        // ========== STEP 5: revision index + branch HEAD in the same batch ==========
        for change in &changes {
            self.revision_repo.index_node_change_to_batch(
                &mut batch,
                tenant_id,
                repo_id,
                &revision,
                &change.node_id,
            )?;
        }

        let updated_branch = self
            .branch_repo
            .update_head_to_batch(&mut batch, tenant_id, repo_id, target_branch, revision)
            .await?;

        // ========== STEP 6: atomic commit ==========
        self.db.write(batch).map_err(|e| {
            raisin_error::Error::storage(format!("Atomic cross-branch copy failed: {}", e))
        })?;

        tracing::info!(
            "copy_nodes_across_branches: {} copied, {} pruned, {} -> {} at revision {}",
            copied,
            deleted,
            source_branch,
            target_branch,
            revision
        );

        // ========== STEP 7: replication + revision metadata (post-commit) ==========
        self.branch_repo
            .capture_head_update_for_replication(
                tenant_id,
                repo_id,
                target_branch,
                &updated_branch,
                revision,
            )
            .await;

        self.capture_cross_branch_operations(
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            &meta_actor,
            &revision,
            &nodes_for_replication,
            &deleted_ids,
        )
        .await;

        // Always store revision metadata — this is what makes the promotion
        // visible to branch diffs and change tracking.
        let operation = operation_meta.map(|mut op_meta| {
            op_meta.revision = revision;
            if op_meta.node_id.is_empty() {
                if let Some(rc) = root_ctxs.first() {
                    op_meta.node_id = rc.node.id.clone();
                }
            }
            op_meta
        });

        let rev_meta = raisin_storage::RevisionMeta {
            revision,
            parent: Some(target.head),
            merge_parent: None,
            branch: target_branch.to_string(),
            timestamp: now,
            actor: meta_actor,
            message: meta_message,
            is_system: meta_is_system,
            changed_nodes: change_infos,
            changed_node_types: Vec::new(),
            changed_archetypes: Vec::new(),
            changed_element_types: Vec::new(),
            operation,
        };
        self.revision_repo
            .store_revision_meta(tenant_id, repo_id, rev_meta)
            .await?;

        Ok(CrossBranchCopySummary {
            copied,
            deleted,
            revision,
            changes,
        })
    }

    /// Resolve a parent path to the parent id used by the ORDERED_CHILDREN
    /// index: `"/"` for root-level nodes, otherwise the parent node's id.
    async fn resolve_parent_id_opt(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_path: &str,
    ) -> Result<Option<String>> {
        if parent_path == "/" {
            return Ok(Some("/".to_string()));
        }
        Ok(self
            .get_by_path_impl(tenant_id, repo_id, branch, workspace, parent_path, None)
            .await?
            .map(|p| p.id))
    }

    /// Tombstone the stale PATH_INDEX and ORDERED_CHILDREN entries left on the
    /// target branch when a node moved/renamed/reordered on the source since
    /// the last promotion.
    #[allow(clippy::too_many_arguments)]
    async fn tombstone_stale_placement(
        &self,
        batch: &mut WriteBatch,
        old_dst: &Node,
        new_node: &Node,
        new_parent_id: &str,
        new_label: &str,
        tenant_id: &str,
        repo_id: &str,
        target_branch: &str,
        workspace: &str,
        revision: &HLC,
    ) -> Result<()> {
        if old_dst.path != new_node.path {
            let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;
            let old_path_key = keys::path_index_key_versioned(
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                &old_dst.path,
                revision,
            );
            batch.put_cf(cf_path, old_path_key, TOMBSTONE);
        }

        let old_parent_path = parent_path_of(&old_dst.path);
        let old_parent_id = self
            .resolve_parent_id_opt(
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                &old_parent_path,
            )
            .await?;
        if let Some(old_pid) = old_parent_id {
            if let Some(old_label) = self.get_order_label_for_child(
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                &old_pid,
                &new_node.id,
            )? {
                if old_pid != new_parent_id || old_label != new_label {
                    let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;
                    let ordered_key = keys::ordered_child_key_versioned(
                        tenant_id,
                        repo_id,
                        target_branch,
                        workspace,
                        &old_pid,
                        &old_label,
                        revision,
                        &new_node.id,
                    );
                    batch.put_cf(cf_ordered, ordered_key, TOMBSTONE);
                }
            }
        }

        Ok(())
    }

    /// Copy the latest node-level and block-level translations of `node_id`
    /// from the source branch onto the target branch (same node id), writing
    /// TRANSLATION_DATA / TRANSLATION_INDEX / meta / snapshot entries at the
    /// shared revision — mirrors the single-branch tree-copy behavior.
    #[allow(clippy::too_many_arguments)]
    fn copy_translations_to_batch(
        &self,
        batch: &mut WriteBatch,
        node_id: &str,
        tenant_id: &str,
        repo_id: &str,
        source_branch: &str,
        target_branch: &str,
        workspace: &str,
        revision: &HLC,
        now: chrono::DateTime<chrono::Utc>,
        actor: &str,
        message: &str,
        is_system: bool,
        operation: ChangeOperation,
        change_infos: &mut Vec<NodeChangeInfo>,
    ) -> Result<()> {
        let cf_translation_data = cf_handle(&self.db, cf::TRANSLATION_DATA)?;
        let cf_translation_index = cf_handle(&self.db, cf::TRANSLATION_INDEX)?;
        let cf_block_translations = cf_handle(&self.db, cf::BLOCK_TRANSLATIONS)?;
        let cf_revisions = cf_handle(&self.db, cf::REVISIONS)?;

        let node_translations = self.collect_node_translations_for_copy(
            tenant_id,
            repo_id,
            source_branch,
            workspace,
            node_id,
        )?;

        for (locale, overlay, parent_translation_revision) in node_translations {
            let overlay_bytes = serde_json::to_vec(&overlay).map_err(|e| {
                raisin_error::Error::storage(format!(
                    "Failed to serialize translation overlay for locale {}: {}",
                    locale.as_str(),
                    e
                ))
            })?;
            let data_key = Self::translation_data_key(
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                node_id,
                locale.as_str(),
                revision,
            );
            batch.put_cf(&cf_translation_data, data_key, overlay_bytes.clone());

            let index_key =
                Self::translation_index_key(tenant_id, repo_id, locale.as_str(), revision, node_id);
            batch.put_cf(&cf_translation_index, index_key, b"");

            let translation_meta = TranslationMeta {
                locale: locale.clone(),
                revision: *revision,
                parent_revision: parent_translation_revision,
                timestamp: now,
                actor: actor.to_string(),
                message: message.to_string(),
                is_system,
            };
            let meta_bytes = serde_json::to_vec(&translation_meta).map_err(|e| {
                raisin_error::Error::storage(format!(
                    "Failed to serialize TranslationMeta for locale {}: {}",
                    locale.as_str(),
                    e
                ))
            })?;
            let meta_key = Self::translation_meta_key(
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                node_id,
                locale.as_str(),
                revision,
            );
            batch.put_cf(&cf_revisions, meta_key, meta_bytes);

            let snapshot_key = keys::translation_snapshot_key(
                tenant_id,
                repo_id,
                node_id,
                locale.as_str(),
                revision,
            );
            batch.put_cf(&cf_revisions, snapshot_key, overlay_bytes.clone());

            change_infos.push(NodeChangeInfo {
                node_id: node_id.to_string(),
                workspace: workspace.to_string(),
                operation,
                translation_locale: Some(locale.as_str().to_string()),
            });
        }

        let block_translations = self.collect_block_translations_for_copy(
            tenant_id,
            repo_id,
            source_branch,
            workspace,
            node_id,
        )?;

        for (block_uuid, locale, overlay, _parent_revision) in block_translations {
            let overlay_bytes = serde_json::to_vec(&overlay).map_err(|e| {
                raisin_error::Error::storage(format!(
                    "Failed to serialize block translation overlay {}::{}: {}",
                    locale.as_str(),
                    block_uuid,
                    e
                ))
            })?;

            let block_key = Self::block_translation_key(
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                node_id,
                &block_uuid,
                locale.as_str(),
                revision,
            );
            batch.put_cf(&cf_block_translations, block_key, overlay_bytes.clone());

            let snapshot_key = keys::translation_snapshot_key(
                tenant_id,
                repo_id,
                node_id,
                &format!("{}::{}", locale.as_str(), block_uuid),
                revision,
            );
            batch.put_cf(&cf_revisions, snapshot_key, overlay_bytes.clone());

            change_infos.push(NodeChangeInfo {
                node_id: node_id.to_string(),
                workspace: workspace.to_string(),
                operation,
                translation_locale: Some(format!("{}::{}", locale.as_str(), block_uuid)),
            });
        }

        Ok(())
    }

    /// Add the full tombstone set for one pruned target-branch node —
    /// mirrors `delete_impl` (node blob, path, property/system-property,
    /// unique, reference, relation, translation, ordered-children entries),
    /// but writes into the shared cross-branch batch instead of committing.
    #[allow(clippy::too_many_arguments)]
    async fn add_cross_branch_prune_to_batch(
        &self,
        batch: &mut WriteBatch,
        node: &Node,
        tenant_id: &str,
        repo_id: &str,
        target_branch: &str,
        workspace: &str,
        revision: &HLC,
    ) -> Result<()> {
        let cf_nodes = cf_handle(&self.db, cf::NODES)?;
        let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;
        let cf_property = cf_handle(&self.db, cf::PROPERTY_INDEX)?;
        let cf_reference = cf_handle(&self.db, cf::REFERENCE_INDEX)?;
        let cf_relation = cf_handle(&self.db, cf::RELATION_INDEX)?;
        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;

        // Node blob tombstone
        let node_key = keys::node_key_versioned(
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            &node.id,
            revision,
        );
        batch.put_cf(cf_nodes, node_key, TOMBSTONE);

        // Path index tombstone
        let path_key = keys::path_index_key_versioned(
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            &node.path,
            revision,
        );
        batch.put_cf(cf_path, path_key, TOMBSTONE);

        // Property index tombstones (user properties + __node_type)
        let is_published = node.published_at.is_some();
        for (prop_name, prop_value) in &node.properties {
            let value_hash = hash_property_value(prop_value);
            let prop_key = keys::property_index_key_versioned(
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                prop_name,
                &value_hash,
                revision,
                &node.id,
                is_published,
            );
            batch.put_cf(cf_property, prop_key, TOMBSTONE);
        }
        let node_type_key = keys::property_index_key_versioned(
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            "__node_type",
            &node.node_type,
            revision,
            &node.id,
            is_published,
        );
        batch.put_cf(cf_property, node_type_key, TOMBSTONE);

        // System field indexes (__name, __archetype, __created_*, __updated_*)
        self.add_field_tombstones_to_batch(
            batch,
            cf_property,
            node,
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            &node.id,
            revision,
            is_published,
        );

        // Unique index tombstones (release unique values)
        self.add_unique_tombstones_to_batch(
            batch,
            node,
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            revision,
        )
        .await?;

        // Reference index tombstones (forward + reverse)
        self.add_reference_tombstones_to_batch(
            batch,
            cf_reference,
            node,
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            &node.id,
            revision,
            is_published,
        );

        // Relation index tombstones (outgoing + incoming)
        self.add_relation_tombstones_to_batch(
            batch,
            cf_relation,
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            &node.id,
            revision,
        )?;

        // Translation tombstones
        self.add_translation_tombstones_to_batch(
            batch,
            tenant_id,
            repo_id,
            target_branch,
            workspace,
            &node.id,
            revision,
        )?;

        // Ordered-children tombstone (resolve the parent id by path — the
        // node's `parent` field holds the parent NAME, not its id)
        let parent_path = parent_path_of(&node.path);
        if let Some(parent_id) = self
            .resolve_parent_id_opt(tenant_id, repo_id, target_branch, workspace, &parent_path)
            .await?
        {
            if let Some(label) = self.get_order_label_for_child(
                tenant_id,
                repo_id,
                target_branch,
                workspace,
                &parent_id,
                &node.id,
            )? {
                let ordered_key = keys::ordered_child_key_versioned(
                    tenant_id,
                    repo_id,
                    target_branch,
                    workspace,
                    &parent_id,
                    &label,
                    revision,
                    &node.id,
                );
                batch.put_cf(cf_ordered, ordered_key, TOMBSTONE);
            }
        }

        Ok(())
    }

    /// Capture replication operations for a cross-branch copy (post-commit):
    /// CreateNode per copied node, DeleteNode per pruned node — mirrors the
    /// single-branch tree-copy and delete capture behavior.
    #[allow(clippy::too_many_arguments)]
    async fn capture_cross_branch_operations(
        &self,
        tenant_id: &str,
        repo_id: &str,
        target_branch: &str,
        workspace: &str,
        actor: &str,
        revision: &HLC,
        nodes_for_replication: &[(Node, String, String)],
        deleted_ids: &HashSet<String>,
    ) {
        if !self.operation_capture.is_enabled() {
            return;
        }

        for (node, parent_id, order_label) in nodes_for_replication {
            let properties_json =
                serde_json::to_value(&node.properties).unwrap_or(serde_json::json!({}));

            let _ = self
                .operation_capture
                .capture_create_node(
                    tenant_id.to_string(),
                    repo_id.to_string(),
                    target_branch.to_string(),
                    node.id.clone(),
                    node.name.clone(),
                    node.node_type.clone(),
                    node.archetype.clone(),
                    Some(parent_id.clone()),
                    order_label.clone(),
                    properties_json,
                    node.owner_id.clone(),
                    Some(workspace.to_string()),
                    node.path.clone(),
                    actor.to_string(),
                )
                .await;
        }

        for node_id in deleted_ids {
            let op_type = raisin_replication::OpType::DeleteNode {
                node_id: node_id.clone(),
            };
            let _ = self
                .operation_capture
                .capture_operation_with_revision(
                    tenant_id.to_string(),
                    repo_id.to_string(),
                    target_branch.to_string(),
                    op_type,
                    actor.to_string(),
                    None,
                    true,
                    Some(*revision),
                )
                .await;
        }
    }
}

/// Parent path of a node path (`"/a/b" -> "/a"`, `"/a" -> "/"`).
fn parent_path_of(path: &str) -> String {
    match path.rsplit_once('/') {
        Some(("", _)) | None => "/".to_string(),
        Some((parent, _)) => parent.to_string(),
    }
}
