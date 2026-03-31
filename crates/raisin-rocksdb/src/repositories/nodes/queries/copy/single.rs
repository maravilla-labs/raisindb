//! Single node copy operation.
//!
//! Copies a single node to a new location with full validation
//! (existence, parent, workspace, node type, and uniqueness checks).

use super::super::super::NodeRepositoryImpl;
use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_storage::{
    BranchRepository, BranchScope, NodeRepository, RevisionRepository, StorageScope,
};

impl NodeRepositoryImpl {
    pub(in crate::repositories::nodes) async fn copy_node_impl(
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
        // Validation 1: Cannot copy root node
        self.validate_not_root_node(source_path)?;

        // Validation 2: Source must exist
        let source = self
            .get_by_path_impl(tenant_id, repo_id, branch, workspace, source_path, None)
            .await?
            .ok_or_else(|| raisin_error::Error::NotFound("Source node not found".to_string()))?;

        // Validation 3: Target parent must exist
        let target_parent_node = self
            .validate_parent_exists(tenant_id, repo_id, branch, workspace, target_parent)
            .await?;

        // Validation 4: Check workspace allows this node type
        let is_root_node = target_parent == "/";
        self.validate_workspace_allows_node_type(
            StorageScope::new(tenant_id, repo_id, branch, workspace),
            &source.node_type,
            is_root_node,
        )
        .await?;

        // Validation 5: Check if this child node type is allowed under parent's NodeType schema
        self.validate_parent_allows_child(
            BranchScope::new(tenant_id, repo_id, branch),
            &target_parent_node.node_type,
            &source.node_type,
        )
        .await?;

        // Validation 6: Check for duplicate names in target location
        let name = new_name.unwrap_or(&source.name);
        self.validate_unique_child_name(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &target_parent_node.id,
            name,
        )
        .await?;

        let new_path = format!("{}/{}", target_parent, name);

        let mut new_node = source.clone();
        new_node.id = nanoid::nanoid!();
        new_node.path = new_path.clone();
        new_node.name = name.to_string();
        new_node.created_at = Some(chrono::Utc::now());
        new_node.updated_at = Some(chrono::Utc::now());

        // Use add_impl since we're creating a new node (copy creates a brand new node with new ID)
        self.add_impl(tenant_id, repo_id, branch, workspace, new_node.clone())
            .await?;

        // Get revision after add_impl (add_impl allocates its own revision)
        let revision = self.revision_repo.allocate_revision();

        // Store operation metadata if provided
        if let Some(mut op_meta) = operation_meta {
            // Update operation metadata with the actual revision and new node ID
            op_meta.revision = revision;
            op_meta.node_id = new_node.id.clone();

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

        Ok(new_node)
    }
}
