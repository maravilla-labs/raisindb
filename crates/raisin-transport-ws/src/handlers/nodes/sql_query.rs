// SPDX-License-Identifier: BSL-1.1

//! SQL query execution handler for WebSocket transport.

use parking_lot::RwLock;
use raisin_storage::{
    scope::RepoScope, transactional::TransactionalStorage, RepositoryManagementRepository,
    WorkspaceRepository,
};
use std::sync::Arc;

use crate::{
    error::WsError,
    handler::WsState,
    protocol::{RequestEnvelope, ResponseEnvelope, SqlQueryPayload},
};

/// Handle SQL query
pub async fn handle_sql_query<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<crate::connection::ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    #[cfg(feature = "storage-rocksdb")]
    {
        use futures::StreamExt;
        use raisin_sql_execution::{QueryEngine, StaticCatalog};

        let payload: SqlQueryPayload = serde_json::from_value(request.payload.clone())?;

        // Extract and validate context
        let tenant_id = request.context.tenant_id.clone();
        let repo = request
            .context
            .repository
            .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;

        // Extract auth context and session branch for identity user authorization
        let (auth_context, session_branch) = {
            let conn = connection_state.read();
            let auth = conn.auth_context().cloned();
            let branch = conn.session_branch();
            tracing::info!(
                "[handle_sql_query] Extracted auth_context from connection: user_id={:?}, has_auth={}",
                auth.as_ref().and_then(|a| a.user_id.as_ref()),
                auth.is_some()
            );
            (auth, branch)
        };

        // Fetch repository to get config with locale fallback chains
        let repository = state
            .storage
            .repository_management()
            .get_repository(&tenant_id, &repo)
            .await?
            .ok_or_else(|| WsError::InvalidRequest(format!("Repository '{}' not found", repo)))?;

        // Determine effective branch: request context > session branch > repository default
        let branch = request
            .context
            .branch
            .or(session_branch)
            .unwrap_or_else(|| repository.config.default_branch.clone());

        // Fetch all workspaces from the repository and register them in the catalog
        let workspaces = state
            .storage
            .workspaces()
            .list(RepoScope::new(&tenant_id, &repo))
            .await?;

        // Create a catalog with all workspaces registered
        let mut catalog = StaticCatalog::default_nodes_schema();
        for workspace in &workspaces {
            catalog.register_workspace(workspace.name.clone());
        }

        // Create QueryEngine with storage, catalog, and repository config for locale translation
        // Note: WS handler doesn't have job registrar, so bulk ops execute synchronously
        let mut engine = QueryEngine::new(state.storage.clone(), &tenant_id, &repo, &branch)
            .with_catalog(Arc::new(catalog))
            .with_repository_config(repository.config.clone());

        // Clone auth context before consuming — callback needs it for function node lookups
        let callback_auth = auth_context.clone();

        // Apply auth context if present (for identity user RLS)
        if let Some(auth) = auth_context {
            engine = engine.with_auth(auth);
        }

        // Wire INVOKE/INVOKE_SYNC callbacks for function execution from SQL
        let invoke_cb = build_invoke_callback(state, &repo, callback_auth.clone());
        let invoke_sync_cb = build_invoke_sync_callback(state, &repo, callback_auth);
        engine = engine
            .with_function_invoke(invoke_cb)
            .with_function_invoke_sync(invoke_sync_cb);

        // Substitute parameters if provided (for SQL injection protection)
        let final_sql = if let Some(ref params) = payload.params {
            raisin_sql_execution::substitute_params(&payload.query, params).map_err(|e| {
                WsError::InvalidRequest(format!("Parameter substitution failed: {}", e))
            })?
        } else {
            payload.query.clone()
        };

        // Execute query (supports single or multiple statements for transactions)
        // QueryEngine returns RowStream in all cases - for async bulk ops (if job registrar
        // were configured), it would return a single row with job_id, status, message columns
        let mut stream = engine
            .execute_batch(&final_sql)
            .await
            .map_err(|e| WsError::OperationError(format!("SQL query failed: {}", e)))?;

        // Check if USE BRANCH was executed and update session branch
        if let Some(new_branch) = engine.take_pending_session_branch().await {
            tracing::info!("WebSocket SQL: Setting session branch to: {}", new_branch);
            connection_state.read().set_session_branch(new_branch);
        }

        // Collect all rows from the stream
        let mut rows = Vec::new();
        while let Some(row) = stream.next().await {
            let row =
                row.map_err(|e| WsError::OperationError(format!("Failed to fetch row: {}", e)))?;

            // Convert row to JSON
            let mut json_row = serde_json::Map::new();
            for (col_name, prop_value) in &row.columns {
                json_row.insert(col_name.clone(), serde_json::to_value(prop_value)?);
            }
            rows.push(serde_json::Value::Object(json_row));
        }

        // Extract column names from first row (or empty if no results)
        let columns: Vec<String> = rows
            .first()
            .and_then(|r| r.as_object())
            .map(|obj| obj.keys().cloned().collect())
            .unwrap_or_default();

        let result = serde_json::json!({
            "columns": columns,
            "rows": rows,
            "row_count": rows.len(),
        });

        Ok(Some(ResponseEnvelope::success(request.request_id, result)))
    }

    #[cfg(not(feature = "storage-rocksdb"))]
    {
        Ok(Some(ResponseEnvelope::error(
            request.request_id,
            "NOT_SUPPORTED".to_string(),
            "SQL queries only supported with RocksDB backend".to_string(),
        )))
    }
}

// ============================================================================
// INVOKE callback builder (async background jobs)
// ============================================================================

/// Build an `INVOKE` callback for async function execution from SQL.
///
/// Registers a `FunctionExecution` job and returns `(execution_id, job_id)`.
#[cfg(feature = "storage-rocksdb")]
fn build_invoke_callback<S, B>(
    state: &Arc<WsState<S, B>>,
    repo: &str,
    _auth_context: Option<raisin_models::auth::AuthContext>,
) -> raisin_sql_execution::FunctionInvokeCallback
where
    S: raisin_storage::Storage + TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    let ws_state = state.clone();
    let ws_repo = repo.to_string();

    Arc::new(
        move |path: String, input: serde_json::Value, workspace: Option<String>| {
            let ws_state = ws_state.clone();
            let ws_repo = ws_repo.clone();

            Box::pin(async move {
                let rocksdb = ws_state.rocksdb_storage.as_ref().ok_or_else(|| {
                    raisin_error::Error::Backend("RocksDB storage not available".into())
                })?;
                let ws = workspace.as_deref().unwrap_or("functions");

                // Find function node via canonical code_loader
                let function_node = raisin_functions::execution::code_loader::find_function(
                    &*ws_state.storage,
                    "default",
                    &ws_repo,
                    "main",
                    ws,
                    &path,
                )
                .await
                .map_err(|e| {
                    raisin_error::Error::Backend(format!("Failed to find function: {}", e))
                })?;

                // Register background job
                let execution_id = nanoid::nanoid!();
                let job_type = raisin_storage::jobs::JobType::FunctionExecution {
                    function_path: function_node.path.clone(),
                    trigger_name: Some("sql".into()),
                    execution_id: execution_id.clone(),
                };

                let mut metadata = std::collections::HashMap::new();
                metadata.insert("input".to_string(), input);

                let context = raisin_storage::jobs::JobContext {
                    tenant_id: "default".to_string(),
                    repo_id: ws_repo.clone(),
                    branch: "main".to_string(),
                    workspace_id: ws.to_string(),
                    revision: raisin_hlc::HLC::new(0, 0),
                    metadata,
                };

                let job_id = rocksdb
                    .job_registry()
                    .register_job(job_type, Some("default".to_string()), None, None, None)
                    .await
                    .map_err(|e| {
                        raisin_error::Error::Backend(format!("Failed to register job: {}", e))
                    })?;

                rocksdb
                    .job_data_store()
                    .put(&job_id, &context)
                    .map_err(|e| {
                        raisin_error::Error::Backend(format!("Failed to store job context: {}", e))
                    })?;

                Ok((execution_id, job_id.to_string()))
            })
        },
    )
}

// ============================================================================
// INVOKE_SYNC callback builder
// ============================================================================

/// Build an `INVOKE_SYNC` callback for inline function execution from SQL.
///
/// Uses the canonical `code_loader::find_function` and `code_loader::load_function_code`
/// for function lookup and code loading, and `create_production_callbacks` for API wiring.
#[cfg(feature = "storage-rocksdb")]
fn build_invoke_sync_callback<S, B>(
    state: &Arc<WsState<S, B>>,
    repo: &str,
    _auth_context: Option<raisin_models::auth::AuthContext>,
) -> raisin_sql_execution::FunctionInvokeSyncCallback
where
    S: raisin_storage::Storage + TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    let ws_state = state.clone();
    let ws_repo = repo.to_string();

    Arc::new(
        move |path: String, input: serde_json::Value, workspace: Option<String>| {
            let ws_state = ws_state.clone();
            let ws_repo = ws_repo.clone();

            Box::pin(async move {
                let ws = workspace.as_deref().unwrap_or("functions");

                // Find function node via canonical code_loader
                let function_node = raisin_functions::execution::code_loader::find_function(
                    &*ws_state.storage,
                    "default",
                    &ws_repo,
                    "main",
                    ws,
                    &path,
                )
                .await
                .map_err(|e| {
                    raisin_error::Error::Backend(format!("Failed to find function: {}", e))
                })?;

                // Load function code via canonical code_loader (resolves entry_file property)
                let (code, metadata) =
                    raisin_functions::execution::code_loader::load_function_code(
                        &*ws_state.storage,
                        &*ws_state.bin,
                        "default",
                        &ws_repo,
                        "main",
                        ws,
                        &function_node,
                        &function_node.path,
                    )
                    .await
                    .map_err(|e| {
                        raisin_error::Error::Backend(format!("Failed to load function code: {}", e))
                    })?;

                let loaded = raisin_functions::LoadedFunction::new(
                    metadata,
                    code,
                    function_node.path.clone(),
                    function_node.id.clone(),
                    function_node
                        .workspace
                        .clone()
                        .unwrap_or_else(|| "functions".into()),
                );

                // Build execution context
                let context =
                    raisin_functions::ExecutionContext::new("default", &ws_repo, "main", "system")
                        .with_workspace(ws)
                        .with_input(input);

                // Build API via canonical create_production_callbacks
                let deps = Arc::new(raisin_functions::execution::ExecutionDependencies {
                    storage: ws_state.storage.clone(),
                    binary_storage: ws_state.bin.clone(),
                    indexing_engine: ws_state.indexing_engine.clone(),
                    hnsw_engine: ws_state.hnsw_engine.clone(),
                    http_client: reqwest::Client::new(),
                    ai_config_store: None,
                    job_registry: None,
                    job_data_store: None,
                });

                let callbacks = raisin_functions::execution::callbacks::create_production_callbacks(
                    deps,
                    "default".to_string(),
                    ws_repo.clone(),
                    "main".to_string(),
                    None,
                );

                let api = Arc::new(raisin_functions::RaisinFunctionApi::new(
                    raisin_functions::ExecutionContext::new("default", &ws_repo, "main", "system")
                        .with_workspace("functions"),
                    loaded.metadata.network_policy.clone(),
                    callbacks,
                ));

                // Execute function
                let executor = raisin_functions::FunctionExecutor::new();
                let result = executor.execute(&loaded, context, api).await.map_err(|e| {
                    raisin_error::Error::Backend(format!("Function execution failed: {}", e))
                })?;

                match result.output {
                    Some(output) => Ok(output),
                    None => {
                        if let Some(err) = result.error {
                            Err(raisin_error::Error::Backend(format!(
                                "Function '{}' failed: {}",
                                path, err
                            )))
                        } else {
                            Ok(serde_json::Value::Null)
                        }
                    }
                }
            })
        },
    )
}
