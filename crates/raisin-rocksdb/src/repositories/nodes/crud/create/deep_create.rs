//! Deep node creation with automatic parent directory creation.
//!
//! Creates a node at any path, automatically creating parent directories
//! as needed. All nodes are created in a single atomic transaction.

use super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_storage::{NodeRepository, RevisionRepository};
use rocksdb::WriteBatch;
use std::collections::HashMap;

impl NodeRepositoryImpl {
    /// Create a deep node with automatic parent directory creation
    ///
    /// This function creates a node at any path, automatically creating parent directories
    /// as needed. All nodes (parents and target) are created in a single atomic transaction
    /// with the same revision.
    pub(in super::super::super) async fn create_deep_node_impl(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        path: &str,
        mut node: Node,
        parent_node_type: &str,
        options: raisin_storage::CreateNodeOptions,
    ) -> Result<Node> {
        // Parse path into segments
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

        if segments.is_empty() {
            return Err(raisin_error::Error::Validation(
                "Path cannot be empty".to_string(),
            ));
        }

        // STEP 1: Allocate SINGLE revision for all operations
        let revision = self.revision_repo.allocate_revision();

        // STEP 2: Collect nodes that need to be created (with IDs and parent IDs)
        let mut nodes_to_create: Vec<(Node, String)> = Vec::new();

        // Track created parent IDs for calculating order labels
        let mut path_to_id: HashMap<String, String> = HashMap::new();
        path_to_id.insert("/".to_string(), "/".to_string());

        // Check and collect missing parent folders
        for i in 1..segments.len() {
            let parent_path = format!("/{}", segments[..i].join("/"));

            let existing_parent = self
                .get_by_path_impl(tenant_id, repo_id, branch, workspace, &parent_path, None)
                .await?;

            if let Some(existing) = existing_parent {
                path_to_id.insert(parent_path, existing.id);
            } else {
                let parent_id = nanoid::nanoid!();
                let parent_name = segments[i - 1].to_string();
                let parent_parent_path = if i == 1 {
                    "/".to_string()
                } else {
                    format!("/{}", segments[..i - 1].join("/"))
                };
                let parent_parent_id = path_to_id
                    .get(&parent_parent_path)
                    .cloned()
                    .unwrap_or_else(|| "/".to_string());

                let parent_node = Node {
                    id: parent_id.clone(),
                    path: parent_path.clone(),
                    name: parent_name,
                    parent: Some(parent_parent_path.clone()),
                    node_type: parent_node_type.to_string(),
                    properties: HashMap::new(),
                    children: Vec::new(),
                    order_key: "a0".to_string(),
                    has_children: None,
                    version: 1,
                    archetype: None,
                    created_at: Some(chrono::Utc::now()),
                    updated_at: Some(chrono::Utc::now()),
                    created_by: node.created_by.clone(),
                    updated_by: node.updated_by.clone(),
                    published_at: None,
                    published_by: None,
                    translations: None,
                    tenant_id: Some(tenant_id.to_string()),
                    workspace: Some(workspace.to_string()),
                    owner_id: None,
                    relations: Vec::new(),
                };

                path_to_id.insert(parent_path, parent_id.clone());
                nodes_to_create.push((parent_node, parent_parent_id));
            }
        }

        // STEP 3: Prepare target node
        let target_parent = if segments.len() == 1 {
            Some("/".to_string())
        } else {
            Some(format!("/{}", segments[..segments.len() - 1].join("/")))
        };

        node.path = path.to_string();
        if node.name.is_empty() {
            node.name = segments
                .last()
                .ok_or_else(|| raisin_error::Error::invalid_state("Path has no segments"))?
                .to_string();
        }

        let target_parent_id = path_to_id
            .get(&target_parent.clone().unwrap_or_else(|| "/".to_string()))
            .cloned()
            .unwrap_or_else(|| "/".to_string());

        node.parent = target_parent;
        if node.id.is_empty() {
            node.id = nanoid::nanoid!();
        }

        // Validate target node (but skip parent validation since we're creating parents)
        let validation_options = raisin_storage::CreateNodeOptions {
            validate_parent_allows_child: false,
            ..options
        };
        self.validate_for_create(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &node,
            &validation_options,
        )
        .await?;

        nodes_to_create.push((node.clone(), target_parent_id.clone()));

        // STEP 4: Create WriteBatch with SAME revision for all nodes
        let mut batch = WriteBatch::default();
        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;

        let mut last_labels: HashMap<String, String> = HashMap::new();

        for (node_to_create, parent_id) in &nodes_to_create {
            self.add_node_to_batch(
                &mut batch,
                node_to_create,
                tenant_id,
                repo_id,
                branch,
                workspace,
                &revision,
                None,
            )?;

            // Calculate fractional index order label
            let order_label = if let Some(last) = last_labels.get(parent_id) {
                match crate::fractional_index::inc(last) {
                    Ok(label) => label,
                    Err(e) => {
                        tracing::warn!(
                            parent_id = %parent_id,
                            last_label = %last,
                            error = %e,
                            "Corrupt cached order label in deep create, falling back to first()"
                        );
                        crate::fractional_index::first()
                    }
                }
            } else {
                let existing_last =
                    self.get_last_order_label(tenant_id, repo_id, branch, workspace, parent_id)?;
                if let Some(ref last) = existing_last {
                    match crate::fractional_index::inc(last) {
                        Ok(label) => label,
                        Err(e) => {
                            tracing::warn!(
                                parent_id = %parent_id,
                                last_label = %last,
                                error = %e,
                                "Corrupt order label in deep create, falling back to first()"
                            );
                            crate::fractional_index::first()
                        }
                    }
                } else {
                    crate::fractional_index::first()
                }
            };

            last_labels.insert(parent_id.clone(), order_label.clone());

            let ordered_key = keys::ordered_child_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                parent_id,
                &order_label,
                &revision,
                &node_to_create.id,
            );
            batch.put_cf(cf_ordered, ordered_key, node_to_create.name.as_bytes());

            let metadata_key =
                keys::last_child_metadata_key(tenant_id, repo_id, branch, workspace, parent_id);
            batch.put_cf(cf_ordered, metadata_key, order_label.as_bytes());
        }

        // STEP 5: Add revision indexing to batch (ATOMIC)
        for (node_to_create, _) in &nodes_to_create {
            self.revision_repo.index_node_change_to_batch(
                &mut batch,
                tenant_id,
                repo_id,
                &revision,
                &node_to_create.id,
            )?;
        }

        let updated_branch = self
            .branch_repo
            .update_head_to_batch(&mut batch, tenant_id, repo_id, branch, revision)
            .await?;

        // STEP 6: Atomic commit
        self.db
            .write(batch)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        // STEP 7: Capture replication events (after atomic write)
        self.branch_repo
            .capture_head_update_for_replication(
                tenant_id,
                repo_id,
                branch,
                &updated_branch,
                revision,
            )
            .await;

        Ok(node)
    }
}
