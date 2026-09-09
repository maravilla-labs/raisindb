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
    Branch, BranchDiff, BranchDivergence, ConflictResolution, MergeConflict, MergeResult,
    MergeStrategy,
};
use raisin_error::Result;
use raisin_hlc::HLC;

/// Everything a branch fork can be told, beyond the four identifiers.
///
/// `create_branch` takes these as positional arguments and conflates two of
/// them: it derives the branch to COPY DATA FROM out of `upstream_branch`,
/// falling back to `main`. That is fine for the callers that pass the fork
/// source as the upstream, and wrong for a caller that has a source and an
/// upstream that differ — SQL's `CREATE BRANCH b FROM 'a' UPSTREAM 'c'`, or
/// `FROM 'a'` with no upstream at all, which forked from `a` and then copied
/// `main`'s indexes over it.
///
/// This struct separates them, and carries the description the older signature
/// had no room for.
#[derive(Debug, Clone, Default)]
pub struct CreateBranchOptions {
    /// Branch whose data and indexes the new branch starts from.
    ///
    /// `None` with a `from_revision` set means `main`, preserving the older
    /// behaviour. `None` with no revision means an empty branch.
    pub source_branch: Option<String>,
    /// Revision to fork at. `None` means the source branch's current HEAD.
    pub from_revision: Option<HLC>,
    /// Upstream branch recorded for divergence comparison. Independent of
    /// `source_branch`: what you forked from and what you track need not match.
    pub upstream_branch: Option<String>,
    /// Whether the branch refuses deletion and merges into it.
    pub protected: bool,
    /// Whether to copy the source branch's revision history (background job).
    pub include_revision_history: bool,
    /// Human-readable description stored on the branch record.
    pub description: Option<String>,
}

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

    /// Create a branch, naming the fork source and the upstream separately and
    /// carrying a description.
    ///
    /// The default body forwards to [`Self::create_branch`], which cannot honour
    /// `source_branch` independently of `upstream_branch` and drops the
    /// description — a backend that cares overrides this. RocksDB does.
    fn create_branch_with_options(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        created_by: &str,
        options: CreateBranchOptions,
    ) -> impl std::future::Future<Output = Result<Branch>> + Send {
        async move {
            self.create_branch(
                tenant_id,
                repo_id,
                branch_name,
                created_by,
                options.from_revision,
                options
                    .upstream_branch
                    .or_else(|| options.source_branch.clone()),
                options.protected,
                options.include_revision_history,
            )
            .await
        }
    }

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

    /// Set HEAD pointer for a branch unconditionally (rollback/reset).
    ///
    /// Unlike `update_head` (fast-forward only, safe for concurrent commits),
    /// this moves the head to ANY revision, including backwards. Use only for
    /// deliberate operator actions (branch reset to an earlier revision).
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch_name` - Branch name
    /// * `new_head` - New HEAD revision (HLC timestamp)
    fn set_head(
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

    /// Compute the per-node diff of `branch` relative to `base_branch`'s merge-base
    ///
    /// Unlike `calculate_divergence` (which returns only ahead/behind counts),
    /// this enumerates exactly which nodes changed since the two branches
    /// diverged, classified into added / modified / deleted.
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `branch` - The branch whose changes are enumerated (e.g. "feature/new-ui")
    /// * `base_branch` - The base branch to diff against (e.g. "main")
    ///
    /// # Returns
    /// `BranchDiff` with the common ancestor and the added/modified/deleted node lists
    fn diff_branches(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        base_branch: &str,
    ) -> impl std::future::Future<Output = Result<BranchDiff>> + Send;

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
