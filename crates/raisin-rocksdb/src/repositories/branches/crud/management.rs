//! Branch management operations (set_upstream, set_protected, set_description)
//!
//! Additional branch operations not part of the BranchRepository trait.

use crate::{cf, cf_handle, keys};
use raisin_context::Branch;
use raisin_error::Result;

use super::super::BranchRepositoryImpl;

impl BranchRepositoryImpl {
    /// Set or clear the upstream branch for divergence comparison
    pub async fn set_upstream_branch(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        upstream: Option<&str>,
    ) -> Result<Branch> {
        use raisin_storage::BranchRepository;

        let mut branch = self
            .get_branch(tenant_id, repo_id, branch_name)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Branch '{}' not found", branch_name))
            })?;

        // Validate upstream branch exists if provided
        if let Some(upstream_name) = upstream {
            let upstream_exists = self
                .get_branch(tenant_id, repo_id, upstream_name)
                .await?
                .is_some();
            if !upstream_exists {
                return Err(raisin_error::Error::NotFound(format!(
                    "Upstream branch '{}' not found",
                    upstream_name
                )));
            }
        }

        branch.upstream_branch = upstream.map(|s| s.to_string());

        let key = keys::branch_key(tenant_id, repo_id, branch_name);
        let value = rmp_serde::to_vec(&branch)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        let cf = cf_handle(&self.db, cf::BRANCHES)?;
        self.db
            .put_cf(cf, key, value)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        Ok(branch)
    }

    /// Set the protected status of a branch
    pub async fn set_protected(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        protected: bool,
    ) -> Result<Branch> {
        use raisin_storage::BranchRepository;

        let mut branch = self
            .get_branch(tenant_id, repo_id, branch_name)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Branch '{}' not found", branch_name))
            })?;

        branch.protected = protected;

        let key = keys::branch_key(tenant_id, repo_id, branch_name);
        let value = rmp_serde::to_vec(&branch)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        let cf = cf_handle(&self.db, cf::BRANCHES)?;
        self.db
            .put_cf(cf, key, value)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        Ok(branch)
    }

    /// Set the description of a branch
    pub async fn set_description(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        description: Option<&str>,
    ) -> Result<Branch> {
        use raisin_storage::BranchRepository;

        let mut branch = self
            .get_branch(tenant_id, repo_id, branch_name)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Branch '{}' not found", branch_name))
            })?;

        branch.description = description.map(|s| s.to_string());

        let key = keys::branch_key(tenant_id, repo_id, branch_name);
        let value = rmp_serde::to_vec(&branch)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        let cf = cf_handle(&self.db, cf::BRANCHES)?;
        self.db
            .put_cf(cf, key, value)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        Ok(branch)
    }
}
