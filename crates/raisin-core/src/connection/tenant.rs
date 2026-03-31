//! Tenant-scoped access to repositories.

use raisin_context::{RepositoryContext, RepositoryInfo};
use raisin_error::Result;
use raisin_storage::Storage;
use std::sync::Arc;

use super::core::{RaisinConnection, RaisinConnectionArc};
use super::repository::Repository;

/// Tenant-scoped access to repositories.
///
/// Provides data isolation for multi-tenant deployments. All repository operations
/// within this scope are isolated to the specified tenant.
///
/// # Example
///
/// ```rust,no_run
/// # use raisin_core::RaisinConnection;
/// # use raisin_storage_memory::InMemoryStorage;
/// # use std::sync::Arc;
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// # let storage = Arc::new(InMemoryStorage::default());
/// # let connection = RaisinConnection::with_storage(storage);
/// // Multi-tenant: tenant from HTTP header
/// let tenant = connection.tenant("acme-corp");
/// let repo = tenant.repository("website");
///
/// // List all repositories for this tenant
/// let repos = tenant.list_repositories().await?;
/// # Ok(())
/// # }
/// ```
pub struct TenantScope<'c, S: Storage> {
    pub(super) connection: &'c RaisinConnection<S>,
    pub(super) tenant_id: String,
}

impl<'c, S: Storage> TenantScope<'c, S> {
    /// Access a repository within this tenant.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use raisin_core::RaisinConnection;
    /// # use raisin_storage_memory::InMemoryStorage;
    /// # use std::sync::Arc;
    /// # let storage = Arc::new(InMemoryStorage::default());
    /// # let connection = RaisinConnection::with_storage(storage);
    /// # let tenant = connection.tenant("acme-corp");
    /// let website = tenant.repository("website");
    /// let mobile = tenant.repository("mobile-app");
    /// ```
    pub fn repository(&self, repo_id: impl Into<String>) -> Repository<S> {
        let repo_id = repo_id.into();
        let context = RepositoryContext::new(&self.tenant_id, &repo_id);

        Repository {
            connection: Arc::new(RaisinConnectionArc(
                Arc::clone(&self.connection.storage),
                self.connection.config.clone(),
            )),
            tenant_id: self.tenant_id.clone(),
            repo_id,
            context: Arc::new(context),
        }
    }

    /// List repositories for this tenant.
    pub async fn list_repositories(&self) -> Result<Vec<RepositoryInfo>> {
        // TODO: Implement repository listing from registry
        Ok(Vec::new())
    }

    /// Create a new repository for this tenant.
    pub async fn create_repository(
        &self,
        repo_id: impl Into<String>,
        config: raisin_context::RepositoryConfig,
    ) -> Result<RepositoryInfo> {
        let repo_id = repo_id.into();
        // TODO: Implement repository creation
        Ok(RepositoryInfo {
            tenant_id: self.tenant_id.clone(),
            repo_id,
            created_at: chrono::Utc::now(),
            branches: vec![config.default_branch.clone()],
            config,
        })
    }

    /// Get the tenant ID.
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }
}
