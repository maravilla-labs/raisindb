// SPDX-License-Identifier: BSL-1.1

//! Cluster-wide index repairs: enqueue on every node, report per node.
//!
//! A repair rewrites a derived index, and derived indexes live on each node
//! separately — nothing about them replicates. So the console asks ONE node,
//! and that node enqueues the repair for itself and forwards the same request
//! to every peer (see [`super::repair_peers`]). The answer lists every node,
//! including the ones that could not be reached, because a repair that ran on
//! two of three nodes is a cluster that still returns wrong rows on the third.
//!
//! `local_only=true` marks a forwarded request: the receiving node acts on
//! itself and never forwards again.

use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    response::Json,
};
use futures::future::join_all;
use raisin_rocksdb::management::async_indexing::repair::{
    enqueue_index_repair, load_state, RepairKind, RepairState,
};
use serde::{Deserialize, Serialize};

use super::repair_peers::{call, failure_parts, local_node_id, peer_path, peers};
use super::types::ErrorResponse;
use crate::middleware::TenantInfo;
use crate::state::AppState;

type HandlerError = (StatusCode, Json<ErrorResponse>);

/// Request body for starting a repair.
#[derive(Debug, Serialize, Deserialize)]
pub struct RepairRequest {
    /// One branch, or every branch of the repository when omitted.
    #[serde(default)]
    pub branch: Option<String>,
    /// Count what would be written without writing it.
    #[serde(default)]
    pub dry_run: bool,
}

/// Query parameters shared by both repair routes.
#[derive(Debug, Default, Deserialize)]
pub struct RepairQuery {
    /// Set on a forwarded request: act on this node only.
    #[serde(default)]
    pub local_only: bool,
    /// Status only: report one branch instead of every branch.
    #[serde(default)]
    pub branch: Option<String>,
}

/// One node's answer to "enqueue this repair".
#[derive(Debug, Serialize, Deserialize)]
pub struct NodeEnqueue {
    pub node_id: String,
    /// `enqueued`, `unreachable` or `error`.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RepairEnqueueResponse {
    pub repair: String,
    pub dry_run: bool,
    pub branch: Option<String>,
    pub nodes: Vec<NodeEnqueue>,
}

/// A branch's persisted repair state on one node; `None` means never started.
#[derive(Debug, Serialize, Deserialize)]
pub struct BranchRepairState {
    pub branch: String,
    pub state: Option<RepairState>,
}

/// One node's repair state for every branch asked about.
#[derive(Debug, Serialize, Deserialize)]
pub struct NodeRepairStatus {
    pub node_id: String,
    /// `reported`, `unreachable` or `error`.
    pub status: String,
    #[serde(default)]
    pub branches: Vec<BranchRepairState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RepairStatusResponse {
    pub repair: String,
    pub nodes: Vec<NodeRepairStatus>,
}

fn bad_request(error: String) -> HandlerError {
    (StatusCode::BAD_REQUEST, Json(ErrorResponse { error }))
}

fn internal(error: String) -> HandlerError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse { error }),
    )
}

fn rocksdb(state: &AppState) -> Result<&raisin_rocksdb::RocksDBStorage, HandlerError> {
    state
        .rocksdb_storage
        .as_deref()
        .ok_or_else(|| internal("RocksDB storage not initialized".to_string()))
}

fn kind(repair: &str) -> Result<RepairKind, HandlerError> {
    RepairKind::from_slug(repair).ok_or_else(|| {
        let expected: Vec<&str> = RepairKind::ALL.iter().map(RepairKind::slug).collect();
        bad_request(format!(
            "unknown repair '{repair}' (expected one of: {})",
            expected.join(", ")
        ))
    })
}

/// Start a repair on every node of the cluster.
///
/// POST /api/management/{repo}/repairs/{repair}
pub async fn enqueue_repair(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantInfo>,
    Path((repo, repair)): Path<(String, String)>,
    Query(query): Query<RepairQuery>,
    Json(req): Json<RepairRequest>,
) -> Result<Json<RepairEnqueueResponse>, HandlerError> {
    let kind = kind(&repair)?;
    let storage = rocksdb(&state)?;
    let tenant_id = tenant.tenant_id.as_str();

    tracing::warn!(
        tenant = %tenant_id, repo = %repo, repair = %repair, branch = ?req.branch,
        dry_run = req.dry_run, local_only = query.local_only,
        "Enqueueing index repair (admin action)"
    );

    let local = match enqueue_index_repair(
        storage,
        tenant_id,
        &repo,
        req.branch.as_deref(),
        kind,
        req.dry_run,
    )
    .await
    {
        Ok(job_id) => NodeEnqueue {
            node_id: local_node_id(storage),
            status: "enqueued".into(),
            job_id: Some(job_id),
            error: None,
        },
        Err(e) => NodeEnqueue {
            node_id: local_node_id(storage),
            status: "error".into(),
            job_id: None,
            error: Some(e.to_string()),
        },
    };
    let mut nodes = vec![local];

    if !query.local_only {
        let body = serde_json::to_value(&req).map_err(|e| internal(e.to_string()))?;
        let path = peer_path(&repo, &repair, "", None);
        let peers = peers(storage);
        let answers = join_all(peers.iter().map(|peer| {
            call::<RepairEnqueueResponse>(
                peer,
                reqwest::Method::POST,
                &path,
                tenant_id,
                Some(&body),
            )
        }))
        .await;
        for (peer, answer) in peers.iter().zip(answers) {
            match answer {
                Ok(resp) if !resp.nodes.is_empty() => nodes.extend(resp.nodes),
                Ok(_) => nodes.push(NodeEnqueue {
                    node_id: peer.node_id.clone(),
                    status: "error".into(),
                    job_id: None,
                    error: Some("peer reported no node".into()),
                }),
                Err(failure) => {
                    let (node_id, status, error) = failure_parts(peer, failure);
                    nodes.push(NodeEnqueue {
                        node_id,
                        status,
                        job_id: None,
                        error,
                    });
                }
            }
        }
    }

    Ok(Json(RepairEnqueueResponse {
        repair,
        dry_run: req.dry_run,
        branch: req.branch,
        nodes,
    }))
}

/// Report every node's persisted state for a repair.
///
/// GET /api/management/{repo}/repairs/{repair}/status
pub async fn repair_status(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantInfo>,
    Path((repo, repair)): Path<(String, String)>,
    Query(query): Query<RepairQuery>,
) -> Result<Json<RepairStatusResponse>, HandlerError> {
    use raisin_storage::{BranchRepository, Storage};

    let kind = kind(&repair)?;
    let storage = rocksdb(&state)?;
    let tenant_id = tenant.tenant_id.as_str();
    let node_id = local_node_id(storage);

    let branches = match &query.branch {
        Some(branch) => vec![branch.clone()],
        None => storage
            .branches()
            .list_branches(tenant_id, &repo)
            .await
            .map_err(|e| internal(e.to_string()))?
            .into_iter()
            .map(|b| b.name)
            .collect(),
    };
    let mut local = NodeRepairStatus {
        node_id: node_id.clone(),
        status: "reported".into(),
        branches: Vec::with_capacity(branches.len()),
        error: None,
    };
    for branch in branches {
        match load_state(
            storage.db(),
            tenant_id,
            &repo,
            &branch,
            kind.slug(),
            &node_id,
        ) {
            Ok(state) => local.branches.push(BranchRepairState { branch, state }),
            Err(e) => {
                local.status = "error".into();
                local.error = Some(format!("branch {branch}: {e}"));
                break;
            }
        }
    }
    let mut nodes = vec![local];

    if !query.local_only {
        let path = peer_path(&repo, &repair, "/status", query.branch.as_deref());
        let peers = peers(storage);
        let answers = join_all(peers.iter().map(|peer| {
            call::<RepairStatusResponse>(peer, reqwest::Method::GET, &path, tenant_id, None)
        }))
        .await;
        for (peer, answer) in peers.iter().zip(answers) {
            match answer {
                Ok(resp) => nodes.extend(resp.nodes),
                Err(failure) => {
                    let (node_id, status, error) = failure_parts(peer, failure);
                    nodes.push(NodeRepairStatus {
                        node_id,
                        status,
                        branches: Vec::new(),
                        error,
                    });
                }
            }
        }
    }

    Ok(Json(RepairStatusResponse { repair, nodes }))
}
