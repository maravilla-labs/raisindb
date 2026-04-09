// SPDX-License-Identifier: BSL-1.1

//! Shared query engine setup logic for SQL handlers.

use std::sync::Arc;

use raisin_embeddings::crypto::ApiKeyEncryptor;
use raisin_embeddings::provider::create_provider;
use raisin_embeddings::TenantEmbeddingConfigStore;
use raisin_sql_execution::{
    FunctionInvokeCallback, FunctionInvokeSyncCallback, JobRegistrarCallback, QueryEngine,
    RestoreTreeRegistrarCallback, StaticCatalog,
};
use raisin_storage::{scope::RepoScope, JobType, Storage, WorkspaceRepository};

use crate::error::ApiError;
use crate::state::AppState;

/// Build catalog with all workspaces and optional embedding support.
pub(super) async fn build_catalog(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    embedding_config: &Option<raisin_embeddings::TenantEmbeddingConfig>,
) -> Result<Arc<StaticCatalog>, ApiError> {
    let storage = state.storage();

    tracing::debug!("   Fetching workspaces from repository...");
    let workspaces = storage
        .workspaces()
        .list(RepoScope::new(tenant_id, repo))
        .await?;

    tracing::info!(
        "   Found {} workspaces: {:?}",
        workspaces.len(),
        workspaces.iter().map(|w| &w.name).collect::<Vec<_>>()
    );

    // Create a catalog with all workspaces registered
    let mut catalog = StaticCatalog::default_nodes_schema();

    // Add embedding column if embeddings are enabled for this tenant
    if let Some(ref config) = embedding_config {
        if config.enabled {
            tracing::info!(
                "   Embedding support enabled: dimensions={}",
                config.dimensions
            );
            catalog = catalog.with_embedding_column(config.dimensions);
        }
    }

    for workspace in &workspaces {
        catalog.register_workspace(workspace.name.clone());
    }

    Ok(Arc::new(catalog))
}

/// Create job registrar callback for async bulk SQL operations.
pub(super) fn create_job_registrar(
    rocksdb_storage: &raisin_rocksdb::RocksDBStorage,
    tenant_id: &str,
    repo: &str,
    branch: &str,
) -> JobRegistrarCallback {
    let rocksdb = rocksdb_storage.clone();
    let tenant_id_owned = tenant_id.to_string();
    let repo_owned = repo.to_string();
    let branch_owned = branch.to_string();

    Arc::new(move |sql: String, actor: String| {
        let rocksdb = rocksdb.clone();
        let tenant_id = tenant_id_owned.clone();
        let repo_id = repo_owned.clone();
        let branch = branch_owned.clone();

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

            let job_context = raisin_storage::jobs::JobContext {
                tenant_id: tenant_id.clone(),
                repo_id: repo_id.clone(),
                branch: branch.clone(),
                workspace_id: String::new(),
                revision: raisin_hlc::HLC::now(),
                metadata,
            };

            // Register job
            let job_id = job_registry
                .register_job(job_type, Some(tenant_id), None, None, None)
                .await
                .map_err(|e| {
                    raisin_error::Error::Backend(format!("Failed to register job: {}", e))
                })?;

            // Store context
            job_data_store.put(&job_id, &job_context).map_err(|e| {
                raisin_error::Error::Backend(format!("Failed to store job context: {}", e))
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

                // Register job
                let job_id = job_registry
                    .register_job(job_type, Some(tenant_id), None, None, None)
                    .await
                    .map_err(|e| {
                        raisin_error::Error::Backend(format!(
                            "Failed to register RestoreTree job: {}",
                            e
                        ))
                    })?;

                // Store context
                job_data_store.put(&job_id, &job_context).map_err(|e| {
                    raisin_error::Error::Backend(format!(
                        "Failed to store RestoreTree job context: {}",
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
pub(super) fn create_function_invoke_callback(
    state: &AppState,
    repo: &str,
    _auth_context: Option<raisin_models::auth::AuthContext>,
) -> FunctionInvokeCallback {
    let state = state.clone();
    let repo = repo.to_string();

    Arc::new(
        move |path: String, input: serde_json::Value, workspace: Option<String>| {
            let state = state.clone();
            let repo = repo.clone();

            Box::pin(async move {
                let rocksdb = state.rocksdb_storage.as_ref().ok_or_else(|| {
                    raisin_error::Error::Backend("RocksDB storage not available".into())
                })?;

                let ws = workspace.as_deref().unwrap_or("functions");

                // Find function node via canonical code_loader
                let function_node = raisin_functions::execution::code_loader::find_function(
                    state.storage.as_ref(),
                    "default",
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
                    tenant_id: "default".to_string(),
                    repo_id: repo.clone(),
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

/// Create a callback for sync function invocation via SQL INVOKE_SYNC().
pub(super) fn create_function_invoke_sync_callback(
    state: &AppState,
    repo: &str,
    _auth_context: Option<raisin_models::auth::AuthContext>,
) -> FunctionInvokeSyncCallback {
    let state = state.clone();
    let repo = repo.to_string();

    Arc::new(
        move |path: String, input: serde_json::Value, workspace: Option<String>| {
            let state = state.clone();
            let repo = repo.clone();

            Box::pin(async move {
                let ws = workspace.as_deref().unwrap_or("functions");

                // Find function node via canonical code_loader
                let function_node = raisin_functions::execution::code_loader::find_function(
                    state.storage.as_ref(),
                    "default",
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
                        "default",
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

                // Build execution context and API
                let context =
                    raisin_functions::ExecutionContext::new("default", &repo, "main", "system")
                        .with_workspace(ws)
                        .with_input(input);

                let api = crate::handlers::functions::build_function_api(
                    &state,
                    &repo,
                    loaded.metadata.network_policy.clone(),
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
    // Decrypt API key
    let master_key = std::env::var("RAISIN_MASTER_KEY")
        .map_err(|_| ApiError::internal("RAISIN_MASTER_KEY not set"))?;
    let master_key_bytes: [u8; 32] = hex::decode(&master_key)
        .map_err(|e| ApiError::internal(format!("Invalid master key hex: {}", e)))?
        .try_into()
        .map_err(|_| ApiError::internal("Master key must be 32 bytes"))?;

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
