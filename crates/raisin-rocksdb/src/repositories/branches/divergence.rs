//! Branch divergence calculation
//!
//! Calculates how many commits ahead/behind one branch is compared to another,
//! similar to Git's divergence tracking.

use raisin_context::BranchDivergence;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_storage::RevisionRepository;
use std::collections::{HashSet, VecDeque};

use super::BranchRepositoryImpl;

impl BranchRepositoryImpl {
    /// Calculate branch divergence (commits ahead/behind) between two branches
    ///
    /// This method finds the common ancestor between two branches and counts:
    /// - **ahead**: Commits in current_branch but not in base_branch (after common ancestor)
    /// - **behind**: Commits in base_branch but not in current_branch (after common ancestor)
    ///
    /// # Algorithm
    /// 1. Get both branches and their HEAD revisions
    /// 2. Walk back through revision parent chains from both HEADs
    /// 3. Find the common ancestor (first revision in both chains)
    /// 4. Count revisions after the common ancestor in each chain
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `current_branch` - The branch to compare (e.g., "feature/new-ui")
    /// * `base_branch` - The base branch to compare against (e.g., "main")
    ///
    /// # Returns
    /// `BranchDivergence` with ahead/behind counts and common ancestor revision
    ///
    /// # Errors
    /// Returns error if either branch doesn't exist or if revision metadata is corrupted
    pub async fn calculate_divergence(
        &self,
        tenant_id: &str,
        repo_id: &str,
        current_branch: &str,
        base_branch: &str,
    ) -> Result<BranchDivergence> {
        use crate::repositories::revisions::RevisionRepositoryImpl;
        use raisin_storage::BranchRepository;

        // Get both branches
        let current = self
            .get_branch(tenant_id, repo_id, current_branch)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Branch '{}' not found", current_branch))
            })?;

        let base = self
            .get_branch(tenant_id, repo_id, base_branch)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Branch '{}' not found", base_branch))
            })?;

        // If branches point to same HEAD, they're in sync
        if current.head == base.head {
            return Ok(BranchDivergence {
                ahead: 0,
                behind: 0,
                common_ancestor: current.head,
            });
        }

        // Create revision repository to access revision metadata
        // Use a unique node ID for this branch operation (for HLC generation)
        let node_id = format!("branch-{}-{}", tenant_id, repo_id);
        let rev_repo = RevisionRepositoryImpl::new(self.db.clone(), node_id);

        // Build complete ancestry set for current branch using BFS
        // This follows BOTH parent and merge_parent to handle merge commits correctly
        let mut current_revisions = HashSet::new();
        let mut current_chain = Vec::new();
        let mut queue = VecDeque::new();
        queue.push_back(current.head);
        current_revisions.insert(current.head);

        while let Some(revision) = queue.pop_front() {
            current_chain.push(revision);

            // Get metadata for this revision
            match rev_repo
                .get_revision_meta(tenant_id, repo_id, &revision)
                .await?
            {
                Some(meta) => {
                    // Follow primary parent
                    if let Some(parent) = meta.parent {
                        if current_revisions.insert(parent) {
                            queue.push_back(parent);
                        }
                    }

                    // IMPORTANT: Also follow merge_parent for merge commits
                    // This ensures we see all history that was merged in
                    if let Some(merge_parent) = meta.merge_parent {
                        if current_revisions.insert(merge_parent) {
                            queue.push_back(merge_parent);
                        }
                    }
                }
                None => {
                    tracing::warn!(
                        "Revision {} not found while traversing {}",
                        revision,
                        current_branch
                    );
                }
            }
        }

        // Build complete ancestry set for base branch
        // Follows both parent and merge_parent to capture all reachable revisions
        let mut base_revisions = HashSet::new();
        queue.clear();
        queue.push_back(base.head);
        base_revisions.insert(base.head);

        while let Some(revision) = queue.pop_front() {
            // Get metadata for this revision
            match rev_repo
                .get_revision_meta(tenant_id, repo_id, &revision)
                .await?
            {
                Some(meta) => {
                    // Follow primary parent
                    if let Some(parent) = meta.parent {
                        if base_revisions.insert(parent) {
                            queue.push_back(parent);
                        }
                    }

                    // Follow merge_parent for merge commits
                    if let Some(merge_parent) = meta.merge_parent {
                        if base_revisions.insert(merge_parent) {
                            queue.push_back(merge_parent);
                        }
                    }
                }
                None => {
                    tracing::warn!(
                        "Revision {} not found while traversing {}",
                        revision,
                        base_branch
                    );
                }
            }
        }

        // Special case: if current HEAD is in base's ancestry, current was merged into base
        // This means the branches are in sync (all of current's content is in base)
        if base_revisions.contains(&current.head) {
            return Ok(BranchDivergence {
                ahead: 0,
                behind: 0,
                common_ancestor: current.head,
            });
        }

        // Find the MOST RECENT common ancestor using HLC timestamp ordering
        // This is critical for bidirectional merges where multiple common ancestors exist
        // Using max() instead of find() ensures we get the most recent, not the first in BFS order
        let common_ancestor_hlc = current_chain
            .iter()
            .filter(|r| base_revisions.contains(r))
            .max() // HLC implements Ord (timestamp-first) - higher = more recent
            .cloned()
            .unwrap_or_else(|| HLC::new(0, 0));

        // Calculate divergence using proper set difference
        // ahead = commits reachable from current HEAD but NOT from base HEAD
        // behind = commits reachable from base HEAD but NOT from current HEAD
        let ahead = current_revisions.difference(&base_revisions).count() as u64;

        let behind = base_revisions.difference(&current_revisions).count() as u64;

        Ok(BranchDivergence {
            ahead,
            behind,
            common_ancestor: common_ancestor_hlc,
        })
    }

    /// Check if a fast-forward merge is possible from source to target branch
    ///
    /// A fast-forward merge is possible when the target branch is a direct ancestor of the source branch,
    /// meaning the source branch contains all commits from the target branch plus additional commits.
    /// In Git terms, this means the target is "behind" by N commits and "ahead" by 0 commits.
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `target_branch` - The branch to merge into (will be moved forward)
    /// * `source_branch` - The branch to merge from (provides new commits)
    ///
    /// # Returns
    /// `true` if fast-forward is possible (target is ancestor of source), `false` otherwise
    ///
    /// # Errors
    /// Returns error if either branch doesn't exist or if divergence calculation fails
    pub async fn can_fast_forward(
        &self,
        tenant_id: &str,
        repo_id: &str,
        target_branch: &str,
        source_branch: &str,
    ) -> Result<bool> {
        // Calculate divergence: source is current, target is base
        // For fast-forward: target must be behind source, and source must not be behind target
        let divergence = self
            .calculate_divergence(tenant_id, repo_id, source_branch, target_branch)
            .await?;

        // Fast-forward is possible when:
        // - behind = 0 (source branch has no commits that target doesn't have)
        // - ahead > 0 (source branch has commits that target doesn't have)
        // This means target is a direct ancestor of source
        Ok(divergence.behind == 0 && divergence.ahead > 0)
    }
}
