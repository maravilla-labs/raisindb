// SPDX-License-Identifier: BSL-1.1

//! Direct file execution handler (SSE streaming).
//!
//! Executes a standalone JavaScript/Starlark/SQL file — or a WebAssembly
//! component uploaded as a `.wasm` asset — without requiring a parent
//! `raisin:Function` node. Useful for testing individual files from the editor
//! and it is the server half of the CLI dev loop, so the SSE event shape
//! (started, log, result, done) is a contract: do not change it.

use axum::{
    extract::{Path, State},
    response::sse::{Event, KeepAlive, Sse},
    Extension, Json,
};
use chrono::Utc;
use futures::stream::Stream;
use raisin_functions::{
    language_for_file_name, ExecutionContext, FunctionCode, FunctionExecutor, FunctionLanguage,
    LoadedFunction,
};
use raisin_models::auth::AuthContext;
use std::convert::Infallible;
use std::time::Duration;

use crate::{error::ApiError, state::AppState};

use super::file_helpers::{
    build_synthetic_metadata_from_name, resolve_file_input, validate_runnable_asset,
    validate_runnable_asset_name,
};
use super::helpers::{find_asset_node_by_id, load_asset_function_code};
use super::types::{RunFileEvent, RunFileRequest};
use super::{DEFAULT_BRANCH, FUNCTIONS_WORKSPACE};
use crate::middleware::TenantInfo;

/// Run a JavaScript file directly by node ID (SSE streaming).
///
/// This endpoint executes a standalone JS file (`raisin:Asset`) without requiring
/// a parent `raisin:Function` node. Useful for testing individual files.
///
/// Returns an SSE stream with events:
/// - `started`: Execution started
/// - `log`: Each console.log/error/warn output
/// - `result`: Final execution result
/// - `done`: Stream complete
#[cfg(feature = "storage-rocksdb")]
pub async fn run_file(
    State(state): State<AppState>,
    Extension(tenant_info): Extension<TenantInfo>,
    Path(repo): Path<String>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<RunFileRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    use super::api_factory::build_function_api;
    use super::file_helpers::find_parent_function_config;

    // The file runs with the system's authority (and may be inline code the
    // caller wrote), so only the system and administrators may run one. This
    // route sits behind optional auth: without this check an anonymous caller
    // could execute arbitrary code as the system.
    require_admin_to_run(auth.as_deref())?;

    let execution_id = nanoid::nanoid!();
    let started_at = Utc::now();

    // Extract auth context for RLS filtering
    let auth_context = auth.map(|Extension(ctx)| ctx);

    // Clone what we need for the async stream
    let state_clone = state.clone();
    let tenant_clone = tenant_info.tenant_id.clone();
    let repo_clone = repo.clone();
    let auth_clone = auth_context.clone();
    let req_node_id = req.node_id.clone();
    let req_code = req.code.clone();
    let req_file_name = req.file_name.clone();
    let req_function_path = req.function_path.clone();
    let req_handler = req.handler.clone();
    let req_input = req.input.clone();
    let req_input_node_id = req.input_node_id.clone();
    let req_input_workspace = req.input_workspace.clone();
    let req_timeout = req.timeout_ms;
    let exec_id = execution_id.clone();

    let stream = async_stream::stream! {
        // Determine code source: inline code OR load from node
        // Returns (code, file_name, path, node_id, workspace)
        let code_source: (FunctionCode, String, String, String, String) = if let Some(inline_code) = req_code {
            // Use inline code directly (unsaved file case)
            let name = req_file_name.unwrap_or_else(|| "inline.js".to_string());

            // Validate it's a runnable file name
            if let Err(e) = validate_runnable_asset_name(&name) {
                yield Ok(Event::default().event("result").data(
                    serde_json::to_string(&RunFileEvent::Result {
                        execution_id: exec_id.clone(),
                        success: false,
                        result: None,
                        error: Some(e.into_message()),
                        duration_ms: 0,
                    }).unwrap_or_default()
                ));
                yield Ok(Event::default().event("done").data(
                    serde_json::to_string(&RunFileEvent::Done).unwrap_or_default()
                ));
                return;
            }

            // A component is bytes, and this field is a JSON string. There is no
            // lossless spelling of an artifact here, so say what to do instead
            // of failing later inside the wasm loader on a mangled UTF-8 copy.
            if language_for_file_name(&name) == Some(FunctionLanguage::Wasm) {
                yield Ok(Event::default().event("result").data(
                    serde_json::to_string(&RunFileEvent::Result {
                        execution_id: exec_id.clone(),
                        success: false,
                        result: None,
                        error: Some(
                            "Inline 'code' cannot carry a WebAssembly component: \
                             upload the artifact and run it by node_id"
                                .to_string(),
                        ),
                        duration_ms: 0,
                    }).unwrap_or_default()
                ));
                yield Ok(Event::default().event("done").data(
                    serde_json::to_string(&RunFileEvent::Done).unwrap_or_default()
                ));
                return;
            }

            // Synthetic values for inline execution
            let synthetic_path = format!("/_inline/{}", name);
            let synthetic_id = format!("inline-{}", exec_id);
            (
                FunctionCode::Text(inline_code),
                name,
                synthetic_path,
                synthetic_id,
                FUNCTIONS_WORKSPACE.to_string(),
            )
        } else if let Some(node_id) = req_node_id {
            // Load from saved node (existing flow)
            let asset_result = find_asset_node_by_id(&state_clone, &tenant_clone, &repo_clone, &node_id, auth_clone.as_ref()).await;
            let asset_node = match asset_result {
                Ok(node) => node,
                Err(e) => {
                    yield Ok(Event::default().event("result").data(
                        serde_json::to_string(&RunFileEvent::Result {
                            execution_id: exec_id.clone(),
                            success: false,
                            result: None,
                            error: Some(e.into_message()),
                            duration_ms: 0,
                        }).unwrap_or_default()
                    ));
                    yield Ok(Event::default().event("done").data(
                        serde_json::to_string(&RunFileEvent::Done).unwrap_or_default()
                    ));
                    return;
                }
            };

            // Validate it's a runnable file
            if let Err(e) = validate_runnable_asset(&asset_node) {
                yield Ok(Event::default().event("result").data(
                    serde_json::to_string(&RunFileEvent::Result {
                        execution_id: exec_id.clone(),
                        success: false,
                        result: None,
                        error: Some(e.into_message()),
                        duration_ms: 0,
                    }).unwrap_or_default()
                ));
                yield Ok(Event::default().event("done").data(
                    serde_json::to_string(&RunFileEvent::Done).unwrap_or_default()
                ));
                return;
            }

            // Load code from asset. Bytes for a `.wasm` component, text for
            // everything else — the language is the file name's, exactly as the
            // synthetic metadata below reads it.
            let asset_language = language_for_file_name(&asset_node.name).unwrap_or_default();
            let code_result =
                load_asset_function_code(&state_clone, &asset_node, asset_language).await;
            match code_result {
                Ok(c) => (
                    c,
                    asset_node.name.clone(),
                    asset_node.path.clone(),
                    asset_node.id.clone(),
                    asset_node.workspace.clone().unwrap_or_else(|| FUNCTIONS_WORKSPACE.into()),
                ),
                Err(e) => {
                    yield Ok(Event::default().event("result").data(
                        serde_json::to_string(&RunFileEvent::Result {
                            execution_id: exec_id.clone(),
                            success: false,
                            result: None,
                            error: Some(e.into_message()),
                            duration_ms: (Utc::now() - started_at).num_milliseconds() as u64,
                        }).unwrap_or_default()
                    ));
                    yield Ok(Event::default().event("done").data(
                        serde_json::to_string(&RunFileEvent::Done).unwrap_or_default()
                    ));
                    return;
                }
            }
        } else {
            // Neither code nor node_id provided
            yield Ok(Event::default().event("result").data(
                serde_json::to_string(&RunFileEvent::Result {
                    execution_id: exec_id.clone(),
                    success: false,
                    result: None,
                    error: Some("Either 'code' or 'node_id' must be provided".to_string()),
                    duration_ms: 0,
                }).unwrap_or_default()
            ));
            yield Ok(Event::default().event("done").data(
                serde_json::to_string(&RunFileEvent::Done).unwrap_or_default()
            ));
            return;
        };

        let (code, file_name, asset_path, asset_id, asset_workspace) = code_source;

        // Send started event
        yield Ok(Event::default().event("started").data(
            serde_json::to_string(&RunFileEvent::Started {
                execution_id: exec_id.clone(),
                file_name: file_name.clone(),
                handler: req_handler.clone(),
            }).unwrap_or_default()
        ));

        // Resolve input
        let input = resolve_file_input(&state_clone, &tenant_clone, &repo_clone, &req_input, &req_input_node_id, &req_input_workspace).await;

        // Build synthetic function metadata
        let mut metadata = build_synthetic_metadata_from_name(&file_name, &req_handler);

        // Look up parent raisin:Function node to get network_policy and resource_limits
        let lookup_path = req_function_path.as_deref().unwrap_or(&asset_path);
        if let Some((network_policy, resource_limits)) = find_parent_function_config(&state_clone, &tenant_clone, &repo_clone, lookup_path).await {
            metadata.network_policy = network_policy;
            metadata.resource_limits = resource_limits;
        }

        // The directory the file lives in IS its module root — a file run from
        // the console imports its siblings by the same `./x.js` specifiers it
        // uses in production, so resolving them against anything else would make
        // "Run" behave differently from a real invocation.
        let module_dir = asset_path
            .rsplit_once('/')
            .map(|(dir, _)| if dir.is_empty() { "/" } else { dir }.to_string());

        let mut loaded = LoadedFunction::new(
            metadata,
            code,
            asset_path,
            asset_id,
            asset_workspace,
        );

        // Without this every `import` fails at declare time with
        // `Error resolving module '…' from 'entry'`, because the QuickJS loader
        // is rebuilt per execution against `files` and an empty map resolves
        // nothing. Skipped for an inline-code run with no path to scan, and for
        // a component: its imports were linked when it was built, so the scan
        // could only ever return files the runtime cannot use — the same
        // reasoning (and the same skip) as `execution/executor.rs`.
        let is_wasm = loaded.metadata.language == FunctionLanguage::Wasm;
        if let Some(dir) = module_dir.filter(|_| !is_wasm) {
            loaded.files = super::load_function_modules_on_branch(
                &state_clone,
                &tenant_clone,
                &repo_clone,
                DEFAULT_BRANCH,
                &dir,
                &file_name,
                loaded.code.as_text().unwrap_or(""),
            )
            .await;
        }

        // Apply timeout override
        if let Some(timeout) = req_timeout {
            loaded.metadata.resource_limits.timeout_ms = timeout;
        }

        // Execute the function
        let context = ExecutionContext::new(&tenant_clone, &repo_clone, DEFAULT_BRANCH, "system")
            .with_workspace(FUNCTIONS_WORKSPACE)
            .with_input(input);

        eprintln!(
            "[DEBUG] run_file - passing network_policy to build_function_api: http_enabled={}, allowed_urls={:?}",
            loaded.metadata.network_policy.http_enabled,
            loaded.metadata.network_policy.allowed_urls
        );
        let api = build_function_api(
            &state_clone,
            &tenant_clone,
            &repo_clone,
            &loaded.metadata,
            None,
        );
        let executor = FunctionExecutor::new();

        let exec_result = executor.execute(&loaded, context.clone(), api.clone()).await;

        match exec_result {
            Ok(result) => {
                // Stream logs
                for log_entry in &result.logs {
                    yield Ok(Event::default().event("log").data(
                        serde_json::to_string(&RunFileEvent::Log {
                            level: log_entry.level.to_string(),
                            message: log_entry.message.clone(),
                            timestamp: log_entry.timestamp.to_rfc3339(),
                        }).unwrap_or_default()
                    ));
                }

                // Also include API logs
                for log_entry in api.get_logs() {
                    yield Ok(Event::default().event("log").data(
                        serde_json::to_string(&RunFileEvent::Log {
                            level: log_entry.level.to_string(),
                            message: log_entry.message.clone(),
                            timestamp: log_entry.timestamp.to_rfc3339(),
                        }).unwrap_or_default()
                    ));
                }

                // Send result
                yield Ok(Event::default().event("result").data(
                    serde_json::to_string(&RunFileEvent::Result {
                        execution_id: exec_id.clone(),
                        success: result.success,
                        result: result.output.clone(),
                        error: result.error.map(|e| format!("{}", e)),
                        duration_ms: result.stats.duration_ms,
                    }).unwrap_or_default()
                ));
            }
            Err(e) => {
                yield Ok(Event::default().event("result").data(
                    serde_json::to_string(&RunFileEvent::Result {
                        execution_id: exec_id.clone(),
                        success: false,
                        result: None,
                        error: Some(e.to_string()),
                        duration_ms: (Utc::now() - started_at).num_milliseconds() as u64,
                    }).unwrap_or_default()
                ));
            }
        }

        // Send done
        yield Ok(Event::default().event("done").data(
            serde_json::to_string(&RunFileEvent::Done).unwrap_or_default()
        ));
    };

    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

/// The caller may run a file: the system (admin console, CLI, API keys and the
/// superadmin bearer all resolve to it) or a `system_admin` identity user.
#[cfg(feature = "storage-rocksdb")]
pub(crate) fn require_admin_to_run(auth: Option<&AuthContext>) -> Result<(), ApiError> {
    let is_admin =
        auth.is_some_and(|a| a.is_system || a.permissions().is_some_and(|p| p.is_system_admin));
    if is_admin {
        Ok(())
    } else {
        Err(ApiError::new(
            axum::http::StatusCode::FORBIDDEN,
            "FORBIDDEN",
            "running a file requires an administrator",
        ))
    }
}

#[cfg(all(test, feature = "storage-rocksdb"))]
mod run_gate_tests {
    use super::require_admin_to_run;
    use raisin_models::auth::AuthContext;
    use raisin_models::permissions::ResolvedPermissions;

    #[test]
    fn only_the_system_and_administrators_may_run_a_file() {
        // Admin console, CLI, API keys, superadmin bearer: all the system.
        assert!(require_admin_to_run(Some(&AuthContext::system())).is_ok());
        let admin =
            AuthContext::for_user("root").with_permissions(ResolvedPermissions::system_admin());
        assert!(require_admin_to_run(Some(&admin)).is_ok());

        let user = AuthContext::for_user("bob").with_permissions(ResolvedPermissions::empty("bob"));
        assert!(require_admin_to_run(Some(&user)).is_err());
        let anon = AuthContext::anonymous_user("anon-node")
            .with_permissions(ResolvedPermissions::anonymous(vec![]));
        assert!(require_admin_to_run(Some(&anon)).is_err());
        assert!(require_admin_to_run(Some(&AuthContext::anonymous())).is_err());
        assert!(require_admin_to_run(None).is_err());
        // An admin impersonating a user acts as that user.
        let impersonated = AuthContext::impersonated("bob", "root")
            .with_permissions(ResolvedPermissions::empty("bob"));
        assert!(require_admin_to_run(Some(&impersonated)).is_err());
    }
}

/// Stub `run_file` without RocksDB.
#[cfg(not(feature = "storage-rocksdb"))]
pub async fn run_file(
    State(_state): State<AppState>,
    Path(_repo): Path<String>,
    Json(_req): Json<RunFileRequest>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = async_stream::stream! {
        yield Ok(Event::default().event("result").data(
            serde_json::to_string(&RunFileEvent::Result {
                execution_id: "none".into(),
                success: false,
                result: None,
                error: Some("File execution requires RocksDB backend".into()),
                duration_ms: 0,
            }).unwrap_or_default()
        ));
        yield Ok(Event::default().event("done").data(
            serde_json::to_string(&RunFileEvent::Done).unwrap_or_default()
        ));
    };

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}
