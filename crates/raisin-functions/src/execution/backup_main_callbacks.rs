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

//! # Backup of Original Callback Code from main.rs
//!
//! This file contains the original callback code extracted from
//! `crates/raisin-server/src/main.rs` lines 733-1537.
//!
//! This is preserved for reference during the refactoring process.
//! The code is intentionally NOT compiled (#[allow(dead_code)]).
//!
//! ## Original Callbacks
//!
//! 1. **sql_executor** (lines 733-812): Executes bulk SQL jobs
//! 2. **function_executor** (lines 835-1452): Executes JS/Starlark functions
//!    - Includes nested callbacks for node ops, SQL, HTTP, AI completion
//! 3. **function_enabled_checker** (lines 1455-1494): Checks if function is enabled
//! 4. **trigger_matcher** (line 1500): Factory call to create_trigger_matcher
//! 5. **binary_retrieval** (lines 1510-1522): Retrieves binary packages

#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]

/*
=============================================================================
BACKUP: Original callback code from crates/raisin-server/src/main.rs
Lines: 733-1537
Date: 2025-12-10
=============================================================================

// =========================================================================
// SQL EXECUTOR CALLBACK (lines 733-812)
// =========================================================================

let sql_executor: raisin_rocksdb::SqlExecutorCallback = {
    use futures::StreamExt;
    use raisin_models::nodes::properties::PropertyValue;
    use raisin_sql_execution::{QueryEngine, StaticCatalog};

    let storage_for_sql = storage.clone();
    let indexing_engine_for_sql = indexing_engine.clone();
    let hnsw_engine_for_sql = hnsw_engine.clone();

    std::sync::Arc::new(
        move |sql: String,
              tenant_id: String,
              repo_id: String,
              branch: String,
              actor: String| {
            let storage = storage_for_sql.clone();
            let indexing_engine = indexing_engine_for_sql.clone();
            let hnsw_engine = hnsw_engine_for_sql.clone();

            Box::pin(async move {
                tracing::info!(
                    "📦 BulkSql executor: tenant={}, repo={}, branch={}",
                    tenant_id,
                    repo_id,
                    branch
                );

                // Build workspace catalog
                let workspaces =
                    storage.workspaces().list(&tenant_id, &repo_id).await?;

                tracing::info!(
                    "📦 BulkSql executor: found {} workspaces: {:?}",
                    workspaces.len(),
                    workspaces.iter().map(|w| &w.name).collect::<Vec<_>>()
                );

                let mut catalog = StaticCatalog::default_nodes_schema();
                for ws in &workspaces {
                    catalog.register_workspace(ws.name.clone());
                }

                // Create QueryEngine WITHOUT job_registrar to avoid infinite recursion
                // (we're already inside a job, don't want to spawn more jobs)
                let mut engine =
                    QueryEngine::new(storage.clone(), &tenant_id, &repo_id, &branch)
                        .with_catalog(std::sync::Arc::new(catalog))
                        .with_default_actor(actor);

                if let Some(idx) = &indexing_engine {
                    engine = engine.with_indexing_engine(idx.clone());
                }
                if let Some(hnsw) = &hnsw_engine {
                    engine = engine.with_hnsw_engine(hnsw.clone());
                }

                // Execute synchronously (we're in background job, no async routing)
                tracing::info!("📦 BulkSql executor: executing SQL: {}", sql);
                let mut stream = engine.execute_batch_sync(&sql).await?;

                // Count affected rows from stream
                let mut affected_rows = 0i64;
                while let Some(row) = stream.next().await {
                    let row = row?;
                    match row.columns.get("affected_rows") {
                        Some(PropertyValue::Integer(n)) => {
                            affected_rows += *n;
                        }
                        Some(PropertyValue::Float(f)) => {
                            affected_rows += *f as i64;
                        }
                        _ => {}
                    }
                }

                Ok(affected_rows)
            })
        },
    )
};

// =========================================================================
// FUNCTION EXECUTOR CALLBACK (lines 835-1452)
// =========================================================================

let (function_executor, function_enabled_checker) = {
    use raisin_functions::{
        ExecutionContext, FunctionApi, FunctionExecutor, FunctionLanguage,
        FunctionMetadata, LoadedFunction, NetworkPolicy, RaisinFunctionApi,
        RaisinFunctionApiCallbacks,
    };
    use raisin_ai::TenantAIConfigStore;
    use raisin_binary::BinaryStorage;
    use raisin_models::nodes::properties::PropertyValue;
    use raisin_rocksdb::{
        FunctionEnabledChecker, FunctionExecutionResult, FunctionExecutorCallback,
    };
    use raisin_storage::NodeRepository;

    let storage_for_func = storage.clone();
    let indexing_engine_for_func = indexing_engine.clone();
    let hnsw_engine_for_func = hnsw_engine.clone();

    // Function executor callback - executes functions via QuickJS runtime
    let function_executor: FunctionExecutorCallback = {
        let storage = storage_for_func.clone();
        let bin = bin.clone();
        let indexing_engine = indexing_engine_for_func.clone();
        let hnsw_engine = hnsw_engine_for_func.clone();

        Arc::new(
            move |function_path,
                  execution_id,
                  input,
                  tenant_id,
                  repo_id,
                  branch,
                  workspace,
                  _auth_context: Option<raisin_models::auth::AuthContext>,
                  _log_emitter: Option<raisin_storage::LogEmitter>| {
                let storage = storage.clone();
                let bin = bin.clone();
                let indexing_engine = indexing_engine.clone();
                let hnsw_engine = hnsw_engine.clone();

                Box::pin(async move {
                    let start_time = std::time::Instant::now();

                    tracing::info!(
                        execution_id = %execution_id,
                        function_path = %function_path,
                        "Executing function"
                    );

                    // Load function node from storage
                    let func_node = storage
                        .nodes()
                        .get_by_path(
                            &tenant_id,
                            &repo_id,
                            &branch,
                            &workspace,
                            &function_path,
                            None,
                        )
                        .await?;

                    let func_node = func_node.ok_or_else(|| {
                        raisin_error::Error::NotFound(format!(
                            "Function not found: {}",
                            function_path
                        ))
                    })?;

                    // Extract metadata from properties
                    let name = func_node
                        .properties
                        .get("name")
                        .and_then(|v| match v {
                            PropertyValue::String(s) => Some(s.clone()),
                            _ => None,
                        })
                        .unwrap_or_else(|| func_node.name.clone());

                    let language_str = func_node
                        .properties
                        .get("language")
                        .and_then(|v| match v {
                            PropertyValue::String(s) => Some(s.as_str()),
                            _ => None,
                        })
                        .unwrap_or("javascript");

                    let language: FunctionLanguage =
                        language_str.parse().unwrap_or(FunctionLanguage::JavaScript);

                    // Get entry_file from function properties (default: "index.js:handler")
                    let entry_file = func_node
                        .properties
                        .get("entry_file")
                        .or_else(|| func_node.properties.get("entrypoint"))
                        .and_then(|v| match v {
                            PropertyValue::String(s) => Some(s.as_str()),
                            _ => None,
                        })
                        .unwrap_or("index.js:handler");

                    // Parse entry_file to get file path and handler
                    let (entry_path, handler) = {
                        let parts: Vec<&str> = entry_file.splitn(2, ':').collect();
                        let file =
                            parts.first().map(|s| s.trim()).unwrap_or("index.js");
                        let handler =
                            parts.get(1).map(|s| s.trim()).unwrap_or("handler");
                        (file.to_string(), handler.to_string())
                    };

                    // Resolve relative path from function path
                    let resolved_path = {
                        let normalized = entry_path.replace("functions:", "");
                        if normalized.starts_with('/') {
                            normalized
                        } else {
                            // Relative path - resolve from function's directory
                            let base = function_path.trim_end_matches('/');
                            let combined = format!("{}/{}", base, normalized);
                            // Normalize path segments (handle ./ and ../)
                            let segments: Vec<&str> = combined.split('/').collect();
                            let mut stack: Vec<&str> = Vec::new();
                            for seg in segments {
                                match seg {
                                    "" | "." => continue,
                                    ".." => {
                                        stack.pop();
                                    }
                                    s => stack.push(s),
                                }
                            }
                            format!("/{}", stack.join("/"))
                        }
                    };

                    tracing::debug!(
                        function_path = %function_path,
                        entry_file = %entry_file,
                        resolved_path = %resolved_path,
                        handler = %handler,
                        "Resolving function entry point"
                    );

                    // Fetch the asset node at the resolved path
                    let asset_node = storage
                        .nodes()
                        .get_by_path(
                            &tenant_id,
                            &repo_id,
                            &branch,
                            &workspace,
                            &resolved_path,
                            None,
                        )
                        .await?
                        .ok_or_else(|| {
                            raisin_error::Error::Validation(format!(
                                "Entry file not found: {} (resolved from {})",
                                resolved_path, entry_file
                            ))
                        })?;

                    if asset_node.node_type != "raisin:Asset" {
                        return Err(raisin_error::Error::Validation(format!(
                            "Entry file at {} is not an asset (found {})",
                            resolved_path, asset_node.node_type
                        )));
                    }

                    // Load code from the asset node
                    let code = if let Some(PropertyValue::String(code)) =
                        asset_node.properties.get("code")
                    {
                        code.clone()
                    } else if let Some(PropertyValue::Resource(res)) =
                        asset_node.properties.get("file")
                    {
                        if let Some(meta) = &res.metadata {
                            if let Some(PropertyValue::String(key)) =
                                meta.get("storage_key")
                            {
                                let bytes = bin.get(key).await.map_err(|e| {
                                    raisin_error::Error::Backend(format!(
                                        "Failed to load code: {}",
                                        e
                                    ))
                                })?;
                                String::from_utf8(bytes.to_vec()).map_err(|e| {
                                    raisin_error::Error::Backend(format!(
                                        "Invalid UTF-8 in code file: {}",
                                        e
                                    ))
                                })?
                            } else {
                                return Err(raisin_error::Error::Validation(
                                    "Asset file property missing storage_key"
                                        .to_string(),
                                ));
                            }
                        } else {
                            return Err(raisin_error::Error::Validation(
                                "Asset file property missing metadata".to_string(),
                            ));
                        }
                    } else {
                        return Err(raisin_error::Error::Validation(format!(
                            "Asset at {} has no code or file property",
                            resolved_path
                        )));
                    };

                    // Build function metadata with handler as entrypoint
                    let metadata = FunctionMetadata::new(name.clone(), language)
                        .with_entry_file(handler.clone());

                    let loaded_function = LoadedFunction::new(
                        metadata,
                        code,
                        function_path.clone(),
                        func_node.id.clone(),
                        workspace.clone(),
                    );

                    // Create RaisinFunctionApi with callbacks for node/SQL operations
                    let api_callbacks = {
                        let storage_api = storage.clone();
                        let tenant = tenant_id.clone();
                        let repo = repo_id.clone();
                        let br = branch.clone();

                        RaisinFunctionApiCallbacks {
                        node_get: Some({
                            let storage = storage_api.clone();
                            let tenant = tenant.clone();
                            let repo = repo.clone();
                            let branch = br.clone();
                            Arc::new(move |ws: String, path: String| {
                                let storage = storage.clone();
                                let tenant = tenant.clone();
                                let repo = repo.clone();
                                let branch = branch.clone();
                                Box::pin(async move {
                                    let node = storage.nodes().get_by_path(&tenant, &repo, &branch, &ws, &path, None).await?;
                                    Ok(node.map(|n| serde_json::to_value(n).unwrap_or_default()))
                                })
                            })
                        }),
                        // ... [additional callbacks truncated for brevity - see full backup below]
                        // All callbacks including:
                        // - node_get_by_id, node_get_children, node_query
                        // - node_create, node_update, node_delete
                        // - sql_query, sql_execute
                        // - emit_event, http_request
                        // - ai_completion, ai_list_models, ai_get_default_model
                        ..Default::default()
                        }
                    };

                    let exec_context =
                        ExecutionContext::new(&tenant_id, &repo_id, &branch, "system")
                            .with_workspace(&workspace)
                            .with_input(input);

                    let network_policy = NetworkPolicy::default();
                    let api: Arc<dyn FunctionApi> = Arc::new(RaisinFunctionApi::new(
                        exec_context.clone(),
                        network_policy,
                        api_callbacks,
                    ));

                    // Execute the function
                    let executor = FunctionExecutor::new();
                    let result =
                        executor.execute(&loaded_function, exec_context, api).await;

                    let duration_ms = start_time.elapsed().as_millis() as u64;

                    match result {
                        Ok(exec_result) => {
                            tracing::info!(
                                execution_id = %execution_id,
                                success = exec_result.success,
                                duration_ms = duration_ms,
                                "Function execution completed"
                            );
                            Ok(FunctionExecutionResult {
                                execution_id,
                                success: exec_result.success,
                                result: exec_result.output,
                                error: exec_result.error.map(|e| e.message),
                                duration_ms,
                                logs: exec_result
                                    .logs
                                    .iter()
                                    .map(|l| format!("[{}] {}", l.level, l.message))
                                    .collect(),
                            })
                        }
                        Err(e) => {
                            tracing::error!(
                                execution_id = %execution_id,
                                error = %e,
                                "Function execution failed"
                            );
                            Ok(FunctionExecutionResult {
                                execution_id,
                                success: false,
                                result: None,
                                error: Some(e.to_string()),
                                duration_ms,
                                logs: vec![format!("[error] {}", e)],
                            })
                        }
                    }
                })
            },
        )
    };

    // Function enabled checker - checks if a function is enabled before execution
    let function_enabled_checker: FunctionEnabledChecker = {
        let storage = storage_for_func.clone();

        Arc::new(
            move |function_path, tenant_id, repo_id, branch, workspace| {
                let storage = storage.clone();

                Box::pin(async move {
                    let func_node = storage
                        .nodes()
                        .get_by_path(
                            &tenant_id,
                            &repo_id,
                            &branch,
                            &workspace,
                            &function_path,
                            None,
                        )
                        .await?;

                    if let Some(node) = func_node {
                        let enabled = node
                            .properties
                            .get("enabled")
                            .and_then(|v| match v {
                                PropertyValue::Boolean(b) => Some(*b),
                                _ => None,
                            })
                            .unwrap_or(true); // Default to enabled if not specified
                        Ok(enabled)
                    } else {
                        Err(raisin_error::Error::NotFound(format!(
                            "Function not found: {}",
                            function_path
                        )))
                    }
                })
            },
        )
    };

    (Some(function_executor), Some(function_enabled_checker))
};

// =========================================================================
// TRIGGER MATCHER (line 1500)
// =========================================================================

let trigger_matcher = Some(raisin_rocksdb::create_trigger_matcher(storage.clone()));

// =========================================================================
// SCHEDULED TRIGGER FINDER (lines 1504-1506)
// =========================================================================

let scheduled_trigger_finder: Option<raisin_rocksdb::ScheduledTriggerFinderCallback> =
    None;
// TODO: Implement scheduled trigger finder when cron triggers are needed

// =========================================================================
// BINARY RETRIEVAL CALLBACK (lines 1510-1522)
// =========================================================================

let binary_retrieval: raisin_rocksdb::BinaryRetrievalCallback = {
    use raisin_binary::BinaryStorage;
    let bin_for_jobs = bin.clone();
    Arc::new(move |key: String| {
        let bin = bin_for_jobs.clone();
        Box::pin(async move {
            bin.get(&key)
                .await
                .map(|bytes| bytes.to_vec())
                .map_err(|e| raisin_error::Error::storage(format!("Failed to retrieve binary: {}", e)))
        })
    })
};

// =========================================================================
// JOB SYSTEM INITIALIZATION (lines 1526-1539)
// =========================================================================

let (pool, token) = storage_for_init
    .init_job_system(
        indexing_engine.clone().unwrap(),
        hnsw_engine.clone().unwrap(),
        Some(sql_executor),
        None, // copy_tree_executor: will be added if large tree copies need background processing
        function_executor,
        function_enabled_checker,
        trigger_matcher,
        scheduled_trigger_finder,
        Some(binary_retrieval),
        Some(binary_storage), // binary_storage for package asset uploads
    )
    .await
    .expect("Failed to initialize job system");

*/
