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

//! Main function execution orchestration.
//!
//! This module contains the primary execution logic that ties together
//! code loading, API callbacks, and the QuickJS runtime.

use std::sync::Arc;

use tracing::debug;

use raisin_binary::BinaryStorage;
use raisin_error::Result;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{NodeRepository, Storage, StorageScope};

use super::callbacks;
use super::code_loader;
use super::module_cache;
use super::types::{ExecutionDependencies, FunctionExecutionConfig, FunctionExecutionResult};

/// Resolve the module set a function may import, via the module cache.
///
/// On a miss (and always, when the cache is disabled) this does exactly what the
/// executor did inline before: a prefix scan for siblings, then one scan per
/// external `../dir/` reference. Both loaders are best-effort — a failure yields
/// an empty set rather than failing the invocation, matching the previous
/// `unwrap_or_default()` behaviour.
#[allow(clippy::too_many_arguments)]
async fn load_module_set<S, B>(
    deps: &ExecutionDependencies<S, B>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    functions_workspace: &str,
    function_path: &str,
    entry_code: &str,
    entry_file_name: &str,
) -> std::collections::HashMap<String, String>
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    if let Some(cached) = module_cache::get(
        tenant_id,
        repo_id,
        branch,
        functions_workspace,
        function_path,
    ) {
        debug!(
            function = %function_path,
            modules = cached.len(),
            "Module set served from cache"
        );
        return (*cached).clone();
    }

    let mut modules = code_loader::load_sibling_files(
        deps.storage.as_ref(),
        deps.binary_storage.as_ref(),
        tenant_id,
        repo_id,
        branch,
        functions_workspace,
        function_path,
        entry_file_name,
    )
    .await
    .unwrap_or_default();

    let external_files = code_loader::load_external_modules(
        deps.storage.as_ref(),
        deps.binary_storage.as_ref(),
        tenant_id,
        repo_id,
        branch,
        functions_workspace,
        function_path,
        entry_code,
        &modules,
    )
    .await
    .unwrap_or_default();
    modules.extend(external_files);

    module_cache::put(
        tenant_id,
        repo_id,
        branch,
        functions_workspace,
        function_path,
        Arc::new(modules.clone()),
    );

    modules
}
use crate::api::{FunctionApi, RaisinFunctionApi};
use crate::executor::FunctionExecutor;
use crate::types::{ExecutionContext, FunctionLanguage, LoadedFunction};

/// Execute a function given its path and context.
///
/// This is the main entry point for function execution. It:
/// 1. Loads the function node from storage
/// 2. Resolves the entry file and loads the code
/// 3. Creates API callbacks with all dependencies
/// 4. Executes the function via QuickJS/Starlark runtime
/// 5. Returns the execution result
///
/// The `auth_context` parameter controls RLS filtering for API operations:
/// - `None`: Operations run without auth (system context, no RLS filtering)
/// - `Some(auth)`: Operations are filtered based on user's permissions
#[allow(clippy::too_many_arguments)]
pub async fn execute_function<S, B>(
    deps: &ExecutionDependencies<S, B>,
    config: &FunctionExecutionConfig,
    function_path: &str,
    execution_id: &str,
    input: serde_json::Value,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    auth_context: Option<AuthContext>,
    log_emitter: Option<raisin_storage::LogEmitter>,
) -> Result<FunctionExecutionResult>
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    let start_time = std::time::Instant::now();

    // 1. Load function node
    let func_node = code_loader::load_function_node(
        deps.storage.as_ref(),
        tenant_id,
        repo_id,
        branch,
        &config.functions_workspace,
        function_path,
    )
    .await?;

    // 1b. Remote MCP proxy: no code to load, no runtime to spin up.
    //
    // Placed here on purpose — BEFORE `load_function_code`, which resolves the
    // entry file and parses `language`. A proxy has neither: its `language` is
    // a placeholder the schema requires, and an unrecognised value there is a
    // hard error. Because every execution path funnels through this function,
    // this single branch serves the JS chat path, `raisin.functions.execute()`,
    // the flow runtime and the HTTP invoke endpoint alike.
    if let Some(proxy) = super::remote_tool::proxy_block(&func_node) {
        return super::remote_tool::invoke_remote_tool(
            deps,
            &proxy,
            function_path,
            execution_id,
            input,
            tenant_id,
            repo_id,
            branch,
            start_time,
        )
        .await;
    }

    // 2. Load code and metadata
    let (code, metadata) = code_loader::load_function_code(
        deps.storage.as_ref(),
        deps.binary_storage.as_ref(),
        tenant_id,
        repo_id,
        branch,
        &config.functions_workspace,
        &func_node,
        function_path,
    )
    .await?;

    // 2b/2c. Resolve the module set: sibling files plus anything reachable via
    // `../dir/` imports. This is a subtree prefix scan plus one scan per external
    // dir plus a binary read per non-inline file — all of it repeated on every
    // invocation for a set that only changes when the function is edited, hence
    // the cache. Disabled by default; see `module_cache`.
    //
    // A wasm component has no module set at all: its imports were linked when
    // it was built, and there is no source to scan for `../dir/` specifiers.
    // Running the scan anyway would be a subtree scan per invocation that can
    // only ever return files the runtime cannot use.
    let entry_file_name = metadata.entry_file_path().to_string();
    let sibling_files = if metadata.language == FunctionLanguage::Wasm {
        std::collections::HashMap::new()
    } else {
        load_module_set(
            deps,
            tenant_id,
            repo_id,
            branch,
            &config.functions_workspace,
            function_path,
            code.as_text().unwrap_or(""),
            &entry_file_name,
        )
        .await
    };

    // 3. Create API callbacks
    // We need to clone the Arc to create callbacks with proper lifetimes
    let deps_arc = Arc::new(ExecutionDependencies {
        storage: deps.storage.clone(),
        binary_storage: deps.binary_storage.clone(),
        indexing_engine: deps.indexing_engine.clone(),
        hnsw_engine: deps.hnsw_engine.clone(),
        http_client: deps.http_client.clone(),
        ai_config_store: deps.ai_config_store.clone(),
        job_registry: deps.job_registry.clone(),
        job_data_store: deps.job_data_store.clone(),
        lock_manager: deps.lock_manager.clone(),
        secret_store: deps.secret_store.clone(),
        identity_repo: deps.identity_repo.clone(),
        schema_stats_cache: deps.schema_stats_cache.clone(),
        // Carried, not dropped. This struct is REBUILT from the deps the caller
        // handed us, so hard-coding `None` here discarded a resolver the caller
        // had — and every field above is cloned from `deps` precisely because
        // the caller's wiring is the answer.
        //
        // The symptom was specific and misleading: a trigger-run function
        // calling `getResource('file')` on a mounted asset got "the
        // virtual-mount sync engine is not running", on a server where it was
        // running perfectly. The message described this process's wiring, not
        // the deployment, and pointed the reader at the sync engine rather than
        // at the line that threw its resolver away.
        mount_content: deps.mount_content.clone(),
    });

    // Check if function requires admin escalation (from function metadata)
    let requires_admin = func_node
        .properties
        .get("requiresAdmin")
        .and_then(|v| match v {
            PropertyValue::Boolean(b) => Some(*b),
            _ => None,
        })
        .unwrap_or(false);

    let api_callbacks = callbacks::create_production_callbacks(
        deps_arc,
        tenant_id.to_string(),
        repo_id.to_string(),
        branch.to_string(),
        auth_context.clone(),
    );

    // 4. Create execution context
    // Parse event data from input (supports both flow_input wrapper and direct format)
    let event_data = if let Some(flow_input) = input.get("flow_input") {
        flow_input.get("event").cloned()
    } else {
        // Direct input format (backwards compatibility)
        input.get("event").cloned()
    };

    debug!(
        function_path = %function_path,
        input = %serde_json::to_string_pretty(&input).unwrap_or_else(|_| "null".to_string()),
        event_data = ?event_data,
        "Executing function"
    );

    // Determine actor from auth context or default to "system"
    let actor = auth_context
        .as_ref()
        .and_then(|a| a.user_id.as_ref())
        .map(|s| s.as_str())
        .unwrap_or("system");

    let mut exec_context = ExecutionContext::new(tenant_id, repo_id, branch, actor)
        .with_workspace(workspace)
        .with_input(input)
        .with_admin_escalation(requires_admin);

    // Set log emitter for real-time log streaming
    if let Some(emitter) = log_emitter {
        exec_context = exec_context.with_log_emitter(emitter);
    }

    // Set auth context if provided
    if let Some(auth) = auth_context {
        exec_context = exec_context.with_auth(auth);
    }

    // Populate event_data if available from input
    if let Some(event) = event_data {
        exec_context = exec_context.with_event_data(event);
    }

    // 5. Create API with callbacks
    // Use the function's network_policy from metadata instead of global config
    // The secret and email policies come from the same function node as the
    // network policy; without them the API denies every raisin.secrets.* and
    // raisin.email.* call. There is more than one site that builds this API
    // (see also the WS function and sql_query handlers) — missing one degrades
    // that transport alone, which is invisible until a function is executed
    // through it.
    let api: Arc<dyn FunctionApi> = Arc::new(
        RaisinFunctionApi::new(
            exec_context.clone(),
            metadata.network_policy.clone(),
            api_callbacks,
        )
        .with_secret_policy(metadata.secret_policy.clone())
        .with_email_policy(metadata.email_policy.clone())
        .with_identity_policy(metadata.identity_policy.clone()),
    );

    // 6. Create loaded function with sibling files for module resolution
    let loaded_function = LoadedFunction::with_files(
        metadata,
        code,
        sibling_files,
        function_path.to_string(),
        func_node.id.clone(),
        workspace.to_string(),
    );

    // 7. Execute
    let executor = FunctionExecutor::new();
    let result = executor.execute(&loaded_function, exec_context, api).await;

    // 8. Convert result
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
                execution_id: execution_id.to_string(),
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
                duration_ms = duration_ms,
                "Function execution failed"
            );
            Ok(FunctionExecutionResult {
                execution_id: execution_id.to_string(),
                success: false,
                result: None,
                error: Some(e.to_string()),
                duration_ms,
                logs: vec![format!("[error] {}", e)],
            })
        }
    }
}

/// Create the function executor callback for the job system.
///
/// The callback now accepts an optional `AuthContext` parameter to control
/// RLS filtering for API operations during function execution:
/// - `None`: Operations run without auth (system context, no RLS filtering)
/// - `Some(auth)`: Operations are filtered based on user's permissions
///
/// For AI agent tool calls, the auth context is determined by the agent's
/// `execution_context` configuration ("user" vs "system").
pub fn create_function_executor<S, B>(
    deps: Arc<ExecutionDependencies<S, B>>,
    config: FunctionExecutionConfig,
) -> raisin_rocksdb::FunctionExecutorCallback
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    Arc::new(
        move |function_path: String,
              execution_id: String,
              input: serde_json::Value,
              tenant_id: String,
              repo_id: String,
              branch: String,
              workspace: String,
              auth_context: Option<AuthContext>,
              log_emitter: Option<raisin_storage::LogEmitter>| {
            let deps = deps.clone();
            let config = config.clone();

            Box::pin(async move {
                execute_function(
                    &deps,
                    &config,
                    &function_path,
                    &execution_id,
                    input,
                    &tenant_id,
                    &repo_id,
                    &branch,
                    &workspace,
                    auth_context,
                    log_emitter,
                )
                .await
            })
        },
    )
}

/// Create the function enabled checker callback for the job system.
pub fn create_function_checker<S>(
    storage: Arc<S>,
    functions_workspace: String,
) -> raisin_rocksdb::FunctionEnabledChecker
where
    S: Storage + 'static,
{
    Arc::new(
        move |function_path: String,
              tenant_id: String,
              repo_id: String,
              branch: String,
              _workspace: String| {
            let storage = storage.clone();
            let functions_ws = functions_workspace.clone();

            Box::pin(async move {
                let func_node = storage
                    .nodes()
                    .get_by_path(
                        StorageScope::new(&tenant_id, &repo_id, &branch, &functions_ws),
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
                        .unwrap_or(true); // Default to enabled

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
}
