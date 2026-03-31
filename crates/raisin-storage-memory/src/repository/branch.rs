//! In-memory branch management implementation.

use raisin_context::{
    Branch, BranchDivergence, ConflictResolution, MergeConflict, MergeResult, MergeStrategy,
};
use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use raisin_storage::BranchRepository;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// In-memory branch management
#[derive(Clone)]
pub struct InMemoryBranchRepo {
    /// branches: key = "{tenant_id}/{repo_id}/{branch_name}" -> Branch
    branches: Arc<RwLock<HashMap<String, Branch>>>,
    event_bus: Arc<dyn raisin_storage::EventBus>,
}

impl Default for InMemoryBranchRepo {
    fn default() -> Self {
        Self {
            branches: Arc::new(RwLock::new(HashMap::new())),
            event_bus: Arc::new(raisin_storage::InMemoryEventBus::new()),
        }
    }
}

impl InMemoryBranchRepo {
    pub fn new(event_bus: Arc<dyn raisin_storage::EventBus>) -> Self {
        Self {
            branches: Arc::new(RwLock::new(HashMap::new())),
            event_bus,
        }
    }

    fn make_key(tenant_id: &str, repo_id: &str, branch_name: &str) -> String {
        format!("{}/{}/{}", tenant_id, repo_id, branch_name)
    }
}

impl BranchRepository for InMemoryBranchRepo {
    async fn create_branch(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        created_by: &str,
        from_revision: Option<HLC>,
        upstream_branch: Option<String>,
        protected: bool,
        _include_revision_history: bool, // In-memory storage doesn't copy revision history
    ) -> Result<Branch> {
        let key = Self::make_key(tenant_id, repo_id, branch_name);
        let mut branches = self.branches.write().await;

        if branches.contains_key(&key) {
            return Err(raisin_error::Error::Conflict(format!(
                "Branch {} already exists in {}/{}",
                branch_name, tenant_id, repo_id
            )));
        }

        let branch = Branch {
            name: branch_name.to_string(),
            head: from_revision.unwrap_or(HLC::new(0, 0)),
            created_by: created_by.to_string(),
            protected,
            created_at: chrono::Utc::now(),
            created_from: from_revision,
            upstream_branch,
            description: None,
        };

        branches.insert(key, branch.clone());
        drop(branches);

        // Emit BranchCreated event
        self.event_bus.publish(raisin_storage::Event::Repository(
            raisin_storage::RepositoryEvent {
                tenant_id: tenant_id.to_string(),
                repository_id: repo_id.to_string(),
                kind: raisin_storage::RepositoryEventKind::BranchCreated,
                workspace: None,
                revision_id: from_revision.map(|r| r.to_string()),
                branch_name: Some(branch_name.to_string()),
                tag_name: None,
                message: None,
                actor: Some(created_by.to_string()),
                metadata: None,
            },
        ));

        Ok(branch)
    }

    async fn get_branch(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
    ) -> Result<Option<Branch>> {
        let key = Self::make_key(tenant_id, repo_id, branch_name);
        let branches = self.branches.read().await;
        Ok(branches.get(&key).cloned())
    }

    async fn list_branches(&self, tenant_id: &str, repo_id: &str) -> Result<Vec<Branch>> {
        let prefix = format!("{}/{}/", tenant_id, repo_id);
        let branches = self.branches.read().await;
        Ok(branches
            .iter()
            .filter_map(|(key, branch)| {
                if key.starts_with(&prefix) {
                    Some(branch.clone())
                } else {
                    None
                }
            })
            .collect())
    }

    async fn delete_branch(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
    ) -> Result<bool> {
        let key = Self::make_key(tenant_id, repo_id, branch_name);
        let mut branches = self.branches.write().await;

        // Check if branch exists and if it's protected
        if let Some(branch) = branches.get(&key) {
            if branch.protected {
                return Err(raisin_error::Error::Forbidden(format!(
                    "Cannot delete protected branch '{}'",
                    branch_name
                )));
            }
        }

        let deleted = branches.remove(&key).is_some();
        drop(branches);

        if deleted {
            // Emit BranchDeleted event
            self.event_bus.publish(raisin_storage::Event::Repository(
                raisin_storage::RepositoryEvent {
                    tenant_id: tenant_id.to_string(),
                    repository_id: repo_id.to_string(),
                    kind: raisin_storage::RepositoryEventKind::BranchDeleted,
                    workspace: None,
                    revision_id: None,
                    branch_name: Some(branch_name.to_string()),
                    tag_name: None,
                    message: None,
                    actor: None,
                    metadata: None,
                },
            ));
        }

        Ok(deleted)
    }

    async fn get_head(&self, tenant_id: &str, repo_id: &str, branch_name: &str) -> Result<HLC> {
        let key = Self::make_key(tenant_id, repo_id, branch_name);
        let branches = self.branches.read().await;
        branches
            .get(&key)
            .map(|b| b.head)
            .ok_or_else(|| raisin_error::Error::NotFound(format!("Branch {}", branch_name)))
    }

    async fn update_head(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        new_head: HLC,
    ) -> Result<()> {
        let key = Self::make_key(tenant_id, repo_id, branch_name);
        let mut branches = self.branches.write().await;

        if let Some(branch) = branches.get_mut(&key) {
            // Check if branch is protected
            if branch.protected {
                return Err(raisin_error::Error::Forbidden(format!(
                    "Cannot modify protected branch '{}': branch is protected from commits",
                    branch_name
                )));
            }

            branch.head = new_head;
            drop(branches);

            // Emit BranchUpdated event
            self.event_bus.publish(raisin_storage::Event::Repository(
                raisin_storage::RepositoryEvent {
                    tenant_id: tenant_id.to_string(),
                    repository_id: repo_id.to_string(),
                    kind: raisin_storage::RepositoryEventKind::BranchUpdated,
                    workspace: None,
                    revision_id: Some(new_head.to_string()),
                    branch_name: Some(branch_name.to_string()),
                    tag_name: None,
                    message: None,
                    actor: None,
                    metadata: None,
                },
            ));

            Ok(())
        } else {
            Err(raisin_error::Error::NotFound(format!(
                "Branch {}",
                branch_name
            )))
        }
    }

    async fn set_upstream_branch(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        upstream: Option<String>,
    ) -> Result<()> {
        let key = Self::make_key(tenant_id, repo_id, branch_name);
        let mut branches = self.branches.write().await;

        if let Some(branch) = branches.get_mut(&key) {
            branch.upstream_branch = upstream;
            Ok(())
        } else {
            Err(raisin_error::Error::NotFound(format!(
                "Branch {}",
                branch_name
            )))
        }
    }

    async fn set_protected(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        protected: bool,
    ) -> Result<()> {
        let key = Self::make_key(tenant_id, repo_id, branch_name);
        let mut branches = self.branches.write().await;

        if let Some(branch) = branches.get_mut(&key) {
            branch.protected = protected;
            Ok(())
        } else {
            Err(raisin_error::Error::NotFound(format!(
                "Branch {}",
                branch_name
            )))
        }
    }

    async fn set_description(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        description: Option<String>,
    ) -> Result<()> {
        let key = Self::make_key(tenant_id, repo_id, branch_name);
        let mut branches = self.branches.write().await;

        if let Some(branch) = branches.get_mut(&key) {
            branch.description = description;
            Ok(())
        } else {
            Err(raisin_error::Error::NotFound(format!(
                "Branch {}",
                branch_name
            )))
        }
    }

    async fn calculate_divergence(
        &self,
        tenant_id: &str,
        repo_id: &str,
        current_branch: &str,
        base_branch: &str,
    ) -> Result<BranchDivergence> {
        // Get both branches
        let current = self
            .get_branch(tenant_id, repo_id, current_branch)
            .await?
            .ok_or_else(|| Error::NotFound(format!("Branch '{}'", current_branch)))?;

        let _base = self
            .get_branch(tenant_id, repo_id, base_branch)
            .await?
            .ok_or_else(|| Error::NotFound(format!("Branch '{}'", base_branch)))?;

        // In-memory storage doesn't track revision history, just compare HEADs
        // If both branches have the same HEAD, they are in sync
        // Otherwise, return 0,0 since we can't walk the revision history
        Ok(BranchDivergence {
            ahead: 0,
            behind: 0,
            common_ancestor: current.head,
        })
    }

    async fn merge_branches(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _target_branch: &str,
        _source_branch: &str,
        _strategy: MergeStrategy,
        _message: &str,
        _actor: &str,
    ) -> Result<MergeResult> {
        // In-memory storage doesn't support merge operations
        Err(Error::Validation(
            "Merge operations are not supported in in-memory storage".to_string(),
        ))
    }

    async fn find_merge_conflicts(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _target_branch: &str,
        _source_branch: &str,
    ) -> Result<Vec<MergeConflict>> {
        // In-memory storage doesn't support merge conflict detection
        Err(Error::Validation(
            "Merge conflict detection is not supported in in-memory storage".to_string(),
        ))
    }

    async fn resolve_merge_with_resolutions(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _target_branch: &str,
        _source_branch: &str,
        _resolutions: Vec<ConflictResolution>,
        _message: &str,
        _actor: &str,
    ) -> Result<MergeResult> {
        // In-memory storage doesn't support merge with conflict resolution
        Err(Error::Validation(
            "Merge with conflict resolution is not supported in in-memory storage".to_string(),
        ))
    }
}
