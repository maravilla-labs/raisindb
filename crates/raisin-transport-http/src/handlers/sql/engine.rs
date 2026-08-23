// SPDX-License-Identifier: BSL-1.1

//! Shared query engine setup logic for SQL handlers.

use std::sync::Arc;

use raisin_embeddings::crypto::ApiKeyEncryptor;
use raisin_embeddings::provider::create_provider;
use raisin_embeddings::TenantEmbeddingConfigStore;
use raisin_models::auth::AuthContext;
use raisin_sql_execution::engine::catalog_cache::workspace_catalog_with_embeddings;
use raisin_sql_execution::{
    FunctionInvokeCallback, FunctionInvokeSyncCallback, JobRegistrarCallback, QueryEngine,
    RestoreTreeRegistrarCallback, StaticCatalog,
};
use raisin_storage::JobType;

use crate::error::ApiError;
use crate::state::AppState;

/// Build catalog with all workspaces and optional embedding support.
///
/// Delegates to the shared, cached builder in `raisin-sql-execution` — this used
/// to list every workspace and rebuild the catalog on each request.
pub(super) async fn build_catalog(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    embedding_config: &Option<raisin_embeddings::TenantEmbeddingConfig>,
) -> Result<Arc<StaticCatalog>, ApiError> {
    let embedding_dimensions = embedding_config
        .as_ref()
        .filter(|c| c.enabled)
        .map(|c| c.dimensions);

    let catalog = workspace_catalog_with_embeddings(
        state.storage().as_ref(),
        tenant_id,
        repo,
        embedding_dimensions,
    )
    .await?;

    Ok(catalog)
}

/// Create job registrar callback for async bulk SQL operations.
///
/// `auth_context` is the identity of the request that submitted the SQL. It is
/// persisted with the job so the deferred bulk write executes under the same RLS
/// as a synchronous request would — the async path must never silently escalate
/// to system privileges.
pub(super) fn create_job_registrar(
    rocksdb_storage: &raisin_rocksdb::RocksDBStorage,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    auth_context: Option<AuthContext>,
) -> JobRegistrarCallback {
    let rocksdb = rocksdb_storage.clone();
    let tenant_id_owned = tenant_id.to_string();
    let repo_owned = repo.to_string();
    let branch_owned = branch.to_string();
    let auth_owned = auth_context;

    Arc::new(move |sql: String, actor: String| {
        let rocksdb = rocksdb.clone();
        let tenant_id = tenant_id_owned.clone();
        let repo_id = repo_owned.clone();
        let branch = branch_owned.clone();
        let auth = auth_owned.clone();

        Box::pin(async move {
            let job_registry = rocksdb.job_registry();
            let job_data_store = rocksdb.job_data_store();

            // Create job type
            let job_type = JobType::BulkSql {
                sql: sql.clone(),
                actor,
            };

            // Create job context
            let mut metadata = std::collections::HashMap::new();
            metadata.insert("sql".to_string(), serde_json::json!(sql));
            // Persist the submitter's identity so the deferred write keeps their RLS.
            if let Some(auth) = &auth {
                if let Ok(auth_json) = serde_json::to_value(auth) {
                    metadata.insert("auth_context".to_string(), auth_json);
                }
            }

            let job_context = raisin_storage::jobs::JobContext {
                tenant_id: tenant_id.clone(),
                repo_id: repo_id.clone(),
                branch: branch.clone(),
                workspace_id: String::new(),
                revision: raisin_hlc::HLC::now(),
                metadata,
            };

            // Store context BEFORE registering so dispatch can never
            // observe the job without its context.
            let job_id = raisin_storage::jobs::JobId::new();
            job_data_store.put(&job_id, &job_context).map_err(|e| {
                raisin_error::Error::Backend(format!("Failed to store job context: {}", e))
            })?;

            // Register job under the pre-generated ID
            job_registry
                .register_job_with_id(job_id.clone(), job_type, tenant_id, None, None, None)
                .await
                .map_err(|e| {
                    raisin_error::Error::Backend(format!("Failed to register job: {}", e))
                })?;

            Ok(job_id.to_string())
        })
    })
}

/// Create restore tree registrar callback for RESTORE TREE operations.
pub(super) fn create_restore_tree_registrar(
    rocksdb_storage: &raisin_rocksdb::RocksDBStorage,
    tenant_id: &str,
    repo: &str,
    branch: &str,
) -> RestoreTreeRegistrarCallback {
    let rocksdb = rocksdb_storage.clone();
    let tenant_id_owned = tenant_id.to_string();
    let repo_owned = repo.to_string();
    let branch_owned = branch.to_string();

    Arc::new(
        move |node_id: String,
              node_path: String,
              revision_hlc: String,
              translations: Option<Vec<String>>,
              actor: String| {
            let rocksdb = rocksdb.clone();
            let tenant_id = tenant_id_owned.clone();
            let repo_id = repo_owned.clone();
            let branch = branch_owned.clone();

            Box::pin(async move {
                let job_registry = rocksdb.job_registry();
                let job_data_store = rocksdb.job_data_store();

                // Create job type
                let job_type = JobType::RestoreTree {
                    node_id: node_id.clone(),
                    node_path: node_path.clone(),
                    revision_hlc: revision_hlc.clone(),
                    recursive: true, // RESTORE TREE is always recursive
                    translations: translations.clone(),
                };

                // Create job context
                let mut metadata = std::collections::HashMap::new();
                metadata.insert("actor".to_string(), serde_json::json!(actor));
                metadata.insert("node_id".to_string(), serde_json::json!(node_id));
                metadata.insert("node_path".to_string(), serde_json::json!(node_path));
                metadata.insert("revision_hlc".to_string(), serde_json::json!(revision_hlc));

                let job_context = raisin_storage::jobs::JobContext {
                    tenant_id: tenant_id.clone(),
                    repo_id: repo_id.clone(),
                    branch: branch.clone(),
                    workspace_id: "default".to_string(),
                    revision: raisin_hlc::HLC::now(),
                    metadata,
                };

                // Store context BEFORE registering so dispatch can never
                // observe the job without its context.
                let job_id = raisin_storage::jobs::JobId::new();
                job_data_store.put(&job_id, &job_context).map_err(|e| {
                    raisin_error::Error::Backend(format!(
                        "Failed to store RestoreTree job context: {}",
                        e
                    ))
                })?;

                // Register job under the pre-generated ID
                job_registry
                    .register_job_with_id(job_id.clone(), job_type, tenant_id, None, None, None)
                    .await
                    .map_err(|e| {
                        raisin_error::Error::Backend(format!(
                            "Failed to register RestoreTree job: {}",
                            e
                        ))
                    })?;

                Ok(job_id.to_string())
            })
        },
    )
}

/// Configure a QueryEngine with optional features.
pub(super) fn configure_engine_features(
    mut engine: QueryEngine<crate::state::Store>,
    state: &AppState,
    embedding_config: Option<raisin_embeddings::TenantEmbeddingConfig>,
    rocksdb_storage: &raisin_rocksdb::RocksDBStorage,
) -> Result<QueryEngine<crate::state::Store>, ApiError> {
    if let Some(idx_engine) = &state.indexing_engine {
        engine = engine.with_indexing_engine(idx_engine.clone());
    }

    if let Some(hnsw_engine) = &state.hnsw_engine {
        engine = engine.with_hnsw_engine(hnsw_engine.clone());
    }

    if let Some(config) = embedding_config {
        if config.enabled {
            engine = configure_embedding_provider(engine, &config, rocksdb_storage)?;
        }
    }

    // Wire embedding config store for SQL AI config management
    let config_store = rocksdb_storage.tenant_embedding_config_repository();
    engine = engine.with_embedding_config_store(Arc::new(config_store));

    if let Ok(master_key) = state.get_master_key() {
        engine = engine.with_master_key(master_key);
    }

    Ok(engine)
}

/// Create a callback for async function invocation via SQL INVOKE().
///
/// `tenant_id` must be the request's authoritative tenant (from `TenantInfo`);
/// it is what binds the loaded function and its registered job to the caller's
/// data plane. Passing the wrong value here lets a SQL query in tenant T
/// execute functions registered in another tenant.
pub(super) fn create_function_invoke_callback(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    _auth_context: Option<raisin_models::auth::AuthContext>,
) -> FunctionInvokeCallback {
    let state = state.clone();
    let tenant_id = tenant_id.to_string();
    let repo = repo.to_string();

    Arc::new(
        move |path: String, input: serde_json::Value, workspace: Option<String>| {
            let state = state.clone();
            let tenant_id = tenant_id.clone();
            let repo = repo.clone();

            Box::pin(async move {
                let rocksdb = state.rocksdb_storage.as_ref().ok_or_else(|| {
                    raisin_error::Error::Backend("RocksDB storage not available".into())
                })?;

                let ws = workspace.as_deref().unwrap_or("functions");

                // Find function node via canonical code_loader
                let function_node = raisin_functions::execution::code_loader::find_function(
                    state.storage.as_ref(),
                    &tenant_id,
                    &repo,
                    "main",
                    ws,
                    &path,
                )
                .await?;

                // Register background job
                let execution_id = nanoid::nanoid!();
                let job_type = JobType::FunctionExecution {
                    function_path: function_node.path.clone(),
                    trigger_name: Some("sql".into()),
                    execution_id: execution_id.clone(),
                };

                let mut metadata = std::collections::HashMap::new();
                metadata.insert("input".to_string(), input);

                let context = raisin_storage::jobs::JobContext {
                    tenant_id: tenant_id.clone(),
                    repo_id: repo.clone(),
                    branch: "main".to_string(),
                    workspace_id: ws.to_string(),
                    revision: raisin_hlc::HLC::new(0, 0),
                    metadata,
                };

                // Store context BEFORE registering so dispatch can never
                // observe the job without its context.
                let job_id = raisin_storage::jobs::JobId::new();
                rocksdb
                    .job_data_store()
                    .put(&job_id, &context)
                    .map_err(|e| {
                        raisin_error::Error::Backend(format!("Failed to store job context: {}", e))
                    })?;

                rocksdb
                    .job_registry()
                    .register_job_with_id(
                        job_id.clone(),
                        job_type,
                        tenant_id.clone(),
                        None,
                        None,
                        None,
                    )
                    .await
                    .map_err(|e| {
                        raisin_error::Error::Backend(format!("Failed to register job: {}", e))
                    })?;

                Ok((execution_id, job_id.to_string()))
            })
        },
    )
}

/// Create a callback for sync function invocation via SQL INVOKE_SYNC().
///
/// See [`create_function_invoke_callback`] — the same tenant-binding rule applies:
/// the function code AND its API surface (via `build_function_api`) are scoped
/// to `tenant_id`.
pub(super) fn create_function_invoke_sync_callback(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    _auth_context: Option<raisin_models::auth::AuthContext>,
) -> FunctionInvokeSyncCallback {
    let state = state.clone();
    let tenant_id = tenant_id.to_string();
    let repo = repo.to_string();

    Arc::new(
        move |path: String, input: serde_json::Value, workspace: Option<String>| {
            let state = state.clone();
            let tenant_id = tenant_id.clone();
            let repo = repo.clone();

            Box::pin(async move {
                let ws = workspace.as_deref().unwrap_or("functions");

                // Find function node via canonical code_loader
                let function_node = raisin_functions::execution::code_loader::find_function(
                    state.storage.as_ref(),
                    &tenant_id,
                    &repo,
                    "main",
                    ws,
                    &path,
                )
                .await?;

                // Load function code via code_loader (resolves entry_file property)
                let (code, metadata) =
                    raisin_functions::execution::code_loader::load_function_code(
                        state.storage.as_ref(),
                        state.bin.as_ref(),
                        &tenant_id,
                        &repo,
                        "main",
                        ws,
                        &function_node,
                        &function_node.path,
                    )
                    .await
                    .map_err(|e| {
                        raisin_error::Error::Backend(format!("Failed to load function code: {}", e))
                    })?;

                let mut loaded = raisin_functions::LoadedFunction::new(
                    metadata,
                    code,
                    function_node.path.clone(),
                    function_node.id.clone(),
                    function_node
                        .workspace
                        .clone()
                        .unwrap_or_else(|| "functions".into()),
                );

                // Importable modules. `LoadedFunction::files` is what the
                // QuickJS resolver consults and the loader is rebuilt per
                // execution for tenant isolation, so an empty map rejects EVERY
                // import — a multi-file function called from SQL would fail with
                // `Error resolving module '…' from 'entry'` while the same
                // function works from a trigger or the job executor.
                //
                // Called directly rather than through the HTTP helper because
                // this path honours the CALLER's workspace, which is not always
                // `functions`; the helper hardcodes it.
                loaded.files = crate::handlers::functions::helpers::load_function_modules_in(
                    &state,
                    &tenant_id,
                    &repo,
                    "main",
                    ws,
                    &function_node.path,
                    loaded.metadata.entry_file_path(),
                    &loaded.code,
                )
                .await;

                // Build execution context and API
                let context =
                    raisin_functions::ExecutionContext::new(&tenant_id, &repo, "main", "system")
                        .with_workspace(ws)
                        .with_input(input);

                let api = crate::handlers::functions::build_function_api(
                    &state,
                    &tenant_id,
                    &repo,
                    &loaded.metadata,
                    None,
                );

                // Execute function
                let executor = raisin_functions::FunctionExecutor::new();
                let result = executor.execute(&loaded, context, api).await.map_err(|e| {
                    raisin_error::Error::Backend(format!("Function execution failed: {}", e))
                })?;

                // Return result as JSON
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

/// Configure the embedding provider and storage on the query engine.
fn configure_embedding_provider(
    mut engine: QueryEngine<crate::state::Store>,
    config: &raisin_embeddings::TenantEmbeddingConfig,
    rocksdb_storage: &raisin_rocksdb::RocksDBStorage,
) -> Result<QueryEngine<crate::state::Store>, ApiError> {
    // Decrypt API key. Goes through the shared loader so this path honours the
    // legacy EMBEDDING_MASTER_KEY fallback like the rest of the server does.
    let master_key_bytes = raisin_crypto::master_key_with_embedding_fallback()
        .map_err(|e| ApiError::internal(format!("Invalid master key: {}", e)))?
        .ok_or_else(|| ApiError::internal("RAISIN_MASTER_KEY not set"))?;

    let encryptor = ApiKeyEncryptor::new(&master_key_bytes);
    if let Some(api_key_encrypted) = &config.api_key_encrypted {
        let api_key = encryptor
            .decrypt(api_key_encrypted)
            .map_err(|e| ApiError::internal(format!("Failed to decrypt API key: {}", e)))?;

        // Create embedding provider
        let provider = create_provider(&config.provider, &api_key, &config.model).map_err(|e| {
            ApiError::internal(format!("Failed to create embedding provider: {}", e))
        })?;

        engine = engine.with_embedding_provider(Arc::from(provider));
        tracing::debug!("   Embedding provider configured: {:?}", config.provider);

        // Also configure embedding storage for reading embeddings from RocksDB
        let embedding_storage = Arc::new(raisin_rocksdb::RocksDBEmbeddingStorage::new(
            rocksdb_storage.db().clone(),
        ));
        engine = engine.with_embedding_storage(embedding_storage);
        tracing::debug!("   Embedding storage configured (can read embeddings from RocksDB)");
    }

    Ok(engine)
}
