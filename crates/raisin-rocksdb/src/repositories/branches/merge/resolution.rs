//! Merge conflict resolution.
//!
//! Handles applying user-provided conflict resolutions after a merge
//! has detected conflicts, creating the final merge commit.

use raisin_context::MergeResult;
use raisin_error::Result;
use raisin_storage::{BranchRepository, RevisionMeta, RevisionRepository};
use std::collections::{HashMap, HashSet, VecDeque};

use super::super::BranchRepositoryImpl;

impl BranchRepositoryImpl {
    /// Complete a merge by applying user-provided conflict resolutions
    ///
    /// This method is called after a `merge_branches` operation has detected conflicts.
    /// It accepts user resolutions for each conflict and creates a merge commit with
    /// the resolved state.
    ///
    /// # Algorithm
    /// 1. Verify both branches exist
    /// 2. Calculate divergence to find common ancestor
    /// 3. Allocate new revision for merge commit
    /// 4. Apply resolved properties to each conflicted node
    /// 5. Create merge commit with both parents (parent and merge_parent)
    /// 6. Update target branch HEAD
    /// 7. Copy non-conflicted changes from source branch
    ///
    /// # Arguments
    /// * `tenant_id` - Tenant identifier
    /// * `repo_id` - Repository identifier
    /// * `target_branch` - Branch to merge into (will be updated)
    /// * `source_branch` - Branch being merged from (remains unchanged)
    /// * `resolutions` - User's resolution for each conflicted node
    /// * `message` - Commit message for the merge
    /// * `actor` - User or system performing the merge
    ///
    /// # Returns
    /// `MergeResult` containing the merge commit revision and statistics
    ///
    /// # Errors
    /// Returns error if:
    /// - Either branch doesn't exist
    /// - Revision allocation fails
    /// - Node updates fail
    /// - Branch update fails
    /// - Database operations fail
    pub async fn resolve_merge_with_resolutions(
        &self,
        tenant_id: &str,
        repo_id: &str,
        target_branch: &str,
        source_branch: &str,
        resolutions: Vec<raisin_context::ConflictResolution>,
        message: &str,
        actor: &str,
    ) -> Result<MergeResult> {
        use crate::repositories::revisions::RevisionRepositoryImpl;
        use raisin_context::ResolutionType;

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

        let node_id = format!("branch-{}-{}", tenant_id, repo_id);
        let rev_repo = RevisionRepositoryImpl::new(self.db.clone(), node_id);

        // Calculate divergence to get common ancestor
        let divergence = self
            .calculate_divergence(tenant_id, repo_id, source_branch, target_branch)
            .await?;

        // Collect change information from both branches to get workspace and operation details
        // This is similar to what find_merge_conflicts does, but we need the workspace info
        // Use BFS to follow BOTH parent AND merge_parent (handles merge commits correctly)

        // Collect all node changes from common ancestor to source HEAD
        let source_changes = self
            .collect_branch_changes(&rev_repo, tenant_id, repo_id, source.head, &divergence)
            .await?;

        // Collect all node changes from common ancestor to target HEAD
        let target_changes = self
            .collect_branch_changes(&rev_repo, tenant_id, repo_id, target.head, &divergence)
            .await?;

        // Allocate new revision for merge commit
        let merge_revision = rev_repo.allocate_revision();

        // Collect all nodes that were changed (including resolved conflicts)
        let mut changed_nodes = Vec::new();

        for resolution in &resolutions {
            // Validate that this node was actually in conflict
            let source_change = source_changes.get(&resolution.node_id);
            let target_change = target_changes.get(&resolution.node_id);

            // Both changes must exist if we have a conflict resolution
            let (_, source_op) = source_change.ok_or_else(|| {
                raisin_error::Error::Validation(format!(
                    "Resolution provided for node {} which has no source change",
                    resolution.node_id
                ))
            })?;
            let (target_workspace, target_op) = target_change.ok_or_else(|| {
                raisin_error::Error::Validation(format!(
                    "Resolution provided for node {} which has no target change",
                    resolution.node_id
                ))
            })?;

            // Use target workspace (where we're merging into)
            let workspace = target_workspace.clone();

            // Determine final operation based on source/target operations and resolution choice
            let operation = match resolution.resolution_type {
                ResolutionType::KeepOurs => {
                    // Keep target version
                    *target_op
                }
                ResolutionType::KeepTheirs => {
                    // Keep source version
                    *source_op
                }
                ResolutionType::Manual => {
                    // Manual resolution - determine operation from resolved properties
                    if resolution.resolved_properties.is_null() {
                        raisin_models::tree::ChangeOperation::Deleted
                    } else {
                        // If either side was Added, keep it as Added, otherwise Modified
                        match (source_op, target_op) {
                            (raisin_models::tree::ChangeOperation::Added, _)
                            | (_, raisin_models::tree::ChangeOperation::Added) => {
                                raisin_models::tree::ChangeOperation::Added
                            }
                            _ => raisin_models::tree::ChangeOperation::Modified,
                        }
                    }
                }
            };

            changed_nodes.push(raisin_storage::NodeChangeInfo {
                node_id: resolution.node_id.clone(),
                workspace: workspace.clone(),
                operation,
                translation_locale: resolution.translation_locale.clone(),
            });

            // WRITE THE CHOSEN SIDE. Recording the resolution in the merge
            // commit's `changed_nodes` is bookkeeping, not an outcome: the
            // node blob and every index entry still hold whatever the two
            // branches left behind, and `copy_branch_indexes` below then
            // replays the SOURCE side into the target at its original
            // revision. So without this write the newest revision simply won,
            // which made `keep-ours` a no-op whenever the source edit was
            // later and `keep-theirs` a no-op whenever the target edit was.
            //
            // The merge revision is allocated fresh above, so it is newer than
            // either HEAD and newer than anything the copy brings over —
            // writing the resolved value there shadows both sides whichever
            // way the resolution went.
            //
            // The bound is each side's own HEAD: reading the source at
            // `source.head` and the target at `target.head` is what makes
            // "ours" and "theirs" mean the two branch tips a user was shown in
            // the conflict, not whatever happens to sit newest in the keyspace.
            match resolution.resolution_type {
                ResolutionType::KeepOurs => {
                    match super::apply::load_node_at(
                        &self.db,
                        tenant_id,
                        repo_id,
                        target_branch,
                        &workspace,
                        &resolution.node_id,
                        &target.head,
                    )? {
                        Some(node) => super::apply::write_resolved_node(
                            &self.db,
                            tenant_id,
                            repo_id,
                            target_branch,
                            &workspace,
                            &node,
                            &merge_revision,
                        )?,
                        // "Keep ours" over a node the target deleted means the
                        // deletion is the thing being kept, and the source's
                        // copied revision must not resurrect it.
                        None => super::apply::write_resolved_deletion(
                            &self.db,
                            tenant_id,
                            repo_id,
                            target_branch,
                            &workspace,
                            &resolution.node_id,
                            &merge_revision,
                        )?,
                    }
                }
                ResolutionType::KeepTheirs => {
                    match super::apply::load_node_at(
                        &self.db,
                        tenant_id,
                        repo_id,
                        source_branch,
                        &workspace,
                        &resolution.node_id,
                        &source.head,
                    )? {
                        Some(node) => super::apply::write_resolved_node(
                            &self.db,
                            tenant_id,
                            repo_id,
                            target_branch,
                            &workspace,
                            &node,
                            &merge_revision,
                        )?,
                        None => super::apply::write_resolved_deletion(
                            &self.db,
                            tenant_id,
                            repo_id,
                            target_branch,
                            &workspace,
                            &resolution.node_id,
                            &merge_revision,
                        )?,
                    }
                }
                ResolutionType::Manual => {
                    // A null payload is DELETE, which is how SQL's
                    // `RESOLVE ... DELETE` reaches here. Anything else is a
                    // property map to lay over the target's node — a partial
                    // map merges, so a caller may send only the fields under
                    // dispute.
                    if resolution.resolved_properties.is_null() {
                        super::apply::write_resolved_deletion(
                            &self.db,
                            tenant_id,
                            repo_id,
                            target_branch,
                            &workspace,
                            &resolution.node_id,
                            &merge_revision,
                        )?;
                    } else {
                        // Deserialize straight into `PropertyValue`, the same
                        // serde route every wire payload takes: arrays and
                        // objects stay structured rather than being flattened
                        // to strings, and a `null` stays `Null`.
                        let overrides: std::collections::HashMap<
                            String,
                            raisin_models::nodes::properties::PropertyValue,
                        > = serde_json::from_value(resolution.resolved_properties.clone())
                            .map_err(|e| {
                                raisin_error::Error::Validation(format!(
                                    "Invalid properties for node {}: {}",
                                    resolution.node_id, e
                                ))
                            })?;

                        // Base on the target's node when it still exists, and
                        // fall back to the source's when the conflict is
                        // "added on both sides" or the target deleted it.
                        let base = super::apply::load_node_at(
                            &self.db,
                            tenant_id,
                            repo_id,
                            target_branch,
                            &workspace,
                            &resolution.node_id,
                            &target.head,
                        )?;
                        let base = match base {
                            Some(node) => Some(node),
                            None => super::apply::load_node_at(
                                &self.db,
                                tenant_id,
                                repo_id,
                                source_branch,
                                &workspace,
                                &resolution.node_id,
                                &source.head,
                            )?,
                        };
                        let mut node = base.ok_or_else(|| {
                            raisin_error::Error::NotFound(format!(
                                "Node {} not found on either side of the merge",
                                resolution.node_id
                            ))
                        })?;

                        for (key, value) in overrides {
                            node.properties.insert(key, value);
                        }

                        super::apply::write_resolved_node(
                            &self.db,
                            tenant_id,
                            repo_id,
                            target_branch,
                            &workspace,
                            &node,
                            &merge_revision,
                        )?;
                    }
                }
            }
        }

        // Collect all changed nodes from source branch since common ancestor
        let mut revision = source.head;
        while revision != divergence.common_ancestor {
            let meta = rev_repo
                .get_revision_meta(tenant_id, repo_id, &revision)
                .await?
                .ok_or_else(|| {
                    raisin_error::Error::NotFound(format!("Revision {} not found", revision))
                })?;

            // Add non-conflicted nodes to changed_nodes
            for node_change in &meta.changed_nodes {
                // Only add if not already present (check by node_id)
                if !changed_nodes
                    .iter()
                    .any(|cn| cn.node_id == node_change.node_id)
                {
                    changed_nodes.push(node_change.clone());
                }
            }

            if let Some(parent) = meta.parent {
                revision = parent;
            } else {
                break;
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

        // Copy branch indexes from source to target (for non-conflicted changes)
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

    /// Collect all node changes from a branch HEAD back to the common ancestor
    ///
    /// Uses BFS to follow both parent and merge_parent links, collecting
    /// all node changes along the way.
    async fn collect_branch_changes(
        &self,
        rev_repo: &crate::repositories::revisions::RevisionRepositoryImpl,
        tenant_id: &str,
        repo_id: &str,
        head: raisin_hlc::HLC,
        divergence: &raisin_context::BranchDivergence,
    ) -> Result<HashMap<String, (String, raisin_models::tree::ChangeOperation)>> {
        let mut changes: HashMap<String, (String, raisin_models::tree::ChangeOperation)> =
            HashMap::new();
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(head);
        visited.insert(head);

        while let Some(revision) = queue.pop_front() {
            if revision == divergence.common_ancestor {
                continue;
            }

            let Some(meta) = rev_repo
                .get_revision_meta(tenant_id, repo_id, &revision)
                .await?
            else {
                continue;
            };

            for change in &meta.changed_nodes {
                changes
                    .entry(change.node_id.clone())
                    .or_insert((change.workspace.clone(), change.operation));
            }

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

        Ok(changes)
    }
}
