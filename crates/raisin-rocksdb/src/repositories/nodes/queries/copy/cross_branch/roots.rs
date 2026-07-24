//! Validation and root resolution for cross-branch copy.

use super::super::super::super::NodeRepositoryImpl;
use super::{parent_path_of, RootContext};
use raisin_error::Result;
use raisin_storage::BranchRepository;

impl NodeRepositoryImpl {
    /// Validate the request and resolve every copy root on the source branch
    /// plus its parent on BOTH branches. Returns the (unprotected) target
    /// branch and the resolved roots. A missing target parent fails the whole
    /// operation early - never write a dangling subtree.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn resolve_cross_branch_roots(
        &self,
        tenant_id: &str,
        repo_id: &str,
        source_branch: &str,
        target_branch: &str,
        workspace: &str,
        roots: &[String],
    ) -> Result<(raisin_context::Branch, Vec<RootContext>)> {
        // ========== VALIDATION ==========
        if roots.is_empty() {
            return Err(raisin_error::Error::Validation(
                "At least one root path is required".to_string(),
            ));
        }
        if source_branch == target_branch {
            return Err(raisin_error::Error::Validation(
                "Source and target branch must differ".to_string(),
            ));
        }

        self.branch_repo
            .get_branch(tenant_id, repo_id, source_branch)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!(
                    "Source branch '{}' not found",
                    source_branch
                ))
            })?;

        let target = self
            .branch_repo
            .get_branch(tenant_id, repo_id, target_branch)
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!(
                    "Target branch '{}' not found",
                    target_branch
                ))
            })?;

        // Mirror merge_branches: never write onto a protected branch.
        if target.protected {
            return Err(raisin_error::Error::Forbidden(format!(
                "Cannot copy into protected branch '{}'",
                target_branch
            )));
        }

        // Resolve each root on the source branch and its parent on BOTH
        // branches. A missing target parent fails the whole operation early —
        // never write a dangling subtree.
        let mut root_ctxs: Vec<RootContext> = Vec::with_capacity(roots.len());
        for root_path in roots {
            let node = self
                .get_by_path_impl(
                    tenant_id,
                    repo_id,
                    source_branch,
                    workspace,
                    root_path,
                    None,
                )
                .await?
                .ok_or_else(|| {
                    raisin_error::Error::NotFound(format!(
                        "Source node '{}' not found on branch '{}'",
                        root_path, source_branch
                    ))
                })?;

            let parent_path = parent_path_of(&node.path);

            let src_parent_id = self
                .resolve_parent_id_opt(tenant_id, repo_id, source_branch, workspace, &parent_path)
                .await?
                .ok_or_else(|| {
                    raisin_error::Error::internal(format!(
                        "Source parent '{}' unresolvable on branch '{}'",
                        parent_path, source_branch
                    ))
                })?;

            let dst_parent_id = self
                .resolve_parent_id_opt(tenant_id, repo_id, target_branch, workspace, &parent_path)
                .await?
                .ok_or_else(|| {
                    raisin_error::Error::Validation(format!(
                        "Target parent '{}' does not exist on branch '{}'",
                        parent_path, target_branch
                    ))
                })?;

            root_ctxs.push(RootContext {
                node,
                src_parent_id,
                dst_parent_id,
            });
        }
        Ok((target, root_ctxs))
    }
}
