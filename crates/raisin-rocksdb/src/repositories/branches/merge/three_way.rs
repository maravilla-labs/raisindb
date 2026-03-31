//! Three-way and fast-forward merge execution.
//!
//! Contains the primary `merge_branches` method which orchestrates
//! both fast-forward and three-way merge strategies.

use raisin_context::{MergeResult, MergeStrategy};
use raisin_error::Result;
use raisin_storage::{BranchRepository, RevisionMeta, RevisionRepository};
use std::collections::{HashSet, VecDeque};

use super::super::BranchRepositoryImpl;

impl BranchRepositoryImpl {
    /// Merge two branches using Git-like three-way merge
    ///
    /// This method orchestrates a complete merge operation between two branches, handling
    /// both fast-forward and three-way merges. It detects conflicts, creates merge commits,
    /// and updates branch pointers.
    ///
    /// # Merge Strategies
    ///
    /// ## FastForward
    /// - Only succeeds if target is a direct ancestor of source
    /// - Simply moves target branch pointer to source HEAD
    /// - No merge commit created
    /// - Equivalent to `git merge --ff-only`
    ///
    /// ## ThreeWay
    /// - Performs full three-way merge analysis
    /// - Detects conflicts between branches
    /// - Creates merge commit with both parents
    /// - Fails if conflicts are detected (manual resolution required)
    ///
    /// # Algorithm
    /// 1. Check if fast-forward is possible (for FastForward strategy)
    /// 2. Find conflicts by comparing changes since common ancestor
    /// 3. If conflicts exist, return failed result with conflict details
    /// 4. If no conflicts:
    ///    - Allocate new revision number
    ///    - Create merge commit with parent and merge_parent
    ///    - Update target branch HEAD
    ///    - Copy changed nodes from source to target branch
    /// 5. Return success result with revision number
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `target_branch` - Branch to merge into (will be updated)
    /// * `source_branch` - Branch to merge from (remains unchanged)
    /// * `strategy` - Merge strategy (FastForward or ThreeWay)
    /// * `message` - Commit message for the merge
    /// * `actor` - User or system performing the merge
    ///
    /// # Returns
    /// `MergeResult` containing:
    /// - `success`: true if merge completed, false if conflicts exist
    /// - `revision`: Some(n) if merge succeeded, None if conflicts
    /// - `conflicts`: Vector of conflicts if any were found
    /// - `fast_forward`: true if this was a fast-forward merge
    /// - `nodes_changed`: Count of nodes affected by merge
    ///
    /// # Errors
    /// Returns error if:
    /// - Either branch doesn't exist
    /// - FastForward strategy requested but fast-forward not possible
    /// - Revision allocation fails
    /// - Branch update fails
    /// - Database operations fail
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// // Fast-forward merge (only if possible)
    /// let result = branch_repo.merge_branches(
    ///     "tenant1",
    ///     "repo1",
    ///     "main",
    ///     "feature/new-ui",
    ///     MergeStrategy::FastForward,
    ///     "Merge feature/new-ui into main",
    ///     "user-123"
    /// ).await?;
    ///
    /// // Three-way merge with conflict detection
    /// let result = branch_repo.merge_branches(
    ///     "tenant1",
    ///     "repo1",
    ///     "main",
    ///     "feature/refactor",
    ///     MergeStrategy::ThreeWay,
    ///     "Merge feature/refactor into main",
    ///     "user-123"
    /// ).await?;
    ///
    /// if !result.success {
    ///     println!("Merge conflicts found: {:?}", result.conflicts);
    ///     // Manual conflict resolution required
    /// }
    /// ```
    pub async fn merge_branches(
        &self,
        tenant_id: &str,
        repo_id: &str,
        target_branch: &str,
        source_branch: &str,
        strategy: MergeStrategy,
        message: &str,
        actor: &str,
    ) -> Result<MergeResult> {
        use crate::repositories::revisions::RevisionRepositoryImpl;

        // Get both branches
        let target = self
            .get_branch(tenant_id, repo_id, target_branch)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!(
                    "Target branch '{}' not found",
                    target_branch
                ))
            })?;

        // Check if target branch is protected
        if target.protected {
            return Err(raisin_error::Error::Forbidden(format!(
                "Cannot merge into protected branch '{}'",
                target_branch
            )));
        }

        let source = self
            .get_branch(tenant_id, repo_id, source_branch)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!(
                    "Source branch '{}' not found",
                    source_branch
                ))
            })?;

        // Check if branches are already in sync
        if target.head == source.head {
            return Ok(MergeResult {
                success: true,
                revision: Some(target.head.timestamp_ms),
                conflicts: Vec::new(),
                fast_forward: false,
                nodes_changed: 0,
            });
        }

        // For FastForward strategy, check if fast-forward is possible
        if matches!(strategy, MergeStrategy::FastForward) {
            let can_ff = self
                .can_fast_forward(tenant_id, repo_id, target_branch, source_branch)
                .await?;

            if !can_ff {
                return Err(raisin_error::Error::Validation(
                    "Fast-forward merge not possible: branches have diverged".to_string(),
                ));
            }

            // Fast-forward: just update target branch HEAD to source HEAD
            self.update_head(tenant_id, repo_id, target_branch, source.head)
                .await?;

            return Ok(MergeResult {
                success: true,
                revision: Some(source.head.timestamp_ms),
                conflicts: Vec::new(),
                fast_forward: true,
                nodes_changed: 0, // No new commit, just pointer movement
            });
        }

        // Three-way merge: check for conflicts
        let conflicts = self
            .find_merge_conflicts(tenant_id, repo_id, target_branch, source_branch)
            .await?;

        if !conflicts.is_empty() {
            return Ok(MergeResult {
                success: false,
                revision: None,
                conflicts,
                fast_forward: false,
                nodes_changed: 0,
            });
        }

        // No conflicts - proceed with merge
        let node_id = format!("branch-{}-{}", tenant_id, repo_id);
        let rev_repo = RevisionRepositoryImpl::new(self.db.clone(), node_id);

        // Calculate divergence to get common ancestor and count changes
        let divergence = self
            .calculate_divergence(tenant_id, repo_id, source_branch, target_branch)
            .await?;

        // Allocate new revision for merge commit
        let merge_revision = rev_repo.allocate_revision();

        // Collect all changed nodes from source branch since common ancestor
        // Use BFS to follow BOTH parent AND merge_parent (handles merge commits correctly)
        let mut changed_nodes = Vec::new();
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(source.head);
        visited.insert(source.head);

        while let Some(revision) = queue.pop_front() {
            // Stop at common ancestor (don't include changes from before the divergence)
            if revision == divergence.common_ancestor {
                continue;
            }

            let Some(meta) = rev_repo
                .get_revision_meta(tenant_id, repo_id, &revision)
                .await?
            else {
                continue;
            };

            changed_nodes.extend(meta.changed_nodes.clone());

            // Follow BOTH parent and merge_parent to traverse all ancestors
            if let Some(parent) = meta.parent {
                if visited.insert(parent) {
                    queue.push_back(parent);
                }
            }
            if let Some(merge_parent) = meta.merge_parent {
                if visited.insert(merge_parent) {
                    queue.push_back(merge_parent);
                }
            }
        }

        // Create merge commit metadata
        let merge_meta = RevisionMeta {
            revision: merge_revision,
            parent: Some(target.head),
            merge_parent: Some(source.head),
            branch: target_branch.to_string(),
            timestamp: chrono::Utc::now(),
            actor: actor.to_string(),
            message: message.to_string(),
            is_system: false,
            changed_nodes: changed_nodes.clone(),
            changed_node_types: Vec::new(),
            changed_archetypes: Vec::new(),
            changed_element_types: Vec::new(),
            operation: None,
        };

        // Store merge commit metadata
        rev_repo
            .store_revision_meta(tenant_id, repo_id, merge_meta)
            .await?;

        // Update target branch HEAD to the merge commit
        self.update_head(tenant_id, repo_id, target_branch, merge_revision)
            .await?;

        // Copy changed indexes from source branch to target branch
        // This ensures all changes from source are visible in target
        self.copy_branch_indexes(
            tenant_id,
            repo_id,
            source_branch,
            target_branch,
            &source.head,
        )
        .await?;

        Ok(MergeResult {
            success: true,
            revision: Some(merge_revision.timestamp_ms),
            conflicts: Vec::new(),
            fast_forward: false,
            nodes_changed: changed_nodes.len(),
        })
    }
}
