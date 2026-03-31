//! Publishing and unpublishing workflow methods for NodeService.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::audit_log::AuditLogAction;
use raisin_storage::{BranchRepository, NodeRepository, Storage, VersioningRepository};

use crate::services::node_service::NodeService;

impl<S: Storage + raisin_storage::transactional::TransactionalStorage> NodeService<S> {
    /// Publishes a single node by setting published_at timestamp.
    ///
    /// Per RFC-0001: Creates a version snapshot BEFORE publishing to capture draft state.
    pub async fn publish(&self, node_path: &str) -> Result<()> {
        let mut node = self
            .storage
            .nodes()
            .get_by_path(self.scope(), node_path, self.revision.as_ref())
            .await?
            .ok_or(raisin_error::Error::NotFound("node".into()))?;

        // Create version BEFORE publishing to capture draft state
        self.storage
            .versioning()
            .create_version_with_note(&node, Some("Pre-publish snapshot".to_string()))
            .await?;

        let actor = self
            .auth_context
            .as_ref()
            .map(|ctx| ctx.actor_id())
            .unwrap_or_else(|| "system".to_string());

        node.published_at = Some(chrono::Utc::now());
        node.published_by = Some(actor);
        self.update_node(node.clone()).await?;

        if let Some(a) = &self.audit {
            a.write(&node, AuditLogAction::Publish, None).await?;
        }

        let current_revision = self
            .storage
            .branches()
            .get_branch(&self.tenant_id, &self.repo_id, &self.branch)
            .await?
            .map(|b| b.head)
            .unwrap_or_else(|| HLC::new(0, 0));

        self.storage
            .event_bus()
            .publish(raisin_storage::Event::Node(raisin_storage::NodeEvent {
                tenant_id: self.tenant_id.clone(),
                repository_id: self.repo_id.clone(),
                branch: self.branch.clone(),
                workspace_id: self.workspace_id.clone(),
                node_id: node.id.clone(),
                node_type: Some(node.node_type.clone()),
                revision: current_revision,
                kind: raisin_storage::NodeEventKind::Published,
                path: Some(node.path.clone()),
                metadata: None,
            }));

        Ok(())
    }

    /// Recursively publishes a node and all its descendants.
    ///
    /// Per RFC-0001: Creates version snapshots for ALL nodes BEFORE publishing.
    pub async fn publish_tree(&self, node_path: &str) -> Result<()> {
        let root = self
            .storage
            .nodes()
            .get_by_path(self.scope(), node_path, self.revision.as_ref())
            .await?
            .ok_or(raisin_error::Error::NotFound("node".into()))?;

        let descendants = self
            .storage
            .nodes()
            .deep_children_flat(self.scope(), node_path, 100, self.revision.as_ref())
            .await?;

        // Create versions for root and all descendants
        self.storage
            .versioning()
            .create_version_with_note(&root, Some("Pre-publish snapshot".to_string()))
            .await?;

        for node in &descendants {
            self.storage
                .versioning()
                .create_version_with_note(node, Some("Pre-publish snapshot".to_string()))
                .await?;
        }

        let actor = self
            .auth_context
            .as_ref()
            .map(|ctx| ctx.actor_id())
            .unwrap_or_else(|| "system".to_string());

        // Publish root
        let mut root = root;
        root.published_at = Some(chrono::Utc::now());
        root.published_by = Some(actor.clone());
        self.update_node(root.clone()).await?;
        if let Some(a) = &self.audit {
            a.write(&root, AuditLogAction::Publish, None).await?;
        }

        // Publish all descendants
        for mut node in descendants {
            node.published_at = Some(chrono::Utc::now());
            node.published_by = Some(actor.clone());
            self.update_node(node.clone()).await?;
            if let Some(a) = &self.audit {
                a.write(&node, AuditLogAction::Publish, None).await?;
            }
        }

        Ok(())
    }

    /// Unpublishes a single node by clearing published_at timestamp.
    pub async fn unpublish(&self, node_path: &str) -> Result<()> {
        let mut node = self
            .storage
            .nodes()
            .get_by_path(self.scope(), node_path, self.revision.as_ref())
            .await?
            .ok_or(raisin_error::Error::NotFound("node".into()))?;

        node.published_at = None;
        node.published_by = None;
        self.update_node(node.clone()).await?;

        if let Some(a) = &self.audit {
            a.write(&node, AuditLogAction::Unpublish, None).await?;
        }

        let current_revision = self
            .storage
            .branches()
            .get_branch(&self.tenant_id, &self.repo_id, &self.branch)
            .await?
            .map(|b| b.head)
            .unwrap_or_else(|| HLC::new(0, 0));

        self.storage
            .event_bus()
            .publish(raisin_storage::Event::Node(raisin_storage::NodeEvent {
                tenant_id: self.tenant_id.clone(),
                repository_id: self.repo_id.clone(),
                branch: self.branch.clone(),
                workspace_id: self.workspace_id.clone(),
                node_id: node.id.clone(),
                node_type: Some(node.node_type.clone()),
                revision: current_revision,
                kind: raisin_storage::NodeEventKind::Unpublished,
                path: Some(node.path.clone()),
                metadata: None,
            }));

        Ok(())
    }

    /// Recursively unpublishes a node and all its descendants.
    pub async fn unpublish_tree(&self, node_path: &str) -> Result<()> {
        self.unpublish(node_path).await?;
        let desc = self
            .storage
            .nodes()
            .deep_children_flat(self.scope(), node_path, 100, self.revision.as_ref())
            .await?;
        for mut n in desc {
            n.published_at = None;
            n.published_by = None;
            self.update_node(n.clone()).await?;
            if let Some(a) = &self.audit {
                a.write(&n, AuditLogAction::Unpublish, None).await?;
            }
        }
        Ok(())
    }
}
