//! Tree copy operation.
//!
//! Recursively copies a node and all its descendants to a new location.
//! Generates new IDs for all copied nodes while preserving the tree structure,
//! fractional index ordering, and translations (node-level and block-level).

use super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_models::translations::TranslationMeta;
use raisin_models::tree::ChangeOperation;
use raisin_storage::{BranchRepository, BranchScope, NodeRepository, RevisionRepository};
use rocksdb::WriteBatch;
use std::collections::HashMap;

impl NodeRepositoryImpl {
    /// Copy node tree recursively
    ///
    /// Recursively copies a node and all its descendants to a new location.
    /// Generates new IDs for all copied nodes while preserving the tree structure
    /// and fractional index ordering.
    ///
    /// # Arguments
    /// * `source_path` - Path to the node to copy
    /// * `target_parent` - Path to the parent where the copy will be placed
    /// * `new_name` - Optional new name for the root of the copied tree
    ///
    /// # Returns
    /// The root node of the copied tree
    pub(in crate::repositories::nodes) async fn copy_node_tree_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        source_path: &str,
        target_parent: &str,
        new_name: Option<&str>,
        operation_meta: Option<raisin_models::operations::OperationMeta>,
    ) -> Result<Node> {
        // VALIDATION 1: Cannot copy root node
        self.validate_not_root_node(source_path)?;

        // VALIDATION 2: Source must exist
        let source = self
            .get_by_path_impl(tenant_id, repo_id, branch, workspace, source_path, None)
            .await?
            .ok_or_else(|| raisin_error::Error::NotFound("Source node not found".to_string()))?;

        // VALIDATION 3: Check for circular reference (cannot copy into own descendant)
        // target_parent cannot be equal to or start with source_path
        if target_parent == source_path || target_parent.starts_with(&format!("{}/", source_path)) {
            return Err(raisin_error::Error::Validation(format!(
                "Cannot copy '{}' into its own descendant '{}'",
                source_path, target_parent
            )));
        }

        // VALIDATION 4 (MINIMAL): Check target doesn't exist
        let name = new_name.unwrap_or(&source.name);
        let new_path = format!("{}/{}", target_parent, name);

        if self
            .get_by_path_impl(tenant_id, repo_id, branch, workspace, &new_path, None)
            .await?
            .is_some()
        {
            return Err(raisin_error::Error::Conflict(format!(
                "Target path '{}' already exists",
                new_path
            )));
        }

        // VALIDATION 5 (MINIMAL): Check parent allows child type
        // Only validate the root of the copied tree - assume source tree is internally valid
        let target_parent_node = self
            .validate_parent_exists(tenant_id, repo_id, branch, workspace, target_parent)
            .await?;

        self.validate_parent_allows_child(
            BranchScope::new(tenant_id, repo_id, branch),
            &target_parent_node.node_type,
            &source.node_type,
        )
        .await?;

        // STEP 1: Allocate SINGLE revision for entire tree copy operation
        let revision = self.revision_repo.allocate_revision();

        tracing::info!(
            "copy_node_tree_impl: source_path={}, target={}, revision={}, using atomic single-revision approach",
            source_path,
            new_path,
            revision
        );

        // STEP 2: Use prefix scan to collect all descendants (no recursion!)
        let descendants = self.scan_descendants_ordered_impl(
            tenant_id, repo_id, branch, workspace, &source.id, None,
        )?;

        tracing::debug!(
            "copy_node_tree_impl: collected {} nodes to copy",
            descendants.len()
        );

        // STEP 3: Build ID mapping and prepare nodes iteratively
        let mut id_mapping: HashMap<String, String> = HashMap::new();
        let mut path_mapping: HashMap<String, String> = HashMap::new();
        let mut order_label_mapping: HashMap<String, String> = HashMap::new();
        let mut path_to_old_id: HashMap<String, String> = HashMap::new();

        // Build path_to_old_id mapping for later parent ID lookups
        for (node, _) in &descendants {
            path_to_old_id.insert(node.path.clone(), node.id.clone());
        }

        // Get fractional index labels for all nodes to preserve order
        for (node, _depth) in &descendants {
            if let Some(parent_name) = &node.parent {
                if parent_name != "/" {
                    let parent_path = node.path.rsplit_once('/').map(|x| x.0).unwrap_or("/");

                    if let Some(parent_node) = self
                        .get_by_path_impl(tenant_id, repo_id, branch, workspace, parent_path, None)
                        .await?
                    {
                        if let Some(label) = self.get_order_label_for_child(
                            tenant_id,
                            repo_id,
                            branch,
                            workspace,
                            &parent_node.id,
                            &node.id,
                        )? {
                            order_label_mapping.insert(node.id.clone(), label);
                        }
                    }
                }
            }
        }

        // STEP 4: Create WriteBatch for atomic operation
        let mut batch = WriteBatch::default();
        let mut copied_node_ids = Vec::new();
        let mut translation_change_infos: Vec<raisin_storage::NodeChangeInfo> = Vec::new();
        let now = chrono::Utc::now();

        let cf_translation_data = cf_handle(&self.db, cf::TRANSLATION_DATA)?;
        let cf_translation_index = cf_handle(&self.db, cf::TRANSLATION_INDEX)?;
        let cf_block_translations = cf_handle(&self.db, cf::BLOCK_TRANSLATIONS)?;
        let cf_revisions = cf_handle(&self.db, cf::REVISIONS)?;

        let (translation_actor, translation_message, translation_is_system) =
            if let Some(meta) = operation_meta.as_ref() {
                (meta.actor.clone(), meta.message.clone(), meta.is_system)
            } else {
                (
                    "system".to_string(),
                    format!("Copy tree {} -> {}", source_path, new_path),
                    true,
                )
            };

        // STEP 5: Process nodes in breadth-first order (parents before children)
        // Collect operation capture data for replication
        let mut nodes_for_replication: Vec<(Node, Option<String>, Option<String>)> = Vec::new();

        for (source_node, depth) in descendants {
            let new_id = nanoid::nanoid!();
            id_mapping.insert(source_node.id.clone(), new_id.clone());
            copied_node_ids.push(new_id.clone());

            // Calculate new path based on depth
            let new_node_path = if depth == 0 {
                new_path.clone()
            } else {
                let relative_path = source_node
                    .path
                    .strip_prefix(&format!("{}/", source.path))
                    .unwrap_or(&source_node.path);
                format!("{}/{}", new_path, relative_path)
            };

            path_mapping.insert(source_node.path.clone(), new_node_path.clone());

            // Construct new node
            let mut new_node = source_node.clone();
            new_node.id = new_id.clone();
            new_node.path = new_node_path.clone();
            new_node.name = if depth == 0 {
                name.to_string()
            } else {
                source_node.name.clone()
            };
            new_node.created_at = Some(now);
            new_node.updated_at = Some(now);
            new_node.has_children = None; // Never store computed field
            new_node.children = vec![]; // Clear children list

            // Update parent reference
            new_node.parent = Node::extract_parent_name_from_path(&new_node_path);

            // Determine the NEW parent ID for ORDERED_CHILDREN index
            let new_parent_id: Option<String> = if depth == 0 {
                Some(target_parent_node.id.clone())
            } else if let Some(_source_parent_name) = &source_node.parent {
                let source_parent_path = source_node
                    .path
                    .rsplit_once('/')
                    .map(|x| x.0)
                    .unwrap_or("/");
                if source_parent_path != "/" {
                    if let Some(old_parent_id) = path_to_old_id.get(source_parent_path) {
                        id_mapping.get(old_parent_id).cloned()
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            // Get fractional index label (preserve source order)
            let mut order_label_owned: Option<String> = None;
            if let Some(ref parent_id) = new_parent_id {
                if let Some(existing) = order_label_mapping.get(&source_node.id) {
                    order_label_owned = Some(existing.clone());
                } else {
                    let next_label = match self
                        .get_last_order_label(tenant_id, repo_id, branch, workspace, parent_id)?
                    {
                        Some(last) => match crate::fractional_index::inc(&last) {
                            Ok(label) => label,
                            Err(e) => {
                                tracing::warn!(
                                    parent_id = %parent_id,
                                    last_label = %last,
                                    error = %e,
                                    "Corrupt order label detected in copy, falling back to first()"
                                );
                                crate::fractional_index::first()
                            }
                        },
                        None => crate::fractional_index::first(),
                    };
                    order_label_mapping.insert(source_node.id.clone(), next_label.clone());
                    order_label_owned = Some(next_label);
                }
            }

            // Add node to batch with SAME revision and parent ID override
            self.add_node_to_batch_with_parent_id(
                &mut batch,
                &new_node,
                tenant_id,
                repo_id,
                branch,
                workspace,
                &revision,
                order_label_owned.as_deref(),
                new_parent_id.as_deref(),
            )?;

            // Copy latest node-level translations (if any)
            let node_translations = self.collect_node_translations_for_copy(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &source_node.id,
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
                    branch,
                    workspace,
                    &new_id,
                    locale.as_str(),
                    &revision,
                );
                batch.put_cf(&cf_translation_data, data_key, overlay_bytes.clone());

                let index_key = Self::translation_index_key(
                    tenant_id,
                    repo_id,
                    locale.as_str(),
                    &revision,
                    &new_id,
                );
                batch.put_cf(&cf_translation_index, index_key, b"");

                let translation_meta = TranslationMeta {
                    locale: locale.clone(),
                    revision,
                    parent_revision: parent_translation_revision,
                    timestamp: now,
                    actor: translation_actor.clone(),
                    message: translation_message.clone(),
                    is_system: translation_is_system,
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
                    branch,
                    workspace,
                    &new_id,
                    locale.as_str(),
                    &revision,
                );
                batch.put_cf(&cf_revisions, meta_key, meta_bytes);

                let snapshot_key = keys::translation_snapshot_key(
                    tenant_id,
                    repo_id,
                    &new_id,
                    locale.as_str(),
                    &revision,
                );
                batch.put_cf(&cf_revisions, snapshot_key, overlay_bytes.clone());

                translation_change_infos.push(raisin_storage::NodeChangeInfo {
                    node_id: new_id.clone(),
                    workspace: workspace.to_string(),
                    operation: ChangeOperation::Added,
                    translation_locale: Some(locale.as_str().to_string()),
                });
            }

            // Copy block-level translations (if any)
            let block_translations = self.collect_block_translations_for_copy(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &source_node.id,
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
                    branch,
                    workspace,
                    &new_id,
                    &block_uuid,
                    locale.as_str(),
                    &revision,
                );
                batch.put_cf(&cf_block_translations, block_key, overlay_bytes.clone());

                let snapshot_key = keys::translation_snapshot_key(
                    tenant_id,
                    repo_id,
                    &new_id,
                    &format!("{}::{}", locale.as_str(), block_uuid),
                    &revision,
                );
                batch.put_cf(&cf_revisions, snapshot_key, overlay_bytes.clone());

                translation_change_infos.push(raisin_storage::NodeChangeInfo {
                    node_id: new_id.clone(),
                    workspace: workspace.to_string(),
                    operation: ChangeOperation::Added,
                    translation_locale: Some(format!("{}::{}", locale.as_str(), block_uuid)),
                });
            }

            if let (Some(parent_id), Some(label)) =
                (new_parent_id.as_ref(), order_label_owned.as_ref())
            {
                let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;
                let metadata_key =
                    keys::last_child_metadata_key(tenant_id, repo_id, branch, workspace, parent_id);
                batch.put_cf(cf_ordered, metadata_key, label.as_bytes());
            }

            // Collect node information for operation capture
            nodes_for_replication.push((new_node, new_parent_id, order_label_owned));
        }

        // STEP 6: Atomic commit - all nodes created in single WriteBatch
        self.db
            .write(batch)
            .map_err(|e| raisin_error::Error::storage(format!("Atomic copy_tree failed: {}", e)))?;

        tracing::info!(
            "copy_node_tree_impl: successfully copied {} nodes with single revision {}",
            copied_node_ids.len(),
            revision
        );

        // STEP 7: Index all node changes for this revision
        for node_id in &copied_node_ids {
            self.revision_repo
                .index_node_change(tenant_id, repo_id, &revision, node_id)
                .await?;
        }

        // STEP 7.5: Capture CreateNode operations for replication
        self.capture_tree_copy_operations(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &operation_meta,
            &nodes_for_replication,
        )
        .await;

        // STEP 8: Store operation metadata with ALL copied node IDs
        if let Some(mut op_meta) = operation_meta {
            op_meta.revision = revision;
            op_meta.node_id = id_mapping
                .get(&source.id)
                .ok_or_else(|| {
                    raisin_error::Error::internal("Source node ID not found in mapping after copy")
                })?
                .clone();

            // Create NodeChangeInfo for each copied node
            let mut changed_nodes: Vec<raisin_storage::NodeChangeInfo> = copied_node_ids
                .iter()
                .map(|node_id| raisin_storage::NodeChangeInfo {
                    node_id: node_id.clone(),
                    workspace: workspace.to_string(),
                    translation_locale: None,
                    operation: ChangeOperation::Added,
                })
                .collect();

            changed_nodes.extend(translation_change_infos.into_iter());

            let rev_meta = raisin_storage::RevisionMeta {
                revision,
                parent: op_meta.parent_revision,
                merge_parent: None,
                branch: branch.to_string(),
                timestamp: op_meta.timestamp,
                actor: op_meta.actor.clone(),
                message: op_meta.message.clone(),
                is_system: op_meta.is_system,
                changed_nodes,
                changed_node_types: Vec::new(),
                changed_archetypes: Vec::new(),
                changed_element_types: Vec::new(),
                operation: Some(op_meta),
            };

            self.revision_repo
                .store_revision_meta(tenant_id, repo_id, rev_meta)
                .await?;
        }

        // STEP 9: Update branch HEAD to the new revision
        self.branch_repo
            .update_head(tenant_id, repo_id, branch, revision)
            .await?;

        // STEP 10: Return the copied root node
        let root_new_id = id_mapping.get(&source.id).ok_or_else(|| {
            raisin_error::Error::internal("Source node ID not found in mapping after copy")
        })?;
        self.get_impl(tenant_id, repo_id, branch, workspace, root_new_id, false)
            .await?
            .ok_or_else(|| raisin_error::Error::storage("Failed to retrieve copied root node"))
    }

    /// Capture CreateNode operations for replication during tree copy.
    async fn capture_tree_copy_operations(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        operation_meta: &Option<raisin_models::operations::OperationMeta>,
        nodes_for_replication: &[(Node, Option<String>, Option<String>)],
    ) {
        if self.operation_capture.is_enabled() {
            let actor = operation_meta
                .as_ref()
                .map(|m| m.actor.clone())
                .unwrap_or_else(|| "system".to_string());

            for (node, parent_id, order_label) in nodes_for_replication {
                let properties_json =
                    serde_json::to_value(&node.properties).unwrap_or(serde_json::json!({}));

                let _ = self
                    .operation_capture
                    .capture_create_node(
                        tenant_id.to_string(),
                        repo_id.to_string(),
                        branch.to_string(),
                        node.id.clone(),
                        node.name.clone(),
                        node.node_type.clone(),
                        node.archetype.clone(),
                        parent_id.clone(),
                        order_label.clone().unwrap_or_else(|| "a0".to_string()),
                        properties_json,
                        node.owner_id.clone(),
                        Some(workspace.to_string()),
                        node.path.clone(),
                        actor.clone(),
                    )
                    .await;
            }
        }
    }
}
