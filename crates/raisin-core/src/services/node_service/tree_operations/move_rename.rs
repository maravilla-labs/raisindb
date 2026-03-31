//! Move and rename operations for nodes.
//!
//! Provides methods for moving nodes to new parents and renaming nodes,
//! including published-state validation and descendant checks.

use raisin_error::Result;
use raisin_models::nodes::audit_log::AuditLogAction;
use raisin_storage::{NodeRepository, Storage};

use super::super::NodeService;
use crate::sanitize_name;

impl<S: Storage + raisin_storage::transactional::TransactionalStorage> NodeService<S> {
    /// Moves a node to a new parent using paths (recommended)
    ///
    /// This is the preferred method for moving nodes as it uses paths directly
    /// and encapsulates all logic in the service layer.
    ///
    /// # Arguments
    /// * `from_path` - Current path of the node to move
    /// * `to_parent_path` - Path of the new parent
    ///
    /// # Authorization
    ///
    /// Requires update permission on the source node and create permission at destination.
    ///
    /// # Example
    /// ```ignore
    /// service.move_to("/folder1/item", "/folder2").await?;
    /// // Result: item is now at /folder2/item
    /// ```
    pub async fn move_to(&self, from_path: &str, to_parent_path: &str) -> Result<()> {
        use raisin_models::permissions::Operation;

        // Fetch node to get ID and name
        let node = self
            .get_by_path(from_path)
            .await?
            .ok_or(raisin_error::Error::NotFound("node".into()))?;

        // RLS Authorization: Check if user can update the source node
        if !self.check_rls_permission(&node, Operation::Update) {
            return Err(raisin_error::Error::PermissionDenied(format!(
                "Permission denied: cannot move node from path '{}'",
                from_path
            )));
        }

        // Construct full destination path: parent + node name
        let new_path = format!("{}/{}", to_parent_path.trim_end_matches('/'), node.name);

        // RLS Authorization: Check if user can create at destination path
        if !self.check_rls_create_permission(&new_path, &node.node_type) {
            return Err(raisin_error::Error::PermissionDenied(format!(
                "Permission denied: cannot move node to path '{}'",
                new_path
            )));
        }

        // VALIDATION: Cannot move published nodes - must unpublish first
        if node.published_at.is_some() {
            return Err(raisin_error::Error::Validation(
                "Cannot move published node - unpublish first".to_string(),
            ));
        }

        // VALIDATION: Check if any descendants are published
        let descendants = self
            .storage
            .nodes()
            .deep_children_flat(self.scope(), from_path, 100, self.revision.as_ref())
            .await?;

        for descendant in descendants {
            if descendant.published_at.is_some() {
                return Err(raisin_error::Error::Validation(
                    format!("Cannot move node - descendant '{}' is published. Unpublish the entire tree first.", descendant.path)
                ));
            }
        }

        // Delegate to ID-based move with full path
        self.move_node(&node.id, &new_path).await
    }

    /// Moves a node to a new path in the hierarchy (ID-based)
    ///
    /// NOTE: Prefer using `move_to()` which takes paths instead.
    /// This method is kept for backward compatibility.
    pub async fn move_node(&self, id: &str, new_path: &str) -> Result<()> {
        // VALIDATION: Get the node and check if it or its descendants are published
        let node = self
            .storage
            .nodes()
            .get(self.scope(), id, self.revision.as_ref())
            .await?
            .ok_or(raisin_error::Error::NotFound(format!(
                "Node with id {} not found",
                id
            )))?;

        // Cannot move published nodes
        if node.published_at.is_some() {
            return Err(raisin_error::Error::Validation(
                "Cannot move published node - unpublish first".to_string(),
            ));
        }

        // Check if any descendants are published
        let descendants = self
            .storage
            .nodes()
            .deep_children_flat(self.scope(), &node.path, 100, self.revision.as_ref())
            .await?;

        for descendant in descendants {
            if descendant.published_at.is_some() {
                return Err(raisin_error::Error::Validation(
                    format!("Cannot move node - descendant '{}' is published. Unpublish the entire tree first.", descendant.path)
                ));
            }
        }

        // Perform the move
        self.storage
            .nodes()
            .move_node(
                self.scope(),
                id,
                new_path,
                None, // TODO: Accept operation metadata from caller
            )
            .await?;
        if let Some(a) = &self.audit {
            if let Some(n) = self
                .storage
                .nodes()
                .get(self.scope(), id, self.revision.as_ref())
                .await?
            {
                a.write(
                    &n,
                    AuditLogAction::Move,
                    Some(format!("new_path={}", new_path)),
                )
                .await?;
            }
        }
        Ok(())
    }

    /// Renames a node and updates all descendant paths accordingly
    ///
    /// # Authorization
    ///
    /// Requires update permission on the node.
    pub async fn rename_node(&self, old_path: &str, new_name: &str) -> Result<()> {
        use raisin_models::permissions::Operation;

        let new_name = sanitize_name(new_name)?;
        let before = self
            .storage
            .nodes()
            .get_by_path(self.scope(), old_path, self.revision.as_ref())
            .await?;

        // VALIDATION: Cannot rename published nodes - must unpublish first
        if let Some(node) = &before {
            // RLS Authorization: Check if user can update this node
            if !self.check_rls_permission(node, Operation::Update) {
                return Err(raisin_error::Error::PermissionDenied(format!(
                    "Permission denied: cannot rename node at path '{}'",
                    old_path
                )));
            }

            if node.published_at.is_some() {
                return Err(raisin_error::Error::Validation(
                    "Cannot rename published node - unpublish first".to_string(),
                ));
            }
        }

        // VALIDATION: Check if any descendants are published
        let descendants = self
            .storage
            .nodes()
            .deep_children_flat(self.scope(), old_path, 100, self.revision.as_ref())
            .await?;

        for descendant in descendants {
            if descendant.published_at.is_some() {
                return Err(raisin_error::Error::Validation(
                    format!("Cannot rename node - descendant '{}' is published. Unpublish the entire tree first.", descendant.path)
                ));
            }
        }

        let new_path = if let Some(idx) = old_path.rfind('/') {
            format!("{}/{}", &old_path[..idx], new_name)
        } else {
            format!("/{}", new_name)
        };
        self.storage
            .nodes()
            .rename_node(self.scope(), old_path, &new_name)
            .await?;
        // Note: With physical ROOT node using IDs in children array,
        // renaming doesn't affect ROOT node since IDs don't change
        // No action needed for root-level nodes
        if let Some(a) = &self.audit {
            if let Some(n) = self
                .storage
                .nodes()
                .get_by_path(self.scope(), &new_path, self.revision.as_ref())
                .await?
            {
                a.write(
                    &n,
                    AuditLogAction::Rename,
                    Some(format!("new_name={}", new_name)),
                )
                .await?;
            }
        }
        Ok(())
    }
}
