//! Property access and update methods for NodeService
//!
//! This module handles reading and updating individual properties within nodes
//! using path notation (e.g., "user.address.city").

use raisin_error::Result;
use raisin_models as models;
use raisin_models::nodes::audit_log::AuditLogAction;
use raisin_storage::{NodeRepository, Storage};

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
    /// Triggers audit logging if enabled.
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
        self.storage
            .nodes()
            .update_property_by_path(self.scope(), node_path, property_path, value.clone())
            .await?;
        if let Some(a) = &self.audit {
            if let Some(n) = self
                .storage
                .nodes()
                .get_by_path(self.scope(), node_path, self.revision.as_ref())
                .await?
            {
                a.write(
                    &n,
                    AuditLogAction::UpdateProperty,
                    Some(format!("property_path={}", property_path)),
                )
                .await?;
            }
        }
        Ok(())
    }
}
