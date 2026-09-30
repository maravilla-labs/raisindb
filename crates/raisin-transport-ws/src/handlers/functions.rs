// SPDX-License-Identifier: BSL-1.1

//! Function invocation handler for WebSocket transport.
//!
//! Provides two modes of function execution:
//! - **Async** (`FunctionInvoke`): Registers a `FunctionExecution` job via the
//!   RocksDB job registry and returns `{ execution_id, job_id }` immediately.
//! - **Sync** (`FunctionInvokeSync`): Executes the function inline within the
//!   WS request handler and returns `{ execution_id, result, ... }` directly.

use parking_lot::RwLock;
use serde::Deserialize;
use std::sync::Arc;

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{RequestEnvelope, ResponseEnvelope},
};

// ---------------------------------------------------------------------------
// Payload types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct FunctionInvokePayload {
    function_name: String,
    #[serde(default)]
    input: serde_json::Value,
    #[serde(default)]
    wait_for_completion: bool,
    #[serde(default)]
    wait_timeout_ms: Option<u64>,
}

// ---------------------------------------------------------------------------
// RocksDB-backed implementation
// ---------------------------------------------------------------------------

#[cfg(feature = "storage-rocksdb")]
mod inner {
    use super::*;
    use raisin_storage::{jobs::JobStatus, Storage};
    use std::collections::HashMap;
    use std::time::Duration;

    const DEFAULT_BRANCH: &str = "main";
    const FUNCTIONS_WORKSPACE: &str = "functions";

    /// Require `context.repository` from the request.
    fn require_repo(request: &RequestEnvelope) -> Result<String, WsError> {
        request
            .context
            .repository
            .clone()
            .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))
    }

    /// The connection's tenant — established at upgrade time from the /sys
    /// path or x-tenant-id header (never from message content). Function
    /// invocation must resolve/execute in the caller's actual tenant, not a
    /// hardcoded one, or every prod invoke silently runs against "default".
    fn connection_tenant(connection_state: &Arc<RwLock<ConnectionState>>) -> String {
        connection_state.read().tenant_id.clone()
    }

    /// The branch an invocation runs against: the request context's, else
    /// `main`. Both invoke handlers ignored it and ran on `main`, so a client
    /// reading its publish branch got functions answering from working content.
    fn request_branch(request: &RequestEnvelope) -> String {
        request
            .context
            .branch
            .clone()
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| DEFAULT_BRANCH.to_string())
    }

    /// The function node for `branch`: on that branch, else on `main`, where
    /// functions are deployed. Returns the node and the branch it came from,
    /// which is where its code is loaded from.
    async fn find_function_for_branch<S: Storage>(
        storage: &S,
        tenant_id: &str,
        repo: &str,
        branch: &str,
        name: &str,
    ) -> Result<(raisin_models::nodes::Node, String), WsError> {
        if branch != DEFAULT_BRANCH {
            if let Ok(node) = raisin_functions::execution::code_loader::find_function(
                storage,
                tenant_id,
                repo,
                branch,
                FUNCTIONS_WORKSPACE,
                name,
            )
            .await
            {
                return Ok((node, branch.to_string()));
            }
        }
        let node = raisin_functions::execution::code_loader::find_function(
            storage,
            tenant_id,
            repo,
            DEFAULT_BRANCH,
            FUNCTIONS_WORKSPACE,
            name,
        )
        .await
        .map_err(|e| WsError::InvalidRequest(e.to_string()))?;
        Ok((node, DEFAULT_BRANCH.to_string()))
    }

    /// Refuse a client invoke the caller may not make: invoking needs
    /// `execute` on the function node (see
    /// `raisin_core::services::function_invoke_access`). Decided from the
    /// node, before any code loads.
    fn authorize_client_invoke(
        connection_state: &Arc<RwLock<ConnectionState>>,
        function_node: &raisin_models::nodes::Node,
        branch: &str,
    ) -> Result<(), WsError> {
        let auth = connection_state.read().auth_context().cloned();
        raisin_core::services::function_invoke_access::authorize_invoke(
            auth.as_ref(),
            function_node,
            branch,
        )
        .map_err(|refusal| {
            tracing::debug!(
                function = %function_node.path,
                reason = %refusal,
                "function invoke refused"
            );
            WsError::PermissionDenied
        })
    }

    // -----------------------------------------------------------------------
    // Async invoke (background job)
    // -----------------------------------------------------------------------

    pub async fn handle_function_invoke<S, B>(
        state: &Arc<WsState<S, B>>,
        connection_state: &Arc<RwLock<ConnectionState>>,
        request: RequestEnvelope,
    ) -> Result<Option<ResponseEnvelope>, WsError>
    where
        S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
        B: raisin_binary::BinaryStorage + 'static,
    {
        let payload: FunctionInvokePayload = serde_json::from_value(request.payload.clone())?;
        let repo = require_repo(&request)?;
        let tenant_id = connection_tenant(connection_state);

        let rocksdb = state
            .rocksdb_storage
            .as_ref()
            .ok_or_else(|| WsError::InternalError("RocksDB storage not available".to_string()))?
            .clone();

        let branch = request_branch(&request);
        let (function_node, code_branch) = find_function_for_branch(
            &*state.storage,
            &tenant_id,
            &repo,
            &branch,
            &payload.function_name,
        )
        .await?;
        authorize_client_invoke(connection_state, &function_node, &code_branch)?;
        // A job loads the function from the branch it runs on.
        if code_branch != branch {
            return Err(WsError::InvalidRequest(format!(
                "Function '{}' does not exist on branch '{branch}'; an asynchronous \
                 invocation loads it from the branch it runs on. Use invokeSync, or make \
                 the function available on that branch.",
                payload.function_name
            )));
        }

        // Register a background job for execution
        let execution_id = nanoid::nanoid!();
        let job_type = raisin_storage::jobs::JobType::FunctionExecution {
            function_path: function_node.path.clone(),
            trigger_name: Some("ws".into()),
            execution_id: execution_id.clone(),
        };

        let mut metadata = HashMap::new();
        metadata.insert("input".to_string(), payload.input);

        // Persist the connection's auth context so the async job executes as
        // the invoking caller, matching the sync WS path and HTTP's async
        // invoke. The job handler drops this unless the function's declared
        // `execution_context` is "user" (the default) — see
        // `function_execution.rs`.
        if let Some(auth) = connection_state.read().auth_context() {
            if let Ok(serialized) = serde_json::to_value(auth) {
                metadata.insert("auth_context".to_string(), serialized);
            }
        }

        let context = raisin_storage::jobs::JobContext {
            tenant_id: tenant_id.clone(),
            repo_id: repo.clone(),
            branch: branch.clone(),
            workspace_id: FUNCTIONS_WORKSPACE.into(),
            revision: raisin_hlc::HLC::new(0, 0),
            metadata,
        };

        // Store job context BEFORE registering so dispatch can never
        // observe the job without its context.
        let job_id = raisin_storage::jobs::JobId::new();
        rocksdb
            .job_data_store()
            .put(&job_id, &context)
            .map_err(|e| WsError::InternalError(e.to_string()))?;

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
            .map_err(|e| WsError::StorageError(e.to_string()))?;

        tracing::info!(
            job_id = %job_id,
            execution_id = %execution_id,
            function = %payload.function_name,
            "Queued function execution via WS"
        );

        if payload.wait_for_completion {
            let wait_timeout_ms = payload
                .wait_timeout_ms
                .unwrap_or(60_000)
                .clamp(1_000, 300_000);
            let waited = wait_for_job_terminal_state(&rocksdb, &job_id, wait_timeout_ms).await?;

            return Ok(Some(ResponseEnvelope::success(
                request.request_id,
                match waited {
                    WaitedJob::Completed(job_info) => {
                        let (result, error, duration_ms, logs) = extract_result_fields(&job_info);
                        serde_json::json!({
                            "execution_id": execution_id,
                            "job_id": job_id.to_string(),
                            "status": job_status_to_string(&job_info.status),
                            "completed": true,
                            "timed_out": false,
                            "waited": true,
                            "result": result,
                            "error": error,
                            "duration_ms": duration_ms,
                            "logs": logs
                        })
                    }
                    WaitedJob::TimedOut => serde_json::json!({
                        "execution_id": execution_id,
                        "job_id": job_id.to_string(),
                        "status": "running",
                        "completed": false,
                        "timed_out": true,
                        "waited": true
                    }),
                },
            )));
        }

        Ok(Some(ResponseEnvelope::success(
            request.request_id,
            serde_json::json!({
                "execution_id": execution_id,
                "job_id": job_id.to_string(),
                "status": "scheduled",
                "completed": false,
                "timed_out": false,
                "waited": false
            }),
        )))
    }

    enum WaitedJob {
        Completed(raisin_storage::jobs::JobInfo),
        TimedOut,
    }

    async fn wait_for_job_terminal_state(
        rocksdb: &Arc<raisin_rocksdb::RocksDBStorage>,
        job_id: &raisin_storage::jobs::JobId,
        wait_timeout_ms: u64,
    ) -> Result<WaitedJob, WsError> {
        let poll = async {
            loop {
                let info = rocksdb
                    .job_registry()
                    .get_job_info(job_id)
                    .await
                    .map_err(|e| WsError::StorageError(e.to_string()))?;

                if !matches!(
                    info.status,
                    JobStatus::Running | JobStatus::Executing | JobStatus::Scheduled
                ) {
                    return Ok::<WaitedJob, WsError>(WaitedJob::Completed(info));
                }

                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        };

        match tokio::time::timeout(Duration::from_millis(wait_timeout_ms), poll).await {
            Ok(result) => result,
            Err(_) => Ok(WaitedJob::TimedOut),
        }
    }

    fn job_status_to_string(status: &JobStatus) -> &'static str {
        match status {
            JobStatus::Scheduled => "scheduled",
            JobStatus::Running | JobStatus::Executing => "running",
            JobStatus::Completed => "completed",
            JobStatus::Cancelled => "cancelled",
            JobStatus::Failed(_) => "failed",
        }
    }

    fn extract_result_fields(
        job_info: &raisin_storage::jobs::JobInfo,
    ) -> (
        Option<serde_json::Value>,
        Option<String>,
        Option<u64>,
        Option<Vec<String>>,
    ) {
        let mut result = None;
        let mut error = job_info.error.clone();
        let mut duration_ms = None;
        let mut logs = None;

        if let Some(payload) = &job_info.result {
            if let Some(obj) = payload.as_object() {
                result = obj.get("result").cloned();
                if error.is_none() {
                    if let Some(err) = obj.get("error").and_then(|v| v.as_str()) {
                        error = Some(err.to_string());
                    } else if obj.get("success").and_then(|v| v.as_bool()) == Some(false) {
                        error = Some("Function execution failed".to_string());
                    }
                }
                duration_ms = obj.get("duration_ms").and_then(|v| v.as_u64());
                logs = obj.get("logs").and_then(|v| {
                    v.as_array().map(|items| {
                        items
                            .iter()
                            .filter_map(|entry| entry.as_str().map(ToString::to_string))
                            .collect::<Vec<String>>()
                    })
                });
            } else {
                result = Some(payload.clone());
            }
        }

        if error.is_none() {
            if let JobStatus::Failed(msg) = &job_info.status {
                error = Some(msg.clone());
            }
        }

        (result, error, duration_ms, logs)
    }

    // -----------------------------------------------------------------------
    // Sync invoke (inline execution)
    // -----------------------------------------------------------------------

    pub async fn handle_function_invoke_sync<S, B>(
        state: &Arc<WsState<S, B>>,
        connection_state: &Arc<RwLock<ConnectionState>>,
        request: RequestEnvelope,
    ) -> Result<Option<ResponseEnvelope>, WsError>
    where
        S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
        B: raisin_binary::BinaryStorage + 'static,
    {
        use raisin_functions::{
            execution::callbacks::create_production_callbacks, execution::ExecutionDependencies,
            ExecutionContext, FunctionExecutor, RaisinFunctionApi,
        };

        let payload: FunctionInvokePayload = serde_json::from_value(request.payload.clone())?;
        let repo = require_repo(&request)?;
        let tenant_id = connection_tenant(connection_state);
        let request_id = request.request_id.clone();

        // Find function via canonical code_loader
        let branch = request_branch(&request);
        let (function_node, code_branch) = find_function_for_branch(
            &*state.storage,
            &tenant_id,
            &repo,
            &branch,
            &payload.function_name,
        )
        .await?;
        authorize_client_invoke(connection_state, &function_node, &code_branch)?;

        // Load function code via canonical code_loader (resolves entry_file property)
        let (code, metadata) = raisin_functions::execution::code_loader::load_function_code(
            &*state.storage,
            &*state.bin,
            &tenant_id,
            &repo,
            &code_branch,
            FUNCTIONS_WORKSPACE,
            &function_node,
            &function_node.path,
        )
        .await
        .map_err(|e| WsError::InternalError(format!("Failed to load function code: {}", e)))?;

        let mut loaded = raisin_functions::LoadedFunction::new(
            metadata.clone(),
            code,
            function_node.path.clone(),
            function_node.id.clone(),
            function_node
                .workspace
                .clone()
                .unwrap_or_else(|| FUNCTIONS_WORKSPACE.into()),
        );

        // Without this every `import` in the entry file fails at declare time —
        // see `super::function_modules`.
        loaded.files = crate::handlers::function_modules::load_function_modules(
            &*state.storage,
            &*state.bin,
            &tenant_id,
            &repo,
            &code_branch,
            FUNCTIONS_WORKSPACE,
            &function_node.path,
            loaded.metadata.entry_file_path(),
            loaded.code.as_text().unwrap_or(""),
        )
        .await;

        // Build ExecutionDependencies
        let deps = Arc::new(ExecutionDependencies {
            storage: state.storage.clone(),
            // This path is generic over the storage backend, so it cannot reach
            // the RocksDB-specific sync engine. A function invoked here that reads
            // a mounted asset's expired bytes is told the fetch is unavailable
            // rather than seeing a missing file. The paths that matter for that —
            // background jobs and HTTP invoke — do supply it.
            mount_content: None,
            binary_storage: state.bin.clone(),
            indexing_engine: state.indexing_engine.clone(),
            hnsw_engine: state.hnsw_engine.clone(),
            http_client: raisin_functions::shared_http_client(),
            // WIRED, and it has to be: this is the path `invokeSync` takes from
            // the browser SDK, so leaving it None meant NO function invoked over
            // the WebSocket could use AI at all — `raisin.ai.completion` failed
            // with "AI operations not configured" however well the tenant was
            // set up, while the identical function invoked over HTTP or from a
            // job worked. That asymmetry is invisible from the outside: the
            // error names configuration, so it sends you to the admin console
            // for a provider that is already there.
            //
            // Same shape as job_registry / secret_store below — the accessor
            // lives on the concrete RocksDB storage, so it comes from the
            // feature-gated handle rather than the generic one. A `match`
            // rather than `.map()` so the concrete repository coerces to the
            // trait object at the field, which is what lets this stay out of
            // the crate's dependency list.
            ai_config_store: match state.rocksdb_storage.as_ref() {
                Some(s) => Some(Arc::new(s.tenant_ai_config_repository())),
                None => None,
            },
            // Same job-system deps as job-driven executions — keeps the callback
            // surface (functions.execute / flows.run / scheduler.*) uniform.
            // The accessors live on the concrete RocksDB storage, so wire them
            // from the feature-gated handle (same pattern as the HTTP transport).
            job_registry: state
                .rocksdb_storage
                .as_ref()
                .map(|s| s.job_registry().clone()),
            job_data_store: state
                .rocksdb_storage
                .as_ref()
                .map(|s| s.job_data_store().clone()),
            lock_manager: state.lock_manager.clone(),
            // Like the job-system deps above, the accessor lives on the
            // concrete RocksDB storage. None when no master keyring is
            // configured; a present store is not a grant, the function's own
            // SecretPolicy still gates every call.
            secret_store: state
                .rocksdb_storage
                .as_ref()
                .and_then(|s| s.secret_store().ok()),
            identity_repo: state
                .rocksdb_storage
                .as_ref()
                .map(|s| Arc::new(s.identity_repository())),
            schema_stats_cache: state.schema_stats_cache.clone(),
        });

        // Enforce the function's declared `execution_context`, same as the HTTP
        // and MCP invocation paths funneled through `execute_function`: "system"
        // is an explicit opt-in and strips the connection's identity (keeping
        // its `agent` marker, if any, for attribution); "user" (the default)
        // runs as the WS connection's own resolved identity, or as nobody if
        // the connection never authenticated — never silently as "system".
        // Who asked, before "system" replaces the identity the function runs as.
        let caller = raisin_functions::types::FunctionCaller::from_auth(
            connection_state.read().auth_context(),
        );
        let auth_context = match metadata.execution_context {
            raisin_functions::types::FunctionExecutionContext::System => {
                let system = raisin_models::auth::AuthContext::system();
                Some(
                    match connection_state
                        .read()
                        .auth_context()
                        .and_then(|a| a.agent.clone())
                    {
                        Some(agent) => system.with_agent(agent),
                        None => system,
                    },
                )
            }
            raisin_functions::types::FunctionExecutionContext::User => {
                connection_state.read().auth_context().cloned()
            }
        };
        let actor = auth_context
            .as_ref()
            .and_then(|a| a.user_id.as_ref())
            .map(|s| s.as_str())
            .unwrap_or("system");

        // Build callbacks via canonical create_production_callbacks
        let callbacks = create_production_callbacks(
            deps,
            tenant_id.clone(),
            repo.clone(),
            branch.clone(),
            auth_context.clone(),
        );

        let mut api_context = ExecutionContext::new(&tenant_id, &repo, &branch, actor)
            .with_workspace(FUNCTIONS_WORKSPACE)
            .with_caller(Some(caller));
        if let Some(auth) = auth_context.clone() {
            api_context = api_context.with_auth(auth);
        }

        let api = Arc::new(
            RaisinFunctionApi::new(api_context, metadata.network_policy.clone(), callbacks)
                .with_secret_policy(metadata.secret_policy.clone())
                .with_email_policy(metadata.email_policy.clone())
                .with_identity_policy(metadata.identity_policy.clone()),
        );

        let mut context = ExecutionContext::new(&tenant_id, &repo, &branch, actor)
            .with_workspace(FUNCTIONS_WORKSPACE)
            .with_input(payload.input);
        if let Some(auth) = auth_context {
            context = context.with_auth(auth);
        }

        let executor = FunctionExecutor::new();
        let result = executor
            .execute(&loaded, context.clone(), api.clone())
            .await
            .map_err(|e| WsError::InternalError(e.to_string()))?;

        let logs: Vec<String> = result
            .logs
            .iter()
            .map(|entry| format!("[{}] {}", entry.level, entry.message))
            .chain(
                api.get_logs()
                    .into_iter()
                    .map(|entry| format!("[{}] {}", entry.level, entry.message)),
            )
            .collect();

        tracing::info!(
            execution_id = %context.execution_id,
            function = %payload.function_name,
            duration_ms = %result.stats.duration_ms,
            success = %result.success,
            "Executed function inline via WS"
        );

        Ok(Some(ResponseEnvelope::success(
            request_id,
            serde_json::json!({
                "execution_id": context.execution_id,
                "result": result.output,
                "error": result.error.map(|e| format!("{}", e)),
                "duration_ms": result.stats.duration_ms,
                "logs": logs,
            }),
        )))
    }
}

// ---------------------------------------------------------------------------
// Feature-gated re-exports / fallback stubs
// ---------------------------------------------------------------------------

#[cfg(feature = "storage-rocksdb")]
pub use inner::handle_function_invoke;

#[cfg(feature = "storage-rocksdb")]
pub use inner::handle_function_invoke_sync;

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn handle_function_invoke<S, B>(
    _state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + raisin_storage::transactional::TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    Ok(Some(ResponseEnvelope::error(
        request.request_id,
        "NOT_IMPLEMENTED".to_string(),
        "Function invocation requires RocksDB backend".to_string(),
    )))
}

#[cfg(not(feature = "storage-rocksdb"))]
pub async fn handle_function_invoke_sync<S, B>(
    _state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + raisin_storage::transactional::TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    Ok(Some(ResponseEnvelope::error(
        request.request_id,
        "NOT_IMPLEMENTED".to_string(),
        "Function invocation requires RocksDB backend".to_string(),
    )))
}

/// Client invokes over the socket, against a real RocksDB state: invoking
/// needs `execute` on the function node, decided before any code loads.
#[cfg(all(test, feature = "storage-rocksdb"))]
mod invoke_gate_tests {
    use super::*;
    use raisin_models::auth::AuthContext;
    use raisin_models::nodes::properties::PropertyValue;
    use raisin_models::nodes::Node;
    use raisin_models::permissions::{Operation, Permission, ResolvedPermissions};
    use raisin_storage::{
        BranchRepository, CreateNodeOptions, NodeRepository, Storage, StorageScope,
    };

    const TENANT: &str = "t_invoke_gate";
    const REPO: &str = "r_invoke_gate";

    type St = raisin_rocksdb::RocksDBStorage;
    type Bn = raisin_binary::FilesystemBinaryStorage;

    async fn state(dir: &std::path::Path) -> Arc<WsState<St, Bn>> {
        let storage = Arc::new(St::new(dir.join("db")).unwrap());
        let _ = storage
            .branches()
            .create_branch(TENANT, REPO, "main", "test", None, None, false, false)
            .await;
        // A system function under /lib/studio, and a public one.
        for (path, system) in [("/studio-fn", true), ("/public-fn", false)] {
            let mut node = Node {
                id: format!("fn{path}"),
                name: path.trim_start_matches('/').to_string(),
                path: path.to_string(),
                node_type: "raisin:Function".to_string(),
                ..Default::default()
            };
            node.properties.insert(
                "language".into(),
                PropertyValue::String("javascript".into()),
            );
            if system {
                node.properties.insert(
                    "execution_context".into(),
                    PropertyValue::String("system".into()),
                );
            }
            storage
                .nodes()
                .create(
                    StorageScope::new(TENANT, REPO, "main", "functions"),
                    node,
                    CreateNodeOptions {
                        validate_schema: false,
                        validate_parent_allows_child: false,
                        validate_workspace_allows_type: false,
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
        }
        let audit = Arc::new(storage.audit_repository());
        Arc::new(WsState::new(
            storage.clone(),
            Arc::new(raisin_core::RaisinConnection::with_storage(storage.clone())),
            Arc::new(raisin_core::WorkspaceService::new(storage.clone())),
            Arc::new(Bn::new(dir.join("bin"), Some("/files".into()))),
            crate::handler::WsConfig::default(),
            None,
            Some(storage.clone()),
            None,
            None,
            None,
            None,
            audit,
        ))
    }

    fn conn(auth: Option<AuthContext>) -> Arc<RwLock<ConnectionState>> {
        let mut c = ConnectionState::new(TENANT.to_string(), Some(REPO.to_string()), 4, 100);
        if let Some(a) = auth {
            c.set_auth_context(a);
        }
        Arc::new(RwLock::new(c))
    }

    fn request(function: &str) -> RequestEnvelope {
        serde_json::from_value(serde_json::json!({
            "request_id": "r1",
            "type": "function_invoke_sync",
            "context": { "tenant_id": TENANT, "repository": REPO },
            "payload": { "function_name": format!("/{function}"), "input": {} }
        }))
        .unwrap()
    }

    fn grant(path: &str, op: Operation) -> Permission {
        Permission::new(path, vec![op]).with_workspace("functions")
    }

    fn user(grants: Vec<Permission>) -> AuthContext {
        let mut p = ResolvedPermissions::empty("u1");
        p.permissions = grants;
        AuthContext::for_user("u1").with_permissions(p)
    }

    fn anonymous(grants: Vec<Permission>) -> AuthContext {
        AuthContext::anonymous_user("anon").with_permissions(ResolvedPermissions::anonymous(grants))
    }

    /// `true` when the gate refused; any later failure (the probe nodes carry
    /// no code) means the gate let the call through.
    async fn refused(state: &Arc<WsState<St, Bn>>, auth: Option<AuthContext>, f: &str) -> bool {
        matches!(
            handle_function_invoke_sync(state, &conn(auth), request(f)).await,
            Err(WsError::PermissionDenied)
        )
    }

    #[tokio::test]
    async fn socket_invokes_need_execute_on_the_function() {
        let dir = tempfile::tempdir().unwrap();
        let s = state(dir.path()).await;

        // Execute granted: runs. Only where granted.
        let runner = user(vec![grant("/studio-fn", Operation::Execute)]);
        assert!(!refused(&s, Some(runner.clone()), "studio-fn").await);
        assert!(refused(&s, Some(runner), "public-fn").await);

        // Read-only: refused. A signed-in caller with nothing: refused.
        assert!(
            refused(
                &s,
                Some(user(vec![grant("/**", Operation::Read)])),
                "studio-fn"
            )
            .await
        );
        assert!(refused(&s, Some(user(vec![])), "studio-fn").await);

        // Anonymous: refused without a grant, runs with one.
        for who in [None, Some(anonymous(vec![]))] {
            assert!(refused(&s, who.clone(), "studio-fn").await);
            assert!(refused(&s, who, "public-fn").await);
        }
        let public = anonymous(vec![grant("/public-fn", Operation::Execute)]);
        assert!(!refused(&s, Some(public.clone()), "public-fn").await);
        assert!(refused(&s, Some(public), "studio-fn").await);

        // Administrators and the system: everything.
        let admin =
            AuthContext::for_user("root").with_permissions(ResolvedPermissions::system_admin());
        for who in [admin, AuthContext::system()] {
            assert!(!refused(&s, Some(who.clone()), "studio-fn").await);
            assert!(!refused(&s, Some(who), "public-fn").await);
        }
    }
}
