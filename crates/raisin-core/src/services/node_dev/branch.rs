// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Branches as worktrees: fork a draft, diff it against its base (bounded by
//! the working roots), merge it with conflicts reported instead of guessed,
//! or discard it. Thin, typed wrappers over the existing branch repository.

use raisin_context::{BranchDiff, MergeConflict, MergeStrategy};
use raisin_models::auth::AuthContext;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{BranchRepository, Storage};
use serde::{Deserialize, Serialize};

use super::changeset::{is_admin, owner_of};
use super::root::Roots;
use super::types::*;
use super::{DevScope, NodeDevService};

/// A branch as the surface reports it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchInfo {
    /// Name.
    pub name: String,
    /// Head revision (HLC).
    pub head: String,
    /// Forked from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
    /// Protected branches cannot be discarded.
    pub protected: bool,
}

/// One changed node in a branch diff.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffEntry {
    /// `added` | `modified` | `deleted` | `reordered`.
    pub operation: String,
    /// Workspace.
    pub workspace: String,
    /// Node id.
    pub node_id: String,
    /// Path on the diffed branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Translation locale, for a translation change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
}

/// A branch diff, filtered to the roots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchDiffResult {
    /// The diffed branch.
    pub branch: String,
    /// Its base.
    pub base: String,
    /// Merge base (HLC).
    pub common_ancestor: String,
    /// Changes inside the roots.
    pub changes: Vec<DiffEntry>,
}

/// A merge attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeOutcome {
    /// Whether it merged.
    pub merged: bool,
    /// Whether it was only checked.
    pub dry_run: bool,
    /// Conflicts (the merge is not performed when any exist).
    pub conflicts: Vec<MergeConflict>,
    /// Nodes changed by the merge.
    pub nodes_changed: usize,
    /// Fast-forward?
    pub fast_forward: bool,
}

fn info(b: raisin_context::Branch) -> BranchInfo {
    BranchInfo {
        name: b.name,
        head: b.head.to_string(),
        upstream: b.upstream_branch,
        protected: b.protected,
    }
}

fn check_name(name: &str) -> DevResult<()> {
    let ok = !name.is_empty()
        && name.len() <= 120
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/' | '.'));
    if ok {
        Ok(())
    } else {
        Err(NodeDevError::invalid(format!(
            "'{name}' is not a valid branch name"
        )))
    }
}

impl<S: Storage + TransactionalStorage> NodeDevService<S> {
    /// Fork `name` from `scope.branch`'s head (idempotent: an existing branch
    /// with the same upstream is returned).
    pub async fn fork_branch(
        &self,
        scope: &DevScope,
        auth: &AuthContext,
        name: &str,
    ) -> DevResult<BranchInfo> {
        check_name(name)?;
        let branches = self.storage.branches();
        if let Some(b) = branches
            .get_branch(&scope.tenant, &scope.repo, name)
            .await?
        {
            if b.upstream_branch.as_deref() == Some(scope.branch.as_str()) {
                return Ok(info(b));
            }
            return Err(NodeDevError::new(
                409,
                "exists",
                format!("branch '{name}' exists"),
            ));
        }
        let base = branches
            .get_branch(&scope.tenant, &scope.repo, &scope.branch)
            .await?
            .ok_or_else(|| NodeDevError::not_found(format!("branch '{}'", scope.branch)))?;
        let b = branches
            .create_branch(
                &scope.tenant,
                &scope.repo,
                name,
                &owner_of(auth),
                Some(base.head),
                Some(scope.branch.clone()),
                false,
                false,
            )
            .await?;
        Ok(info(b))
    }

    /// Diff `scope.branch` against `base`, keeping changes inside the roots.
    pub async fn diff_branch(
        &self,
        scope: &DevScope,
        roots: &Roots,
        base: &str,
    ) -> DevResult<BranchDiffResult> {
        let d: BranchDiff = self
            .storage
            .branches()
            .diff_branches(&scope.tenant, &scope.repo, &scope.branch, base)
            .await?;
        let changes = d
            .added
            .into_iter()
            .chain(d.modified)
            .chain(d.deleted)
            .filter(|n| match &n.path {
                Some(p) => roots.check(&n.workspace, p, OpKind::Read).is_ok(),
                None => roots.all().iter().any(|r| r.workspace == n.workspace),
            })
            .map(|n| DiffEntry {
                operation: n.operation,
                workspace: n.workspace,
                node_id: n.node_id,
                path: n.path,
                locale: n.translation_locale,
            })
            .collect();
        Ok(BranchDiffResult {
            branch: scope.branch.clone(),
            base: base.to_string(),
            common_ancestor: d.common_ancestor.to_string(),
            changes,
        })
    }

    /// Merge `scope.branch` into `target`. Conflicts are reported and the
    /// merge is NOT performed; `dry_run` only reports.
    pub async fn merge_branch(
        &self,
        scope: &DevScope,
        auth: &AuthContext,
        target: &str,
        message: Option<&str>,
        dry_run: bool,
    ) -> DevResult<MergeOutcome> {
        let branches = self.storage.branches();
        let into = branches
            .get_branch(&scope.tenant, &scope.repo, target)
            .await?
            .ok_or_else(|| NodeDevError::not_found(format!("branch '{target}'")))?;
        if into.protected && !is_admin(auth) {
            return Err(NodeDevError::forbidden(format!(
                "branch '{target}' is protected"
            )));
        }
        let conflicts = branches
            .find_merge_conflicts(&scope.tenant, &scope.repo, target, &scope.branch)
            .await?;
        if dry_run || !conflicts.is_empty() {
            return Ok(MergeOutcome {
                merged: false,
                dry_run,
                conflicts,
                nodes_changed: 0,
                fast_forward: false,
            });
        }
        let msg = message
            .map(str::to_string)
            .unwrap_or_else(|| format!("merge {} into {target}", scope.branch));
        let r = branches
            .merge_branches(
                &scope.tenant,
                &scope.repo,
                target,
                &scope.branch,
                MergeStrategy::ThreeWay,
                &msg,
                &owner_of(auth),
            )
            .await?;
        Ok(MergeOutcome {
            merged: r.success,
            dry_run: false,
            conflicts: r.conflicts,
            nodes_changed: r.nodes_changed,
            fast_forward: r.fast_forward,
        })
    }

    /// Discard (delete) `scope.branch`. Protected branches and a branch
    /// without an upstream (a root branch such as `main`) are refused.
    pub async fn discard_branch(&self, scope: &DevScope, auth: &AuthContext) -> DevResult<bool> {
        let branches = self.storage.branches();
        let b = branches
            .get_branch(&scope.tenant, &scope.repo, &scope.branch)
            .await?
            .ok_or_else(|| NodeDevError::not_found(format!("branch '{}'", scope.branch)))?;
        if b.created_by != owner_of(auth) && !is_admin(auth) {
            return Err(NodeDevError::forbidden(
                "only the branch's creator may discard it",
            ));
        }
        if b.protected || b.upstream_branch.is_none() {
            return Err(NodeDevError::forbidden(format!(
                "branch '{}' is protected or a root branch",
                scope.branch
            )));
        }
        Ok(branches
            .delete_branch(&scope.tenant, &scope.repo, &scope.branch)
            .await?)
    }
}
