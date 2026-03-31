//! UpdateBuilder for fluent node property updates
//!
//! Provides a builder pattern for updating node properties.

use raisin_error::Result;
use raisin_models as models;
use raisin_models::nodes::audit_log::AuditLogAction;
use raisin_storage::{transactional::TransactionalStorage, Storage};

use super::NodeService;

impl<S: Storage + TransactionalStorage> NodeService<S> {
    /// Create an update builder for updating node properties.
    ///
    /// This provides a fluent API for updating mutable node fields (properties, translations)
    /// while preserving system fields (id, path, name, node_type, created_at).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// service.update(workspace, "/my-node")
    ///     .with_properties(props)
    ///     .with_translations(trans)
    ///     .save()
    ///     .await?;
    /// ```
    pub fn update<'a>(&'a self, workspace: &'a str, path: &'a str) -> UpdateBuilder<'a, S> {
        UpdateBuilder::new(self, workspace, path)
    }
}

/// Builder for updating node properties using the fluent API pattern.
///
/// This builder ensures that only mutable fields (properties, translations) can be updated,
/// while system fields (id, path, name, node_type) remain immutable.
pub struct UpdateBuilder<'a, S: Storage + TransactionalStorage> {
    service: &'a NodeService<S>,
    workspace: &'a str,
    path: &'a str,
    properties: Option<std::collections::HashMap<String, models::nodes::properties::PropertyValue>>,
    translations:
        Option<std::collections::HashMap<String, models::nodes::properties::PropertyValue>>,
}

impl<'a, S: Storage + TransactionalStorage> UpdateBuilder<'a, S> {
    fn new(service: &'a NodeService<S>, workspace: &'a str, path: &'a str) -> Self {
        Self {
            service,
            workspace,
            path,
            properties: None,
            translations: None,
        }
    }

    /// Set the properties to update.
    pub fn with_properties(
        mut self,
        props: std::collections::HashMap<String, models::nodes::properties::PropertyValue>,
    ) -> Self {
        self.properties = Some(props);
        self
    }

    /// Set the translations to update.
    pub fn with_translations(
        mut self,
        trans: std::collections::HashMap<String, models::nodes::properties::PropertyValue>,
    ) -> Self {
        self.translations = Some(trans);
        self
    }

    /// Save the updates to the node.
    ///
    /// This method:
    /// 1. Fetches the existing node by PATH (source of truth)
    /// 2. Applies updates to mutable fields only
    /// 3. Sets updated_at automatically
    /// 4. Validates against NodeType schema
    /// 5. Persists to storage
    /// 6. Logs audit trail if enabled
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The node doesn't exist
    /// - Validation fails
    /// - Storage operation fails
    pub async fn save(self) -> Result<models::nodes::Node> {
        // 1. Fetch existing node by PATH (source of truth)
        let mut node = self
            .service
            .get_by_path(self.path)
            .await?
            .ok_or(raisin_error::Error::NotFound("node".into()))?;

        // 2. Apply updates (ONLY mutable fields)
        if let Some(props) = self.properties {
            node.properties = props;
        }
        if let Some(trans) = self.translations {
            node.translations = Some(trans);
        }

        // 3. System fields set automatically
        node.updated_at = Some(chrono::Utc::now());
        // Note: updated_by could be set from context if available in future

        // 4. NOTE: Schema validation is now performed in transaction layer
        // (TransactionalContext.put_node) for consistent validation across all paths.

        // 5. Persist to storage (with versioning)
        self.service.update_node(node.clone()).await?;

        // 6. Audit if enabled
        if let Some(audit) = &self.service.audit {
            audit.write(&node, AuditLogAction::Update, None).await?;
        }

        Ok(node)
    }
}
