//! Registry operations: tenant, deployment, repository

use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_events::{Event, RepositoryEvent, RepositoryEventKind};
use raisin_models::registry::{DeploymentRegistration, TenantRegistration};
use raisin_replication::Operation;
use std::collections::HashMap;

use super::super::conflict_resolution::should_apply_by_last_seen;
use super::super::db_helpers::serialize_and_write_compact;
use super::OperationApplicator;

impl OperationApplicator {
    /// Apply a tenant update operation
    ///
    /// This writes the tenant directly to the REGISTRY column family.
    /// We bypass the RegistryRepository to avoid triggering operation capture.
    pub(in crate::replication::application) async fn apply_update_tenant(
        &self,
        tenant_id: &str,
        tenant: &TenantRegistration,
        op: &Operation,
    ) -> Result<()> {
        tracing::info!(
            "📥 Applying tenant update: {} from node {}",
            tenant_id,
            op.cluster_node_id
        );

        let key = keys::tenant_key(tenant_id);
        let cf = cf_handle(&self.db, cf::REGISTRY)?;

        // Use LWW conflict resolution helper
        if !should_apply_by_last_seen::<TenantRegistration, _>(
            &self.db,
            cf,
            &key,
            tenant.last_seen,
            |t| t.last_seen,
        )? {
            return Ok(());
        }

        // Serialize and write using helper
        serialize_and_write_compact(
            &self.db,
            cf,
            key,
            tenant,
            &format!("apply_update_tenant_{}", tenant_id),
        )?;

        // Emit TenantCreated event to trigger admin user initialization
        // Note: We always emit this even for updates, as the event handler checks if admin exists
        let event = Event::Repository(RepositoryEvent {
            tenant_id: tenant_id.to_string(),
            repository_id: String::new(),
            kind: RepositoryEventKind::TenantCreated,
            workspace: None,
            revision_id: None,
            branch_name: None,
            tag_name: None,
            message: None,
            actor: None,
            metadata: Some(
                tenant
                    .metadata
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                    .collect(),
            ),
        });
        self.event_bus.publish(event);

        tracing::info!("✅ Tenant applied successfully: {}", tenant_id);
        Ok(())
    }

    /// Apply a deployment update operation
    pub(in crate::replication::application) async fn apply_update_deployment(
        &self,
        tenant_id: &str,
        deployment_id: &str,
        deployment: &DeploymentRegistration,
        op: &Operation,
    ) -> Result<()> {
        tracing::info!(
            "📥 Applying deployment update: {}/{} from node {}",
            tenant_id,
            deployment_id,
            op.cluster_node_id
        );

        let key = keys::deployment_key(tenant_id, deployment_id);
        let cf = cf_handle(&self.db, cf::REGISTRY)?;

        // Use LWW conflict resolution helper
        if !should_apply_by_last_seen::<DeploymentRegistration, _>(
            &self.db,
            cf,
            &key,
            deployment.last_seen,
            |d| d.last_seen,
        )? {
            return Ok(());
        }

        // Serialize and write using helper
        serialize_and_write_compact(
            &self.db,
            cf,
            key,
            deployment,
            &format!("apply_update_deployment_{}/{}", tenant_id, deployment_id),
        )?;

        tracing::info!(
            "✅ Deployment applied successfully: {}/{}",
            tenant_id,
            deployment_id
        );
        Ok(())
    }

    /// Apply a repository update operation
    pub(in crate::replication::application) async fn apply_update_repository(
        &self,
        tenant_id: &str,
        repo_id: &str,
        repository: &raisin_context::RepositoryInfo,
        op: &Operation,
    ) -> Result<()> {
        tracing::info!(
            "📥 Applying repository update: {}/{} from node {}",
            tenant_id,
            repo_id,
            op.cluster_node_id
        );

        // Check if repository exists for LWW
        let key = keys::repository_key(tenant_id, repo_id);
        let cf = cf_handle(&self.db, cf::REGISTRY)?;

        let is_new_repository = match self.db.get_cf(cf, &key) {
            Ok(Some(_bytes)) => false,
            Ok(None) => true,
            Err(e) => {
                tracing::error!("Failed to check existing repository: {}", e);
                return Err(raisin_error::Error::storage(e.to_string()));
            }
        };

        // Serialize and write using helper
        serialize_and_write_compact(
            &self.db,
            cf,
            key,
            repository,
            &format!("apply_update_repository_{}/{}", tenant_id, repo_id),
        )?;

        // Only emit RepositoryCreated event for NEW repositories
        if is_new_repository {
            let event = Event::Repository(RepositoryEvent {
                tenant_id: tenant_id.to_string(),
                repository_id: repo_id.to_string(),
                kind: RepositoryEventKind::Created,
                workspace: None,
                revision_id: None,
                branch_name: Some(repository.config.default_branch.clone()),
                tag_name: None,
                message: None,
                actor: None,
                metadata: None,
            });
            self.event_bus.publish(event);
            tracing::info!(
                "✅ New repository created and applied: {}/{}",
                tenant_id,
                repo_id
            );
        } else {
            tracing::info!(
                "✅ Repository updated and applied: {}/{}",
                tenant_id,
                repo_id
            );
        }

        Ok(())
    }
}
