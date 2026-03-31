// SPDX-License-Identifier: BSL-1.1

//! HTTP handler functions for SQL query execution.

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use futures::StreamExt;
use raisin_embeddings::TenantEmbeddingConfigStore;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql_execution::QueryEngine;
use raisin_storage::{RepositoryManagementRepository, Storage};

use crate::error::ApiError;
use crate::state::AppState;

use super::convert::row_to_json;
use super::engine;
use super::types::{SqlQueryRequest, SqlQueryResponse};

/// Execute a SQL query against the repository
///
/// # Endpoint
/// `POST /api/sql/{repo}`
///
/// # Request Body
/// ```json
/// {
///   "sql": "SELECT * FROM sites WHERE name = 'Home'"
/// }
/// ```
///
/// # Response
/// ```json
/// {
///   "columns": ["id", "name", "path", "properties"],
///   "rows": [
///     { "id": "home", "name": "Home", "path": "/", "properties": {} }
///   ],
///   "row_count": 1,
///   "execution_time_ms": 23
/// }
/// ```
///
/// # Notes
/// - Workspace comes from SQL: `SELECT * FROM workspace_name`
/// - Branch is taken from repository's default_branch configuration
/// - Tenant is always "default" (multi-tenancy not yet supported)
/// - Only available with `storage-rocksdb` feature flag
#[cfg(feature = "storage-rocksdb")]
pub async fn execute_sql_query(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<SqlQueryRequest>,
) -> Result<Json<SqlQueryResponse>, ApiError> {
    let tenant_id = "default";
    let auth_context = auth.map(|Extension(ctx)| ctx);

    tracing::info!("HTTP SQL Query Request");
    tracing::info!("   Repository: {}", repo);
    tracing::debug!("   SQL: {}", req.sql);

    let final_sql = substitute_params(&req.sql, &req.params)?;

    let storage = state.storage();
    let repo_mgmt = storage.repository_management();
    let repository = repo_mgmt
        .get_repository(tenant_id, &repo)
        .await?
        .ok_or_else(|| ApiError::repository_not_found(&repo))?;

    let branch = repository.config.default_branch.clone();
    tracing::info!("   Default branch: {}", branch);

    let engine = build_engine(&state, tenant_id, &repo, &branch, &repository, auth_context).await?;

    collect_and_respond(engine, &final_sql).await
}

/// Execute a SQL query against the repository with explicit branch in path
///
/// # Endpoint
/// `POST /api/sql/{repo}/{branch}`
///
/// # Request Body
/// ```json
/// {
///   "sql": "SELECT * FROM sites WHERE name = 'Home'"
/// }
/// ```
///
/// # Response
/// Same as `execute_sql_query`
#[cfg(feature = "storage-rocksdb")]
pub async fn execute_sql_query_with_branch(
    State(state): State<AppState>,
    Path((repo, branch)): Path<(String, String)>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<SqlQueryRequest>,
) -> Result<Json<SqlQueryResponse>, ApiError> {
    let tenant_id = "default";
    let auth_context = auth.map(|Extension(ctx)| ctx);

    tracing::info!("HTTP SQL Query Request (with branch)");
    tracing::info!("   Repository: {}", repo);
    tracing::info!("   Branch: {}", branch);
    tracing::debug!("   SQL: {}", req.sql);

    let final_sql = substitute_params(&req.sql, &req.params)?;

    let storage = state.storage();
    let repo_mgmt = storage.repository_management();
    let repository = repo_mgmt
        .get_repository(tenant_id, &repo)
        .await?
        .ok_or_else(|| ApiError::repository_not_found(&repo))?;

    let engine = build_engine(&state, tenant_id, &repo, &branch, &repository, auth_context).await?;

    collect_and_respond(engine, &final_sql).await
}

/// Substitute query parameters if provided.
fn substitute_params(
    sql: &str,
    params: &Option<Vec<serde_json::Value>>,
) -> Result<String, ApiError> {
    if let Some(ref params) = params {
        raisin_sql_execution::substitute_params(sql, params).map_err(|e| {
            ApiError::validation_failed(format!("Parameter substitution failed: {}", e))
        })
    } else {
        Ok(sql.to_string())
    }
}

/// Build a fully-configured QueryEngine with all features.
async fn build_engine(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    repository: &raisin_context::RepositoryInfo,
    auth_context: Option<AuthContext>,
) -> Result<QueryEngine<crate::state::Store>, ApiError> {
    let rocksdb_storage = state
        .rocksdb_storage
        .as_ref()
        .ok_or_else(|| ApiError::internal("RocksDB storage not available"))?;

    let config_repo = rocksdb_storage.tenant_embedding_config_repository();
    let embedding_config = config_repo.get_config(tenant_id).ok().flatten();

    let catalog = engine::build_catalog(state, tenant_id, repo, &embedding_config).await?;

    let job_registrar = engine::create_job_registrar(rocksdb_storage, tenant_id, repo, branch);
    let restore_tree_registrar =
        engine::create_restore_tree_registrar(rocksdb_storage, tenant_id, repo, branch);

    let mut engine = QueryEngine::new(state.storage.clone(), tenant_id, repo, branch)
        .with_catalog(catalog)
        .with_job_registrar(job_registrar)
        .with_restore_tree_registrar(restore_tree_registrar)
        .with_default_actor("http-client".to_string())
        .with_repository_config(repository.config.clone());

    // Clone auth context before consuming — callbacks need it for function node lookups
    let callback_auth = auth_context.clone();

    if let Some(auth) = auth_context {
        tracing::info!(
            "   Auth context: user_id={:?}, is_system={}",
            auth.user_id,
            auth.is_system
        );
        engine = engine.with_auth(auth);
    }

    engine = engine::configure_engine_features(engine, state, embedding_config, rocksdb_storage)?;

    // Wire function invocation callbacks for SQL INVOKE() and INVOKE_SYNC()
    let invoke_cb = engine::create_function_invoke_callback(state, repo, callback_auth.clone());
    let invoke_sync_cb = engine::create_function_invoke_sync_callback(state, repo, callback_auth);
    engine = engine
        .with_function_invoke(invoke_cb)
        .with_function_invoke_sync(invoke_sync_cb);

    Ok(engine)
}

/// Execute the query and collect results into a response.
async fn collect_and_respond(
    engine: QueryEngine<crate::state::Store>,
    final_sql: &str,
) -> Result<Json<SqlQueryResponse>, ApiError> {
    let start = std::time::Instant::now();
    let mut stream = engine
        .execute_batch(final_sql)
        .await
        .map_err(|e| ApiError::validation_failed(format!("Failed to execute SQL query: {}", e)))?;

    let mut rows = Vec::new();
    let mut row_count = 0;
    let mut explain_plan: Option<String> = None;

    while let Some(row) = stream.next().await {
        let row = row.map_err(|e| ApiError::internal(format!("Failed to fetch row: {}", e)))?;

        tracing::debug!(
            "   Row has {} columns: {:?}",
            row.columns.len(),
            row.columns.keys().collect::<Vec<_>>()
        );

        // Check if this is an EXPLAIN result (single column "QUERY PLAN")
        if row.columns.len() == 1 && row.columns.contains_key("QUERY PLAN") {
            if let Some(PropertyValue::String(plan_text)) = row.columns.get("QUERY PLAN") {
                tracing::info!("   EXPLAIN plan detected");
                explain_plan = Some(plan_text.clone());
            }
        }

        let json_row = row_to_json(&row);
        tracing::debug!("   JSON row: {}", json_row);
        rows.push(json_row);
        row_count += 1;
    }
    let execution_time_ms = start.elapsed().as_millis() as u64;

    tracing::info!(
        "Query completed: {} rows in {}ms",
        row_count,
        execution_time_ms
    );

    // Extract column names from first row (or empty if no results)
    let columns = rows
        .first()
        .and_then(|r| r.as_object())
        .map(|obj| obj.keys().cloned().collect())
        .unwrap_or_default();

    Ok(Json(SqlQueryResponse {
        columns,
        row_count,
        rows,
        execution_time_ms,
        explain_plan,
    }))
}
