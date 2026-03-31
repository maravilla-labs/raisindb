use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use raisin_error::Result;
use raisin_models as models;
use raisin_storage::RegistryRepository;

#[derive(Default, Clone)]
pub struct InMemoryRegistryRepo {
    // key: tenant_id
    tenants: Arc<RwLock<HashMap<String, models::registry::TenantRegistration>>>,
    // key: (tenant_id, deployment_key)
    deployments: Arc<RwLock<HashMap<(String, String), models::registry::DeploymentRegistration>>>,
}

impl InMemoryRegistryRepo {
    pub fn new() -> Self {
        Self {
            tenants: Arc::new(RwLock::new(HashMap::new())),
            deployments: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl RegistryRepository for InMemoryRegistryRepo {
    fn register_tenant(
        &self,
        tenant_id: &str,
        metadata: std::collections::HashMap<String, String>,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let tenant_id = tenant_id.to_string();
        async move {
            let mut tenants = self.tenants.write().await;

            // If tenant already exists, update metadata
            if let Some(tenant) = tenants.get_mut(&tenant_id) {
                tenant.metadata = metadata;
                tenant.last_seen = chrono::Utc::now();
            } else {
                // Create new tenant
                let mut tenant = models::registry::TenantRegistration::new(&tenant_id);
                tenant.metadata = metadata;
                tenants.insert(tenant_id, tenant);
            }

            Ok(())
        }
    }

    fn get_tenant(
        &self,
        tenant_id: &str,
    ) -> impl std::future::Future<Output = Result<Option<models::registry::TenantRegistration>>> + Send
    {
        let tenant_id = tenant_id.to_string();
        async move {
            let tenants = self.tenants.read().await;
            Ok(tenants.get(&tenant_id).cloned())
        }
    }

    async fn list_tenants(&self) -> Result<Vec<models::registry::TenantRegistration>> {
        let tenants = self.tenants.read().await;
        Ok(tenants.values().cloned().collect())
    }

    fn update_tenant_last_seen(
        &self,
        tenant_id: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let tenant_id = tenant_id.to_string();
        async move {
            let mut tenants = self.tenants.write().await;
            if let Some(tenant) = tenants.get_mut(&tenant_id) {
                tenant.last_seen = chrono::Utc::now();
            }
            Ok(())
        }
    }

    fn register_deployment(
        &self,
        tenant_id: &str,
        deployment_key: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let tenant_id = tenant_id.to_string();
        let deployment_key = deployment_key.to_string();
        async move {
            let mut deployments = self.deployments.write().await;
            let mut tenants = self.tenants.write().await;

            let key = (tenant_id.clone(), deployment_key.clone());

            // If deployment already exists, just update last_seen
            if let Some(deployment) = deployments.get_mut(&key) {
                deployment.last_seen = chrono::Utc::now();
            } else {
                // Create new deployment
                let deployment =
                    models::registry::DeploymentRegistration::new(&tenant_id, &deployment_key);
                deployments.insert(key, deployment);

                // Update tenant's deployments list
                if let Some(tenant) = tenants.get_mut(&tenant_id) {
                    if !tenant.deployments.contains(&deployment_key) {
                        tenant.deployments.push(deployment_key.clone());
                    }
                }
            }

            Ok(())
        }
    }

    fn get_deployment(
        &self,
        tenant_id: &str,
        deployment_key: &str,
    ) -> impl std::future::Future<Output = Result<Option<models::registry::DeploymentRegistration>>> + Send
    {
        let tenant_id = tenant_id.to_string();
        let deployment_key = deployment_key.to_string();
        async move {
            let deployments = self.deployments.read().await;
            let key = (tenant_id, deployment_key);
            Ok(deployments.get(&key).cloned())
        }
    }

    fn list_deployments(
        &self,
        tenant_id: Option<&str>,
    ) -> impl std::future::Future<Output = Result<Vec<models::registry::DeploymentRegistration>>> + Send
    {
        let tenant_id = tenant_id.map(|s| s.to_string());
        async move {
            let deployments = self.deployments.read().await;

            let result: Vec<models::registry::DeploymentRegistration> = match tenant_id {
                // Filter by tenant_id if provided
                Some(tid) => deployments
                    .iter()
                    .filter(|((t, _), _)| t == &tid)
                    .map(|(_, deployment)| deployment.clone())
                    .collect(),
                // Return all deployments if no tenant_id
                None => deployments.values().cloned().collect(),
            };

            Ok(result)
        }
    }

    fn update_deployment_nodetype_version(
        &self,
        tenant_id: &str,
        deployment_key: &str,
        version: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let tenant_id = tenant_id.to_string();
        let deployment_key = deployment_key.to_string();
        let version = version.to_string();
        async move {
            let mut deployments = self.deployments.write().await;
            let key = (tenant_id, deployment_key);

            if let Some(deployment) = deployments.get_mut(&key) {
                deployment.nodetype_version = Some(version);
                deployment.last_seen = chrono::Utc::now();
            }

            Ok(())
        }
    }

    fn update_deployment_last_seen(
        &self,
        tenant_id: &str,
        deployment_key: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let tenant_id = tenant_id.to_string();
        let deployment_key = deployment_key.to_string();
        async move {
            let mut deployments = self.deployments.write().await;
            let key = (tenant_id, deployment_key);

            if let Some(deployment) = deployments.get_mut(&key) {
                deployment.last_seen = chrono::Utc::now();
            }

            Ok(())
        }
    }
}
