// SPDX-License-Identifier: BSL-1.1

//! Synchronous, inline invocation of a connector's adapter function.
//!
//! Shared by every endpoint that needs to ask a provider something *right now*,
//! outside the sync engine: `POST …/test` (the connection probe) and
//! `POST …/browse` (remote discovery for the mount editor). The adapter runs
//! directly via [`FunctionExecutor`] — the same way `handlers::functions::invoke`
//! runs a synchronous function — and **never** enqueues a job.
//!
//! This lives in one module on purpose. Both callers must apply the identical
//! network policy, system-auth escalation and error classification; two copies
//! of "invoke an adapter over HTTP" is precisely the mirrored-code drift that
//! keeps producing bugs in this codebase (see CLAUDE.md). Add a third caller by
//! calling [`invoke_adapter`], not by copying it.
//!
//! Security invariants inherited by every caller: no `access_token`,
//! `refresh_token` or `client_secret` may appear in a response body or log line,
//! and a hard runtime failure is returned as [`AdapterResult::Failed`] — a
//! diagnostic result — rather than a 500.

#![cfg(feature = "storage-rocksdb")]

use raisin_models::nodes::Node;
use serde_json::{json, Value};

use crate::error::ApiError;
use crate::state::AppState;

/// Branch adapter functions are resolved on.
pub(super) const FUNCTIONS_BRANCH: &str = "main";
/// Workspace adapter functions live in.
pub(super) const FUNCTIONS_WORKSPACE: &str = "functions";

/// Outcome of one adapter invocation. A provider-side failure is data, not an
/// error: callers turn it into a structured diagnostic.
pub(super) enum AdapterResult {
    Ok(Value),
    Failed(String),
}

/// Invoke one adapter operation synchronously, inline (never a job).
#[allow(clippy::too_many_arguments)]
pub(super) async fn invoke_adapter(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    node: &Node,
    operation: &str,
    params: Value,
    credential: &Option<Value>,
    mount: &Value,
) -> Result<AdapterResult, ApiError> {
    use crate::handlers::functions::api_factory::build_function_api;
    use crate::handlers::functions::helpers::{
        build_loaded_function, load_function_code, load_function_modules_on_branch,
    };
    use raisin_functions::{ExecutionContext, FunctionExecutor};

    let input = json!({
        "operation": operation,
        "params": params,
        "credential": credential.clone().unwrap_or(Value::Null),
        "mount": mount.clone(),
    });

    let code = load_function_code(state, tenant_id, repo, node).await?;
    let mut loaded = build_loaded_function(node, code)?;

    // The adapter's importable modules. `LoadedFunction::files` is what the
    // QuickJS resolver consults, and the loader is rebuilt per execution for
    // tenant isolation — so an empty map rejects EVERY import, and an adapter
    // split across sibling files fails here with
    // `Error resolving module './x.js' from 'entry'`.
    //
    // The sync engine fills this in through `code_loader` and was never
    // affected, which is what made the symptom so confusing: a mount synced
    // perfectly while Test connection on the same connector failed outright.
    loaded.files = load_function_modules_on_branch(
        state,
        tenant_id,
        repo,
        FUNCTIONS_BRANCH,
        &node.path,
        loaded.metadata.entry_file_path(),
        &loaded.code,
    )
    .await;

    let context = ExecutionContext::new(tenant_id, repo, FUNCTIONS_BRANCH, "system")
        .with_workspace(FUNCTIONS_WORKSPACE)
        .with_input(input)
        .with_admin_escalation(true);

    let api = build_function_api(state, tenant_id, repo, &loaded.metadata, None);
    let result = FunctionExecutor::new()
        .execute(&loaded, context, api)
        .await
        // A hard runtime failure (code load, syntax error) is a diagnostic
        // result, not a 500 — surface it as a failed invocation.
        .map_err(|e| e.to_string());

    match result {
        Ok(exec) if exec.success => Ok(AdapterResult::Ok(exec.output.unwrap_or(Value::Null))),
        Ok(exec) => Ok(AdapterResult::Failed(
            exec.error
                .map(|e| format!("{e}"))
                .unwrap_or_else(|| "adapter returned failure".to_string()),
        )),
        Err(msg) => Ok(AdapterResult::Failed(format!("invocation_failed: {msg}"))),
    }
}

/// Load the adapter function node, by path first then by name.
pub(super) async fn load_adapter_node(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    adapter_path: &str,
    actor: &str,
) -> Result<Option<Node>, ApiError> {
    let svc = state.node_service_for_context(
        tenant_id,
        repo,
        FUNCTIONS_BRANCH,
        FUNCTIONS_WORKSPACE,
        Some(system_auth(actor)),
    );
    if let Some(node) = svc.get_by_path(adapter_path).await? {
        return Ok(Some(node));
    }
    // Fallback: match on the trailing function name.
    let name = adapter_path.rsplit('/').next().unwrap_or(adapter_path);
    let nodes = svc.list_by_type("raisin:Function").await?;
    Ok(nodes.into_iter().find(|n| {
        n.name == name || string_prop(n, "name").as_deref() == Some(name) || n.path == adapter_path
    }))
}

/// System auth context for privileged config/function reads. `actor` names the
/// endpoint so the read is attributable.
pub(super) fn system_auth(actor: &str) -> raisin_models::auth::AuthContext {
    let mut auth = raisin_models::auth::AuthContext::system();
    auth.user_id = Some(actor.to_string());
    auth
}

/// Read a non-empty string property.
fn string_prop(node: &Node, key: &str) -> Option<String> {
    use raisin_models::nodes::properties::PropertyValue;
    match node.properties.get(key)? {
        PropertyValue::String(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}
