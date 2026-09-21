//! Property access and update methods for NodeService
//!
//! This module handles reading and updating individual properties within nodes
//! using path notation (e.g., "user.address.city").

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models as models;
use raisin_models::nodes::audit_log::AuditLogAction;
use raisin_models::permissions::Operation;
use raisin_storage::{BranchRepository, NodeRepository, Storage};

use super::NodeService;

impl<S: Storage + raisin_storage::transactional::TransactionalStorage> NodeService<S> {
    /// Gets a specific property value by path notation
    ///
    /// # Example
    /// ```ignore
    /// let value = service.get_property_by_path("/user/john", "profile.email").await?;
    /// ```
    pub async fn get_property_by_path(
        &self,
        node_path: &str,
        property_path: &str,
    ) -> Result<Option<models::nodes::properties::PropertyValue>> {
        self.storage
            .nodes()
            .get_property_by_path(
                self.scope(),
                node_path,
                property_path,
                self.revision.as_ref(),
            )
            .await
    }

    /// Updates a specific property value by path notation
    ///
    /// Checks the caller may update the node, triggers audit logging if
    /// enabled, and emits `Updated` like every other node write.
    ///
    /// Both used to be missing. With no event, a `node_event` trigger never
    /// saw the change: switching an automation off through
    /// `PUT …/<node>@enabled` left its compiled trigger armed, still firing on
    /// real content (measured 2026-09-21). With no RLS check, this was the one
    /// REST write a caller's row-level permissions did not govern.
    ///
    /// # Example
    /// ```ignore
    /// service.update_property_by_path(
    ///     "/user/john",
    ///     "profile.email",
    ///     PropertyValue::String("john@example.com".into())
    /// ).await?;
    /// ```
    pub async fn update_property_by_path(
        &self,
        node_path: &str,
        property_path: &str,
        value: models::nodes::properties::PropertyValue,
    ) -> Result<()> {
        let existing = self
            .storage
            .nodes()
            .get_by_path(self.scope(), node_path, self.revision.as_ref())
            .await?
            .ok_or_else(|| raisin_error::Error::NotFound(format!("Node not found: {node_path}")))?;
        if !self
            .check_rls_permission(&existing, Operation::Update)
            .await
        {
            return Err(raisin_error::Error::PermissionDenied(format!(
                "Permission denied: cannot update node '{}' at path '{}'",
                existing.id, existing.path
            )));
        }

        self.storage
            .nodes()
            .update_property_by_path(self.scope(), node_path, property_path, value.clone())
            .await?;
        let updated = self
            .storage
            .nodes()
            .get_by_path(self.scope(), node_path, self.revision.as_ref())
            .await?;
        if self.audit.is_some() {
            if let Some(n) = &updated {
                self.audit_write(
                    n,
                    AuditLogAction::UpdateProperty,
                    Some(format!("property_path={}", property_path)),
                )
                .await?;
            }
        }

        let revision = self
            .storage
            .branches()
            .get_branch(&self.tenant_id, &self.repo_id, &self.branch)
            .await?
            .map(|b| b.head)
            .unwrap_or_else(|| HLC::new(0, 0));
        let node = updated.as_ref().unwrap_or(&existing);
        self.storage
            .event_bus()
            .publish(raisin_storage::Event::Node(raisin_storage::NodeEvent {
                tenant_id: self.tenant_id.clone(),
                repository_id: self.repo_id.clone(),
                branch: self.branch.clone(),
                workspace_id: self.workspace_id.clone(),
                node_id: node.id.clone(),
                node_type: Some(node.node_type.clone()),
                revision,
                kind: raisin_storage::NodeEventKind::Updated,
                path: Some(node.path.clone()),
                metadata: None,
            }));
        Ok(())
    }
}
