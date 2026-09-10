// SPDX-License-Identifier: BSL-1.1

//! PATH_INDEX reconstruction handler.
//!
//! Repairs the forward path index from NODE_PATH, which is the source of truth
//! and is not cleared by any rebuild. See
//! `raisin_rocksdb::management::async_indexing::path_repair` for why this is a
//! separate operation from `reindex` and why it is write-only.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

use super::types::{get_branch_name, DatabaseOpQuery, ErrorResponse};

/// Request body for a path-index repair.
#[derive(Debug, Deserialize)]
pub struct PathRepairRequest {
    /// Workspace to repair. Omit (or null) to repair every workspace of the
    /// branch.
    #[serde(default)]
    pub workspace: Option<String>,

    /// Report what would be written without writing anything.
    #[serde(default)]
    pub dry_run: bool,
}

/// Per-workspace counts plus the totals, so an operator can verify the repair
/// did the whole job rather than infer it from a query that started passing.
#[derive(Debug, Serialize)]
pub struct PathRepairResponse {
    pub tenant: String,
    pub repo: String,
    pub branch: String,
    pub dry_run: bool,
    pub workspaces: Vec<raisin_rocksdb::management::async_indexing::PathIndexRepairStats>,
    pub total_entries_written: usize,
    pub total_nodes_seen: usize,
    /// NOT ZERO IS A PROBLEM: each one is a node that stays unreachable by path.
    pub total_skipped_unreadable: usize,
}

/// Rebuild PATH_INDEX from NODE_PATH.
///
/// POST /api/admin/management/database/:tenant/:repo/path-index/repair
///
/// Synchronous, unlike `reindex/start`: this reads one index and writes
/// another, decoding no nodes and consulting no schema, so it is a prefix scan
/// rather than a rebuild. Running it in the foreground means the operator gets
/// the counts back with the request instead of polling a job — which matters,
/// because this is run while something is broken.
#[cfg(feature = "storage-rocksdb")]
pub async fn repair_path_index(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(params): Query<DatabaseOpQuery>,
    Json(req): Json<PathRepairRequest>,
) -> Result<Json<PathRepairResponse>, (StatusCode, Json<ErrorResponse>)> {
    use raisin_rocksdb::management::async_indexing;

    let Some(storage) = &state.rocksdb_storage else {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "RocksDB storage not initialized".to_string(),
            }),
        ));
    };

    let branch = get_branch_name(&state, &tenant, &repo, params.branch).await?;

    tracing::warn!(
        tenant = %tenant,
        repo = %repo,
        branch = %branch,
        workspace = ?req.workspace,
        dry_run = req.dry_run,
        "Repairing PATH_INDEX from NODE_PATH (admin action)"
    );

    let result = match &req.workspace {
        Some(workspace) => async_indexing::repair_path_index(
            storage,
            &tenant,
            &repo,
            &branch,
            workspace,
            req.dry_run,
        )
        .await
        .map(|s| vec![s]),
        None => {
            async_indexing::repair_path_index_all_workspaces(
                storage,
                &tenant,
                &repo,
                &branch,
                req.dry_run,
            )
            .await
        }
    };

    let workspaces = result.map_err(|e| {
        tracing::error!(error = %e, "PATH_INDEX repair failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Path index repair failed: {}", e),
            }),
        )
    })?;

    Ok(Json(PathRepairResponse {
        tenant,
        repo,
        branch,
        dry_run: req.dry_run,
        total_entries_written: workspaces.iter().map(|s| s.entries_written).sum(),
        total_nodes_seen: workspaces.iter().map(|s| s.nodes_seen).sum(),
        total_skipped_unreadable: workspaces.iter().map(|s| s.skipped_unreadable).sum(),
        workspaces,
    }))
}
