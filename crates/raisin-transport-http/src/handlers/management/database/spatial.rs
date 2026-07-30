// SPDX-License-Identifier: BSL-1.1

//! Spatial (geohash) index management handlers.
//!
//! The HTTP mirror of the `SPATIAL INDEX` SQL admin statements, so an admin console
//! can drive the same operations without composing SQL. Both surfaces read and write
//! exactly the same records — `WorkspaceConfig.spatial` for intent, the local
//! `INDEX_STATUS` record plus a physical key census for reality — so they cannot
//! report different answers.
//!
//! # Gating
//!
//! These routes carry `require_admin_auth_middleware` explicitly at the router.
//! That is deliberate and worth stating: the sibling `fulltext/*` and `vector/*`
//! routes in the same subtree carry **no** auth layer in the main API router, so a
//! rebuild there is reachable unauthenticated. Rather than copy that, the spatial
//! routes are gated; the gap on the siblings is reported separately.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::Json,
};
use serde::{Deserialize, Serialize};

use raisin_models::nodes::properties::{
    resolve_spatial_policy, sorted_precisions, SpatialCoverMode, SpatialPropertySchema,
    SpatialWorkspaceSchema,
};
use raisin_storage::scope::RepoScope;
use raisin_storage::{Storage, WorkspaceRepository};

use crate::state::AppState;

use super::types::{get_branch_name, DatabaseOpQuery, ErrorResponse, JobResponse};

type ApiError = (StatusCode, Json<ErrorResponse>);

fn err(status: StatusCode, message: impl Into<String>) -> ApiError {
    (
        status,
        Json(ErrorResponse {
            error: message.into(),
        }),
    )
}

/// Request body naming the scope an operation applies to.
#[derive(Debug, Deserialize)]
pub struct SpatialScopeRequest {
    /// Workspace to operate on.
    pub workspace: String,
    /// Property to operate on. `None` means the workspace default / every geometry
    /// property, matching the SQL grammar's optional `PROPERTY` clause.
    #[serde(default)]
    pub property: Option<String>,
}

/// Request body for `PUT …/spatial/config`.
#[derive(Debug, Deserialize)]
pub struct SpatialConfigRequest {
    pub workspace: String,
    #[serde(default)]
    pub property: Option<String>,
    #[serde(default)]
    pub precisions: Option<Vec<usize>>,
    #[serde(default)]
    pub srid: Option<u32>,
    #[serde(default)]
    pub bucket_property: Option<String>,
    /// `"centroid"` or `"extent"`.
    #[serde(default)]
    pub cover: Option<String>,
    /// Drop the declaration for this scope instead of setting it — the HTTP
    /// equivalent of `ALTER SPATIAL INDEX … RESET`.
    #[serde(default)]
    pub reset: bool,
}

/// One resolved configuration scope.
#[derive(Debug, Serialize)]
pub struct SpatialConfigEntry {
    pub workspace: String,
    pub property: String,
    pub precisions: Vec<usize>,
    pub srid: Option<u32>,
    pub bucket_property: Option<String>,
    pub cover: String,
    pub policy_hash: u64,
    /// Id of the `SpatialIndexBuild` this change queued on THIS node, when the
    /// new policy differs from what the local index was built under.
    ///
    /// Peers reconcile themselves: `WorkspaceConfig.spatial` is replicated, and
    /// each peer's next write to the workspace notices the drift and queues its
    /// own local build.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rebuild_job_id: Option<String>,
}

/// One property's local index state plus its physical census.
#[derive(Debug, Serialize)]
pub struct SpatialHealthEntry {
    pub workspace: String,
    pub property: String,
    pub phase: String,
    pub indexed_precisions: Vec<usize>,
    pub indexed_policy_hash: u64,
    pub configured_precisions: Vec<usize>,
    pub configured_policy_hash: u64,
    pub needs_rebuild: bool,
    pub nodes_indexed: u64,
    pub live_entries: u64,
    pub tombstoned_entries: u64,
    pub distinct_nodes: u64,
    pub legacy_entries: u64,
    pub live_per_precision: Vec<(usize, u64)>,
}

#[derive(Debug, Serialize)]
pub struct SpatialVerifyEntry {
    pub workspace: String,
    pub property: String,
    pub status: String,
    pub detail: String,
    pub live_entries: u64,
    pub distinct_nodes: u64,
}

#[cfg(feature = "storage-rocksdb")]
fn storage(state: &AppState) -> Result<&std::sync::Arc<raisin_rocksdb::RocksDBStorage>, ApiError> {
    state.rocksdb_storage.as_ref().ok_or_else(|| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "RocksDB storage not initialized",
        )
    })
}

/// Read the declared spatial policy.
///
/// GET /api/admin/management/database/:tenant/:repo/spatial/config?workspace=…&property=…
#[cfg(feature = "storage-rocksdb")]
pub async fn get_spatial_config(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(scope_params): Query<SpatialScopeQuery>,
) -> Result<Json<Vec<SpatialConfigEntry>>, ApiError> {
    let storage = storage(&state)?;
    let scope = RepoScope::new(&tenant, &repo);

    let workspaces = match &scope_params.workspace {
        Some(ws) => vec![ws.clone()],
        None => storage
            .workspaces()
            .list(scope)
            .await
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .into_iter()
            .map(|w| w.name)
            .collect(),
    };

    let mut out = Vec::new();
    for ws_name in workspaces {
        let Some(ws) = storage
            .workspaces()
            .get(scope, &ws_name)
            .await
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        else {
            continue;
        };
        let schema = ws.config.spatial.clone();

        let mut scopes: Vec<Option<String>> = Vec::new();
        match &scope_params.property {
            Some(p) => scopes.push(Some(p.clone())),
            None => {
                scopes.push(None);
                if let Some(s) = &schema {
                    let mut names: Vec<String> = s.properties.keys().cloned().collect();
                    names.sort();
                    scopes.extend(names.into_iter().map(Some));
                }
            }
        }

        for prop in scopes {
            let resolved =
                resolve_spatial_policy(None, schema.as_ref(), prop.as_deref().unwrap_or(""));
            out.push(SpatialConfigEntry {
                workspace: ws_name.clone(),
                property: prop.unwrap_or_else(|| "*".into()),
                precisions: resolved.precisions.clone(),
                srid: resolved.srid,
                bucket_property: resolved.bucket_property.clone(),
                cover: format!("{:?}", resolved.cover).to_lowercase(),
                policy_hash: resolved.policy_hash(),
                rebuild_job_id: None,
            });
        }
    }

    Ok(Json(out))
}

/// Query parameters for the read endpoints.
#[derive(Debug, Deserialize)]
pub struct SpatialScopeQuery {
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub property: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
}

/// Write the declared spatial policy.
///
/// PUT /api/admin/management/database/:tenant/:repo/spatial/config
#[cfg(feature = "storage-rocksdb")]
pub async fn put_spatial_config(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Json(req): Json<SpatialConfigRequest>,
) -> Result<Json<SpatialConfigEntry>, ApiError> {
    let storage = storage(&state)?;
    let scope = RepoScope::new(&tenant, &repo);

    let mut ws = storage
        .workspaces()
        .get(scope, &req.workspace)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| {
            err(
                StatusCode::NOT_FOUND,
                format!("Workspace '{}' not found", req.workspace),
            )
        })?;

    let property = req.property.as_deref();

    let settings = if req.reset {
        None
    } else {
        let mut merged: SpatialPropertySchema = ws
            .config
            .spatial
            .as_ref()
            .map(|s| s.scope(property))
            .unwrap_or_default();

        if let Some(precisions) = &req.precisions {
            // Validated here rather than trusted: this record replicates, so a bad
            // value reaches every peer before anyone reads it back.
            for p in precisions {
                if !(1..=12).contains(p) {
                    return Err(err(
                        StatusCode::BAD_REQUEST,
                        format!("precision {} is out of range (1..=12)", p),
                    ));
                }
            }
            if precisions.is_empty() {
                return Err(err(StatusCode::BAD_REQUEST, "precisions must not be empty"));
            }
            merged.precisions = Some(sorted_precisions(precisions.clone()));
        }
        if let Some(srid) = req.srid {
            merged.srid = Some(srid);
        }
        if let Some(bucket) = &req.bucket_property {
            merged.bucket_property = Some(bucket.clone());
        }
        if let Some(cover) = &req.cover {
            merged.cover = Some(match cover.to_lowercase().as_str() {
                "centroid" => SpatialCoverMode::Centroid,
                "extent" => SpatialCoverMode::Extent,
                other => {
                    return Err(err(
                        StatusCode::BAD_REQUEST,
                        format!(
                            "unknown cover mode '{}': expected centroid or extent",
                            other
                        ),
                    ))
                }
            });
        }
        Some(merged)
    };

    ws.config.spatial =
        SpatialWorkspaceSchema::edited(ws.config.spatial.clone(), property, settings);
    let schema = ws.config.spatial.clone();

    storage
        .workspaces()
        .put(scope, ws)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let resolved = resolve_spatial_policy(None, schema.as_ref(), property.unwrap_or(""));

    // A configuration change that nothing acts on is the whole bug this endpoint
    // used to have: the write path now emits `configured ∪ indexed` immediately,
    // but the entries the OLD policy produced and the new one does not want are
    // only removed by a build. Queue it here rather than waiting for the next
    // write to this workspace to notice the drift — a read-mostly workspace could
    // wait forever.
    let rebuild_job_id =
        queue_rebuild_if_drifted(&state, &tenant, &repo, &req.workspace, property).await;

    Ok(Json(SpatialConfigEntry {
        workspace: req.workspace.clone(),
        property: property.unwrap_or("*").to_string(),
        precisions: resolved.precisions.clone(),
        srid: resolved.srid,
        bucket_property: resolved.bucket_property.clone(),
        cover: format!("{:?}", resolved.cover).to_lowercase(),
        policy_hash: resolved.policy_hash(),
        rebuild_job_id,
    }))
}

/// Queue a LOCAL `SpatialIndexBuild` when the freshly written configuration no
/// longer matches what this node's index was built under.
///
/// Never fails the configuration write: the record is already durable and
/// replicated at this point, and reconciliation on the next node event is a
/// second chance at the same job. Returns `None` when nothing has drifted, when
/// no index has been built yet (the first write creates it at the new policy
/// anyway), or when the enqueue failed — in which case the reason is logged.
#[cfg(feature = "storage-rocksdb")]
async fn queue_rebuild_if_drifted(
    state: &AppState,
    tenant: &str,
    repo: &str,
    workspace: &str,
    property: Option<&str>,
) -> Option<String> {
    let storage = state.rocksdb_storage.as_ref()?;
    let branch = get_branch_name(state, tenant, repo, None).await.ok()?;
    let admin = storage.spatial_admin()?;

    let states = admin.states(tenant, repo, &branch, workspace).ok()?;
    let schema = storage
        .workspaces()
        .get(RepoScope::new(tenant, repo), workspace)
        .await
        .ok()
        .flatten()
        .and_then(|w| w.config.spatial);

    let drifted = states.iter().any(|(name, s)| {
        if property.is_some_and(|want| want != name) {
            return false;
        }
        s.policy_hash != resolve_spatial_policy(None, schema.as_ref(), name).policy_hash()
    });
    if !drifted {
        return None;
    }

    match storage
        .queue_spatial_index_build(tenant, repo, &branch, workspace, property, true)
        .await
    {
        Ok(job_id) => Some(job_id.0),
        Err(e) => {
            tracing::warn!(
                error = %e,
                workspace = %workspace,
                "Spatial policy changed but the rebuild could not be queued; \
                 reconciliation will retry on the next write to this workspace"
            );
            None
        }
    }
}

/// Local index state plus a physical census.
///
/// GET /api/admin/management/database/:tenant/:repo/spatial/health
#[cfg(feature = "storage-rocksdb")]
pub async fn get_spatial_health(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(params): Query<SpatialScopeQuery>,
) -> Result<Json<Vec<SpatialHealthEntry>>, ApiError> {
    let storage = storage(&state)?;
    let branch = get_branch_name(&state, &tenant, &repo, params.branch.clone()).await?;
    let admin = storage.spatial_admin().ok_or_else(|| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Spatial index administration unavailable",
        )
    })?;
    let scope = RepoScope::new(&tenant, &repo);

    let workspaces = match &params.workspace {
        Some(ws) => vec![ws.clone()],
        None => storage
            .workspaces()
            .list(scope)
            .await
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .into_iter()
            .map(|w| w.name)
            .collect(),
    };

    let mut out = Vec::new();
    for ws_name in workspaces {
        let schema = storage
            .workspaces()
            .get(scope, &ws_name)
            .await
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .and_then(|w| w.config.spatial);

        let states = admin
            .states(&tenant, &repo, &branch, &ws_name)
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let census = admin
            .census(
                &tenant,
                &repo,
                &branch,
                &ws_name,
                params.property.as_deref(),
            )
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let mut names: Vec<String> = states.iter().map(|(n, _)| n.clone()).collect();
        for c in &census {
            if !names.contains(&c.property) {
                names.push(c.property.clone());
            }
        }
        if let Some(want) = &params.property {
            names.retain(|n| n == want);
        }
        names.sort();

        for name in names {
            let state = states.iter().find(|(n, _)| *n == name).map(|(_, s)| s);
            let counts = census
                .iter()
                .find(|c| c.property == name)
                .cloned()
                .unwrap_or_default();
            let configured = resolve_spatial_policy(None, schema.as_ref(), &name);

            out.push(SpatialHealthEntry {
                workspace: ws_name.clone(),
                property: name.clone(),
                phase: state
                    .map(|s| format!("{:?}", s.phase).to_lowercase())
                    .unwrap_or_else(|| "not_built".to_string()),
                indexed_precisions: state.map(|s| s.precisions.clone()).unwrap_or_default(),
                indexed_policy_hash: state.map(|s| s.policy_hash).unwrap_or(0),
                configured_precisions: configured.precisions.clone(),
                configured_policy_hash: configured.policy_hash(),
                needs_rebuild: match state {
                    None => true,
                    Some(s) => {
                        s.policy_hash != configured.policy_hash()
                            || s.phase == raisin_storage::spatial::SpatialBuildPhase::NotBuilt
                    }
                },
                nodes_indexed: state.map(|s| s.nodes_indexed).unwrap_or(0),
                live_entries: counts.live_entries,
                tombstoned_entries: counts.tombstoned_entries,
                distinct_nodes: counts.distinct_nodes,
                legacy_entries: counts.legacy_entries,
                live_per_precision: counts.live_per_precision.clone(),
            });
        }
    }

    Ok(Json(out))
}

/// Queue a LOCAL spatial index rebuild.
///
/// POST /api/admin/management/database/:tenant/:repo/spatial/rebuild
#[cfg(feature = "storage-rocksdb")]
pub async fn rebuild_spatial_index(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(params): Query<DatabaseOpQuery>,
    Json(req): Json<SpatialScopeRequest>,
) -> Result<Json<JobResponse>, ApiError> {
    let storage = storage(&state)?;
    let branch = get_branch_name(&state, &tenant, &repo, params.branch).await?;

    let job_id = storage
        .queue_spatial_index_build(
            &tenant,
            &repo,
            &branch,
            &req.workspace,
            req.property.as_deref(),
            // Tombstone-then-re-emit, so a shrinking precision set actually removes
            // the entries it no longer wants.
            true,
        )
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(JobResponse {
        job_id: job_id.0,
        message: format!(
            "Spatial index rebuild queued for {}/{}/{}/{} (LOCAL to this node; \
             run on each peer, or change the replicated configuration to fan out)",
            tenant, repo, branch, req.workspace
        ),
    }))
}

/// Compare configured intent against local reality.
///
/// POST /api/admin/management/database/:tenant/:repo/spatial/verify
#[cfg(feature = "storage-rocksdb")]
pub async fn verify_spatial_index(
    State(state): State<AppState>,
    Path((tenant, repo)): Path<(String, String)>,
    Query(params): Query<DatabaseOpQuery>,
    Json(req): Json<SpatialScopeRequest>,
) -> Result<Json<Vec<SpatialVerifyEntry>>, ApiError> {
    let storage = storage(&state)?;
    let branch = get_branch_name(&state, &tenant, &repo, params.branch).await?;
    let admin = storage.spatial_admin().ok_or_else(|| {
        err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Spatial index administration unavailable",
        )
    })?;
    let scope = RepoScope::new(&tenant, &repo);

    let schema = storage
        .workspaces()
        .get(scope, &req.workspace)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .and_then(|w| w.config.spatial);

    let states = admin
        .states(&tenant, &repo, &branch, &req.workspace)
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let census = admin
        .census(
            &tenant,
            &repo,
            &branch,
            &req.workspace,
            req.property.as_deref(),
        )
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut out = Vec::new();
    for counts in &census {
        let state = states
            .iter()
            .find(|(n, _)| *n == counts.property)
            .map(|(_, s)| s);
        let configured = resolve_spatial_policy(None, schema.as_ref(), &counts.property);

        let mut problems: Vec<String> = Vec::new();
        match state {
            None => problems.push("no local index state record".into()),
            Some(s) => {
                if s.policy_hash != configured.policy_hash() {
                    problems.push(format!(
                        "policy drift: built under {} but configured is {}",
                        s.policy_hash,
                        configured.policy_hash()
                    ));
                }
            }
        }
        let indexed: Vec<usize> = counts.live_per_precision.iter().map(|(p, _)| *p).collect();
        let mut expected = configured.precisions.clone();
        expected.sort_unstable();
        if counts.live_entries > 0 && indexed != expected {
            problems.push(format!(
                "entries at precisions {:?}, configured {:?}",
                indexed, expected
            ));
        }

        out.push(SpatialVerifyEntry {
            workspace: req.workspace.clone(),
            property: counts.property.clone(),
            status: if problems.is_empty() {
                "OK".into()
            } else {
                "NEEDS_REBUILD".into()
            },
            detail: problems.join("; "),
            live_entries: counts.live_entries,
            distinct_nodes: counts.distinct_nodes,
        });
    }

    Ok(Json(out))
}
