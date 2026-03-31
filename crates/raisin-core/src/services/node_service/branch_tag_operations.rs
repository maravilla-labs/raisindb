//! Branch and tag operations for NodeService
//!
//! This module provides methods for managing branches and tags through the NodeService interface.

use raisin_context::{Branch, Tag};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_storage::{BranchRepository, Storage, TagRepository};

use super::NodeService;

impl<S: Storage + raisin_storage::transactional::TransactionalStorage> NodeService<S> {
    /// Create a new branch in the current repository
    ///
    /// # Arguments
    /// * `branch_name` - Name of the branch to create
    /// * `from_revision` - Optional revision to branch from (defaults to current HEAD if None)
    /// * `protected` - Whether the branch should be protected from deletion
    /// * `include_revision_history` - Whether to copy revision history from source branch
    ///
    /// # Returns
    /// The newly created Branch
    pub async fn create_branch(
        &self,
        branch_name: &str,
        from_revision: Option<HLC>,
        upstream_branch: Option<String>,
        protected: bool,
        include_revision_history: bool,
    ) -> Result<Branch> {
        self.storage
            .branches()
            .create_branch(
                &self.tenant_id,
                &self.repo_id,
                branch_name,
                "system", // TODO: Get actual user from context
                from_revision,
                upstream_branch,
                protected,
                include_revision_history,
            )
            .await
    }

    /// Get a branch by name
    pub async fn get_branch(&self, branch_name: &str) -> Result<Option<Branch>> {
        self.storage
            .branches()
            .get_branch(&self.tenant_id, &self.repo_id, branch_name)
            .await
    }

    /// List all branches in the current repository
    pub async fn list_branches(&self) -> Result<Vec<Branch>> {
        self.storage
            .branches()
            .list_branches(&self.tenant_id, &self.repo_id)
            .await
    }

    /// Delete a branch
    ///
    /// # Arguments
    /// * `branch_name` - Name of the branch to delete
    ///
    /// # Returns
    /// `true` if the branch was deleted, `false` if it didn't exist
    pub async fn delete_branch(&self, branch_name: &str) -> Result<bool> {
        self.storage
            .branches()
            .delete_branch(&self.tenant_id, &self.repo_id, branch_name)
            .await
    }

    /// Get the HEAD revision of a branch
    pub async fn get_branch_head(&self, branch_name: &str) -> Result<raisin_hlc::HLC> {
        self.storage
            .branches()
            .get_head(&self.tenant_id, &self.repo_id, branch_name)
            .await
    }

    /// Create a new tag pointing to a specific revision
    ///
    /// # Arguments
    /// * `tag_name` - Name of the tag to create
    /// * `revision` - Revision (HLC timestamp) to tag
    /// * `message` - Optional annotation message
    /// * `protected` - Whether the tag should be protected from deletion
    ///
    /// # Returns
    /// The newly created Tag
    pub async fn create_tag(
        &self,
        tag_name: &str,
        revision: &raisin_hlc::HLC,
        message: Option<String>,
        protected: bool,
    ) -> Result<Tag> {
        self.storage
            .tags()
            .create_tag(
                &self.tenant_id,
                &self.repo_id,
                tag_name,
                revision,
                "system", // TODO: Get actual user from context
                message,
                protected,
            )
            .await
    }

    /// Get a tag by name
    pub async fn get_tag(&self, tag_name: &str) -> Result<Option<Tag>> {
        self.storage
            .tags()
            .get_tag(&self.tenant_id, &self.repo_id, tag_name)
            .await
    }

    /// List all tags in the current repository
    pub async fn list_tags(&self) -> Result<Vec<Tag>> {
        self.storage
            .tags()
            .list_tags(&self.tenant_id, &self.repo_id)
            .await
    }

    /// Delete a tag
    ///
    /// # Arguments
    /// * `tag_name` - Name of the tag to delete
    ///
    /// # Returns
    /// `true` if the tag was deleted, `false` if it didn't exist
    pub async fn delete_tag(&self, tag_name: &str) -> Result<bool> {
        self.storage
            .tags()
            .delete_tag(&self.tenant_id, &self.repo_id, tag_name)
            .await
    }

    /// Get the current branch name for this service instance
    pub fn current_branch(&self) -> &str {
        &self.branch
    }

    /// Get the current repository ID for this service instance
    pub fn current_repository(&self) -> &str {
        &self.repo_id
    }

    /// Get the current tenant ID for this service instance
    pub fn current_tenant(&self) -> &str {
        &self.tenant_id
    }
}
