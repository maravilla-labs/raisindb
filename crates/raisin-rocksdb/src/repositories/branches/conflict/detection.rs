//! Three-way merge conflict detection algorithm.

use crate::{cf, cf_handle, keys};
use raisin_context::{ConflictType, MergeConflict};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_storage::RevisionRepository;
use std::collections::{HashMap, HashSet, VecDeque};

use super::super::BranchRepositoryImpl;

impl BranchRepositoryImpl {
    /// Find merge conflicts between two branches
    ///
    /// Performs a three-way merge analysis to identify nodes that were modified in both branches
    /// since their common ancestor. This implements Git-like conflict detection by comparing
    /// changes in both branches against their merge base.
    ///
    /// # Algorithm
    /// 1. Calculate divergence to find the common ancestor revision
    /// 2. Build complete revision sets for each branch (BFS traversal)
    /// 3. Collect node changes exclusive to each branch
    /// 4. Find (node_id, locale) pairs modified in both branches
    /// 5. For each conflict, retrieve state at base, target, and source
    pub async fn find_merge_conflicts(
        &self,
        tenant_id: &str,
        repo_id: &str,
        target_branch: &str,
        source_branch: &str,
    ) -> Result<Vec<MergeConflict>> {
        use crate::repositories::revisions::RevisionRepositoryImpl;
        use raisin_storage::BranchRepository;

        // Calculate divergence to get common ancestor
        let divergence = self
            .calculate_divergence(tenant_id, repo_id, source_branch, target_branch)
            .await?;

        let common_ancestor = divergence.common_ancestor;

        // If branches are identical or one is ancestor of the other, no conflicts
        if divergence.ahead == 0 || divergence.behind == 0 {
            return Ok(Vec::new());
        }

        let node_id = format!("branch-{}-{}", tenant_id, repo_id);
        let rev_repo = RevisionRepositoryImpl::new(self.db.clone(), node_id);

        // Get both branches to access their HEAD revisions
        let target = self
            .get_branch(tenant_id, repo_id, target_branch)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Branch '{}' not found", target_branch))
            })?;

        let source = self
            .get_branch(tenant_id, repo_id, source_branch)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!("Branch '{}' not found", source_branch))
            })?;

        // STEP 1: Build complete revision sets for both branches
        let source_revisions =
            Self::walk_revision_history(&rev_repo, tenant_id, repo_id, source.head).await?;
        let target_revisions =
            Self::walk_revision_history(&rev_repo, tenant_id, repo_id, target.head).await?;

        // STEP 2: Collect changes exclusive to each branch
        let source_changes = Self::collect_exclusive_changes(
            &rev_repo,
            tenant_id,
            repo_id,
            &source_revisions,
            &target_revisions,
        )
        .await?;

        let target_changes = Self::collect_exclusive_changes(
            &rev_repo,
            tenant_id,
            repo_id,
            &target_revisions,
            &source_revisions,
        )
        .await?;

        // STEP 3: Find conflicting (node_id, locale) pairs and build conflict objects
        let cf_nodes = cf_handle(&self.db, cf::NODES)?;
        let cf_translation_data = cf_handle(&self.db, cf::TRANSLATION_DATA)?;

        let mut conflicts = Vec::new();

        for ((node_id, translation_locale), (source_workspace, source_op, _source_rev)) in
            &source_changes
        {
            let key = (node_id.clone(), translation_locale.clone());
            if let Some((target_workspace, target_op, _target_rev)) = target_changes.get(&key) {
                let conflict_type = Self::determine_conflict_type(source_op, target_op);

                let (base_properties, target_properties, source_properties) = self
                    .retrieve_conflict_properties(
                        tenant_id,
                        repo_id,
                        target_branch,
                        source_branch,
                        target_workspace,
                        source_workspace,
                        node_id,
                        translation_locale.as_deref(),
                        &common_ancestor,
                        &target.head,
                        &source.head,
                        cf_nodes,
                        cf_translation_data,
                    )
                    .await?;

                let path = self
                    .resolve_conflict_path(
                        tenant_id,
                        repo_id,
                        target_branch,
                        target_workspace,
                        node_id,
                        translation_locale.as_ref(),
                        &target.head,
                        &target_properties,
                        &source_properties,
                        cf_nodes,
                    )
                    .await?;

                conflicts.push(MergeConflict {
                    node_id: node_id.clone(),
                    path,
                    conflict_type,
                    base_properties,
                    target_properties,
                    source_properties,
                    translation_locale: translation_locale.clone(),
                });
            }
        }

        Ok(conflicts)
    }

    /// Walk revision history (BFS, following both parent and merge_parent)
    async fn walk_revision_history(
        rev_repo: &crate::repositories::revisions::RevisionRepositoryImpl,
        tenant_id: &str,
        repo_id: &str,
        head: HLC,
    ) -> Result<HashSet<HLC>> {
        let mut revisions = HashSet::new();
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(head);

        while let Some(rev) = queue.pop_front() {
            if !visited.insert(rev) {
                continue;
            }
            revisions.insert(rev);
            if let Some(meta) = rev_repo.get_revision_meta(tenant_id, repo_id, &rev).await? {
                if let Some(p) = meta.parent {
                    queue.push_back(p);
                }
                if let Some(mp) = meta.merge_parent {
                    queue.push_back(mp);
                }
            }
        }

        Ok(revisions)
    }

    /// Collect node changes from revisions exclusive to `branch_revisions` (not in `other_revisions`)
    ///
    /// Skips merge commits that brought in changes from the other branch.
    async fn collect_exclusive_changes(
        rev_repo: &crate::repositories::revisions::RevisionRepositoryImpl,
        tenant_id: &str,
        repo_id: &str,
        branch_revisions: &HashSet<HLC>,
        other_revisions: &HashSet<HLC>,
    ) -> Result<
        HashMap<(String, Option<String>), (String, raisin_models::tree::ChangeOperation, HLC)>,
    > {
        let mut changes = HashMap::new();

        for &revision in branch_revisions.difference(other_revisions) {
            let Some(meta) = rev_repo
                .get_revision_meta(tenant_id, repo_id, &revision)
                .await?
            else {
                continue;
            };

            // Skip merge commits that brought in the other branch's changes
            if let Some(merge_parent) = meta.merge_parent {
                if other_revisions.contains(&merge_parent) {
                    continue;
                }
            }

            for change in &meta.changed_nodes {
                let key = (change.node_id.clone(), change.translation_locale.clone());
                changes.entry(key).or_insert((
                    change.workspace.clone(),
                    change.operation,
                    revision,
                ));
            }
        }

        Ok(changes)
    }

    /// Determine conflict type based on source and target operations
    fn determine_conflict_type(
        source_op: &raisin_models::tree::ChangeOperation,
        target_op: &raisin_models::tree::ChangeOperation,
    ) -> ConflictType {
        match (source_op, target_op) {
            (
                raisin_models::tree::ChangeOperation::Deleted,
                raisin_models::tree::ChangeOperation::Modified,
            ) => ConflictType::DeletedBySourceModifiedByTarget,
            (
                raisin_models::tree::ChangeOperation::Modified,
                raisin_models::tree::ChangeOperation::Deleted,
            ) => ConflictType::ModifiedBySourceDeletedByTarget,
            (
                raisin_models::tree::ChangeOperation::Added,
                raisin_models::tree::ChangeOperation::Added,
            ) => ConflictType::BothAdded,
            _ => ConflictType::BothModified,
        }
    }
}
