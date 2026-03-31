//! Registry repository implementation for tenant and deployment tracking

use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_events::{Event, EventBus, RepositoryEvent, RepositoryEventKind};
use raisin_models::registry::{DeploymentRegistration, TenantRegistration};
use raisin_storage::RegistryRepository;
use rocksdb::DB;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
pub struct RegistryRepositoryImpl {
    db: Arc<DB>,
    event_bus: Arc<dyn EventBus>,
    operation_capture: Option<Arc<crate::OperationCapture>>,
}

impl RegistryRepositoryImpl {
    pub fn new(db: Arc<DB>, event_bus: Arc<dyn EventBus>) -> Self {
        Self {
            db,
            event_bus,
            operation_capture: None,
        }
    }

    pub fn new_with_capture(
        db: Arc<DB>,
        event_bus: Arc<dyn EventBus>,
        operation_capture: Arc<crate::OperationCapture>,
    ) -> Self {
        Self {
            db,
            event_bus,
            operation_capture: Some(operation_capture),
        }
    }
}

impl RegistryRepository for RegistryRepositoryImpl {
    async fn register_tenant(
        &self,
        tenant_id: &str,
        metadata: HashMap<String, String>,
    ) -> Result<()> {
        // Check if this is a new tenant
        let is_new_tenant = self.get_tenant(tenant_id).await?.is_none();

        let registration = TenantRegistration {
            tenant_id: tenant_id.to_string(),
            created_at: if is_new_tenant {
                chrono::Utc::now()
            } else {
                // Keep original created_at if updating
                self.get_tenant(tenant_id)
                    .await?
                    .map(|t| t.created_at)
                    .unwrap_or_else(chrono::Utc::now)
            },
            last_seen: chrono::Utc::now(),
            deployments: Vec::new(),
            metadata,
        };

        let key = keys::tenant_key(tenant_id);
        let value = rmp_serde::to_vec(&registration)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        let cf = cf_handle(&self.db, cf::REGISTRY)?;
        self.db
            .put_cf(cf, key, value)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        // Capture operation for replication
        if let Some(ref capture) = self.operation_capture {
            if capture.is_enabled() {
                let _op = capture
                    .capture_update_tenant(
                        tenant_id.to_string(),
                        registration.clone(),
                        "system".to_string(),
                    )
                    .await;
                // Ignore capture errors - don't fail tenant creation if replication fails
            }
        }

        // Emit TenantCreated event for new tenants
        if is_new_tenant {
            let event = Event::Repository(RepositoryEvent {
                tenant_id: tenant_id.to_string(),
                repository_id: String::new(), // No specific repository for tenant events
                kind: RepositoryEventKind::TenantCreated,
                workspace: None,
                revision_id: None,
                branch_name: None,
                tag_name: None,
                message: None,
                actor: None,
                metadata: Some(
                    registration
                        .metadata
                        .iter()
                        .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
                        .collect(),
                ),
            });
            self.event_bus.publish(event);
        }

        Ok(())
    }

    async fn get_tenant(&self, tenant_id: &str) -> Result<Option<TenantRegistration>> {
        let key = keys::tenant_key(tenant_id);
        let cf = cf_handle(&self.db, cf::REGISTRY)?;

        match self.db.get_cf(cf, key) {
            Ok(Some(bytes)) => {
                let registration = rmp_serde::from_slice(&bytes).map_err(|e| {
                    raisin_error::Error::storage(format!("Deserialization error: {}", e))
                })?;
                Ok(Some(registration))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(raisin_error::Error::storage(e.to_string())),
        }
    }

    async fn list_tenants(&self) -> Result<Vec<TenantRegistration>> {
        let prefix = keys::KeyBuilder::new().push("tenants").build_prefix();

        let cf = cf_handle(&self.db, cf::REGISTRY)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        let mut tenants = Vec::new();

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&prefix_clone) {
                break;
            }

            let tenant: TenantRegistration = rmp_serde::from_slice(&value).map_err(|e| {
                raisin_error::Error::storage(format!("Deserialization error: {}", e))
            })?;
            tenants.push(tenant);
        }

        Ok(tenants)
    }

    async fn update_tenant_last_seen(&self, tenant_id: &str) -> Result<()> {
        if let Some(mut tenant) = self.get_tenant(tenant_id).await? {
            tenant.last_seen = chrono::Utc::now();

            let key = keys::tenant_key(tenant_id);
            let value = rmp_serde::to_vec(&tenant)
                .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

            let cf = cf_handle(&self.db, cf::REGISTRY)?;
            self.db
                .put_cf(cf, key, value)
                .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        }

        Ok(())
    }

    async fn register_deployment(&self, tenant_id: &str, deployment_key: &str) -> Result<()> {
        let registration = DeploymentRegistration {
            tenant_id: tenant_id.to_string(),
            deployment_key: deployment_key.to_string(),
            created_at: chrono::Utc::now(),
            last_seen: chrono::Utc::now(),
            nodetype_version: None,
            node_count: None,
        };

        let key = keys::deployment_key(tenant_id, deployment_key);
        let value = rmp_serde::to_vec(&registration)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        let cf = cf_handle(&self.db, cf::REGISTRY)?;
        self.db
            .put_cf(cf, key, value)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        // Capture operation for replication
        if let Some(ref capture) = self.operation_capture {
            if capture.is_enabled() {
                let _op = capture
                    .capture_update_deployment(
                        tenant_id.to_string(),
                        deployment_key.to_string(),
                        registration.clone(),
                        "system".to_string(),
                    )
                    .await;
                // Ignore capture errors
            }
        }

        Ok(())
    }

    async fn get_deployment(
        &self,
        tenant_id: &str,
        deployment_key: &str,
    ) -> Result<Option<DeploymentRegistration>> {
        let key = keys::deployment_key(tenant_id, deployment_key);
        let cf = cf_handle(&self.db, cf::REGISTRY)?;

        match self.db.get_cf(cf, key) {
            Ok(Some(bytes)) => {
                let registration = rmp_serde::from_slice(&bytes).map_err(|e| {
                    raisin_error::Error::storage(format!("Deserialization error: {}", e))
                })?;
                Ok(Some(registration))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(raisin_error::Error::storage(e.to_string())),
        }
    }

    async fn list_deployments(
        &self,
        tenant_id: Option<&str>,
    ) -> Result<Vec<DeploymentRegistration>> {
        let prefix = if let Some(tid) = tenant_id {
            keys::KeyBuilder::new()
                .push("deployments")
                .push(tid)
                .build_prefix()
        } else {
            keys::KeyBuilder::new().push("deployments").build_prefix()
        };

        let cf = cf_handle(&self.db, cf::REGISTRY)?;
        let prefix_clone = prefix.clone();
        let iter = self.db.prefix_iterator_cf(cf, prefix);

        let mut deployments = Vec::new();

        for item in iter {
            let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&prefix_clone) {
                break;
            }

            let deployment: DeploymentRegistration =
                rmp_serde::from_slice(&value).map_err(|e| {
                    raisin_error::Error::storage(format!("Deserialization error: {}", e))
                })?;
            deployments.push(deployment);
        }

        Ok(deployments)
    }

    async fn update_deployment_nodetype_version(
        &self,
        tenant_id: &str,
        deployment_key: &str,
        version: &str,
    ) -> Result<()> {
        if let Some(mut deployment) = self.get_deployment(tenant_id, deployment_key).await? {
            deployment.nodetype_version = Some(version.to_string());

            let key = keys::deployment_key(tenant_id, deployment_key);
            let value = rmp_serde::to_vec(&deployment)
                .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

            let cf = cf_handle(&self.db, cf::REGISTRY)?;
            self.db
                .put_cf(cf, key, value)
                .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        }

        Ok(())
    }

    async fn update_deployment_last_seen(
        &self,
        tenant_id: &str,
        deployment_key: &str,
    ) -> Result<()> {
        if let Some(mut deployment) = self.get_deployment(tenant_id, deployment_key).await? {
            deployment.last_seen = chrono::Utc::now();

            let key = keys::deployment_key(tenant_id, deployment_key);
            let value = rmp_serde::to_vec(&deployment)
                .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

            let cf = cf_handle(&self.db, cf::REGISTRY)?;
            self.db
                .put_cf(cf, key, value)
                .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        }

        Ok(())
    }
}
