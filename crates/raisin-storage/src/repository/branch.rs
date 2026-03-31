// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Branch management storage trait

use raisin_context::{
    Branch, BranchDivergence, ConflictResolution, MergeConflict, MergeResult, MergeStrategy,
};
use raisin_error::Result;
use raisin_hlc::HLC;

/// Branch management storage operations.
///
/// Provides operations for managing Git-like branches within repositories.
pub trait BranchRepository: Send + Sync {
    /// Create a new branch
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch_name` - Name for the new branch
    /// * `created_by` - Actor creating the branch
    /// * `from_revision` - Optional revision to branch from (None = create from scratch)
    /// * `upstream_branch` - Optional upstream branch for divergence comparison
    /// * `protected` - Whether the branch is protected from deletion
    /// * `include_revision_history` - Whether to copy revision history from source branch (via background job)
    ///
    /// # Returns
    /// The created branch
    fn create_branch(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        created_by: &str,
        from_revision: Option<HLC>,
        upstream_branch: Option<String>,
        protected: bool,
        include_revision_history: bool,
    ) -> impl std::future::Future<Output = Result<Branch>> + Send;

    /// Get branch information
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch_name` - Branch name
    ///
    /// # Returns
    /// Branch information if it exists
    fn get_branch(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
    ) -> impl std::future::Future<Output = Result<Option<Branch>>> + Send;

    /// List all branches in a repository
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    ///
    /// # Returns
    /// Vector of branches
    fn list_branches(
        &self,
        tenant_id: &str,
        repo_id: &str,
    ) -> impl std::future::Future<Output = Result<Vec<Branch>>> + Send;

    /// Delete a branch
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch_name` - Branch name
    ///
    /// # Returns
    /// `true` if deleted, `false` if not found
    fn delete_branch(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
    ) -> impl std::future::Future<Output = Result<bool>> + Send;

    /// Get current HEAD revision for a branch
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch_name` - Branch name
    ///
    /// # Returns
    /// Current HEAD revision (HLC timestamp)
    fn get_head(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send;

    /// Update HEAD pointer for a branch (fast-forward)
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch_name` - Branch name
    /// * `new_head` - New HEAD revision (HLC timestamp)
    fn update_head(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        new_head: HLC,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Set upstream branch for divergence tracking
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch_name` - Branch name
    /// * `upstream` - Upstream branch name (None to unset)
    fn set_upstream_branch(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        upstream: Option<String>,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Set branch protected status
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch_name` - Branch name
    /// * `protected` - Whether the branch should be protected
    fn set_protected(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        protected: bool,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Set branch description
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch_name` - Branch name
    /// * `description` - Branch description (None to clear)
    fn set_description(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        description: Option<String>,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Calculate branch divergence (commits ahead/behind) between two branches
    ///
    /// Returns how many commits the current branch is ahead/behind the base branch,
    /// similar to Git's divergence tracking.
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `current_branch` - The branch to compare (e.g., "feature/new-ui")
    /// * `base_branch` - The base branch to compare against (e.g., "main")
    ///
    /// # Returns
    /// `BranchDivergence` with ahead/behind counts and common ancestor revision
    fn calculate_divergence(
        &self,
        tenant_id: &str,
        repo_id: &str,
        current_branch: &str,
        base_branch: &str,
    ) -> impl std::future::Future<Output = Result<BranchDivergence>> + Send;

    /// Merge two branches using Git-like three-way merge
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
    /// `MergeResult` containing success status, revision, and any conflicts
    fn merge_branches(
        &self,
        tenant_id: &str,
        repo_id: &str,
        target_branch: &str,
        source_branch: &str,
        strategy: MergeStrategy,
        message: &str,
        actor: &str,
    ) -> impl std::future::Future<Output = Result<MergeResult>> + Send;

    /// Find merge conflicts between two branches without performing the merge
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `target_branch` - The branch being merged into (ours)
    /// * `source_branch` - The branch being merged from (theirs)
    ///
    /// # Returns
    /// Vector of `MergeConflict` objects describing each conflict
    fn find_merge_conflicts(
        &self,
        tenant_id: &str,
        repo_id: &str,
        target_branch: &str,
        source_branch: &str,
    ) -> impl std::future::Future<Output = Result<Vec<MergeConflict>>> + Send;

    /// Complete a merge by applying user-provided conflict resolutions
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `target_branch` - Branch to merge into (will be updated)
    /// * `source_branch` - Branch being merged from
    /// * `resolutions` - User's resolution for each conflicted node
    /// * `message` - Commit message for the merge
    /// * `actor` - User or system performing the merge
    ///
    /// # Returns
    /// `MergeResult` containing the merge commit revision and statistics
    fn resolve_merge_with_resolutions(
        &self,
        tenant_id: &str,
        repo_id: &str,
        target_branch: &str,
        source_branch: &str,
        resolutions: Vec<ConflictResolution>,
        message: &str,
        actor: &str,
    ) -> impl std::future::Future<Output = Result<MergeResult>> + Send;
}
