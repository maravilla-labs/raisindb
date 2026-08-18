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

//! Branch operation callbacks for function execution.
//!
//! These callbacks implement the `raisin.branches.*` API available to
//! server-side functions: per-node branch diffs and ahead/behind divergence,
//! both scoped to the executing function's tenant and repository.

use std::sync::Arc;

use raisin_error::Error;
use raisin_storage::{BranchRepository, NodeRepository, Storage};

use crate::api::{BranchCompareCallback, BranchCopyNodesCallback, BranchDiffCallback};

/// Create the branch_diff callback: `raisin.branches.diff(branch, baseBranch)`.
///
/// Computes the per-node diff of `branch` relative to `baseBranch`'s
/// merge-base via the storage-level `BranchRepository::diff_branches` —
/// exactly which nodes were added / modified / deleted since the branches
/// diverged (cost is O(commits since fork), not O(total nodes)).
pub fn create_branch_diff<S>(
    storage: Arc<S>,
    tenant_id: String,
    repo_id: String,
) -> BranchDiffCallback
where
    S: Storage + 'static,
{
    Arc::new(move |branch: String, base_branch: String| {
        let storage = storage.clone();
        let tenant_id = tenant_id.clone();
        let repo_id = repo_id.clone();

        Box::pin(async move {
            let diff = storage
                .branches()
                .diff_branches(&tenant_id, &repo_id, &branch, &base_branch)
                .await?;

            serde_json::to_value(&diff)
                .map_err(|e| Error::Internal(format!("Failed to serialize branch diff: {}", e)))
        })
    })
}

/// Create the branch_compare callback: `raisin.branches.compare(branch, baseBranch)`.
///
/// Calculates branch divergence (ahead/behind counts and the common
/// ancestor revision) via `BranchRepository::calculate_divergence`.
pub fn create_branch_compare<S>(
    storage: Arc<S>,
    tenant_id: String,
    repo_id: String,
) -> BranchCompareCallback
where
    S: Storage + 'static,
{
    Arc::new(move |branch: String, base_branch: String| {
        let storage = storage.clone();
        let tenant_id = tenant_id.clone();
        let repo_id = repo_id.clone();

        Box::pin(async move {
            let divergence = storage
                .branches()
                .calculate_divergence(&tenant_id, &repo_id, &branch, &base_branch)
                .await?;

            serde_json::to_value(&divergence).map_err(|e| {
                Error::Internal(format!("Failed to serialize branch divergence: {}", e))
            })
        })
    })
}

/// Create the branch_copy_nodes callback:
/// `raisin.branches.copyNodes(sourceBranch, targetBranch, opts)`.
///
/// Copies a node set from `sourceBranch` onto `targetBranch` (node ids
/// preserved, one atomic commit) via the storage-level
/// `NodeRepository::copy_nodes_across_branches`, scoped to the executing
/// tenant + repository. `opts`:
/// `{ workspace: string, roots: string[], recursive?: bool (default true),
///    deleteMissing?: bool (default false) }`.
pub fn create_branch_copy_nodes<S>(
    storage: Arc<S>,
    tenant_id: String,
    repo_id: String,
) -> BranchCopyNodesCallback
where
    S: Storage + 'static,
{
    Arc::new(
        move |source_branch: String, target_branch: String, opts: serde_json::Value| {
            let storage = storage.clone();
            let tenant_id = tenant_id.clone();
            let repo_id = repo_id.clone();

            Box::pin(async move {
                let workspace = opts
                    .get("workspace")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        Error::Validation("copyNodes: 'workspace' is required".to_string())
                    })?
                    .to_string();

                let roots: Vec<String> = opts
                    .get("roots")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                if roots.is_empty() {
                    return Err(Error::Validation(
                        "copyNodes: 'roots' must be a non-empty array of node paths".to_string(),
                    ));
                }

                let recursive = opts
                    .get("recursive")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let delete_missing = opts
                    .get("deleteMissing")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                // Optional point-in-time pin. A promotion of a large set is not
                // instantaneous; without this the copy reads each node when its
                // turn comes, so a writer working in parallel lands partly
                // inside the result. Accepts the HLC string form a revision is
                // reported as; an unparseable value is a caller error rather
                // than a silent fall back to "latest", because silently
                // publishing newer data is exactly what the pin exists to
                // prevent.
                let source_revision = match opts.get("sourceRevision").and_then(|v| v.as_str()) {
                    Some(s) if !s.is_empty() => Some(s.parse::<raisin_hlc::HLC>().map_err(|e| {
                        Error::Validation(format!("copyNodes: invalid 'sourceRevision' {s:?}: {e}"))
                    })?),
                    _ => None,
                };

                let summary = storage
                    .nodes()
                    .copy_nodes_across_branches(
                        &tenant_id,
                        &repo_id,
                        &source_branch,
                        &target_branch,
                        &workspace,
                        &roots,
                        recursive,
                        delete_missing,
                        source_revision.as_ref(),
                        None,
                    )
                    .await?;

                serde_json::to_value(&summary).map_err(|e| {
                    Error::Internal(format!("Failed to serialize copy summary: {}", e))
                })
            })
        },
    )
}
