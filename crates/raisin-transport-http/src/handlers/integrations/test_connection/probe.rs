// SPDX-License-Identifier: BSL-1.1

//! Synchronous, inline adapter invocation for the connection test. Never
//! enqueues a job — the adapter runs directly via [`FunctionExecutor`], the same
//! way `handlers::functions::invoke` runs a synchronous function.

#![cfg(feature = "storage-rocksdb")]

use raisin_models::nodes::Node;
use serde_json::{json, Value};

use super::support::{
    adapter_error_code, error_is_auth_expired, probe_from_list, sanitize, system_auth,
};
use super::{Capabilities, ProbeOutcome, FUNCTIONS_BRANCH, FUNCTIONS_WORKSPACE, PROBE_LIMIT};
use crate::error::ApiError;
use crate::state::AppState;

/// Outcome of one adapter invocation.
enum AdapterResult {
    Ok(Value),
    Failed(String),
}

/// Run `capabilities` then a bounded `list`. Capabilities failures are
/// non-fatal (fallback) unless they signal expired auth; the `list` probe is
/// the real connectivity signal.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_probe(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    adapter_node: &Node,
    credential: &Option<Value>,
    mount: &Value,
    remote_root: &Option<String>,
    base_auth: &str,
) -> Result<ProbeOutcome, ApiError> {
    // 1. capabilities
    let caps = match invoke_adapter(
        state,
        tenant_id,
        repo,
        adapter_node,
        "capabilities",
        json!({}),
        credential,
        mount,
    )
    .await?
    {
        AdapterResult::Ok(value) => Capabilities::from_value(&value),
        AdapterResult::Failed(msg) => {
            if error_is_auth_expired(&msg) {
                return Ok(expired(Some(Capabilities::fallback())));
            }
            // Missing/soft-failing capabilities is tolerated — keep probing.
            Capabilities::fallback()
        }
    };

    // 2. bounded list
    let params = json!({ "folder_id": remote_root, "limit": PROBE_LIMIT });
    match invoke_adapter(
        state,
        tenant_id,
        repo,
        adapter_node,
        "list",
        params,
        credential,
        mount,
    )
    .await?
    {
        AdapterResult::Ok(value) => Ok(ProbeOutcome {
            ok: true,
            auth: base_auth.to_string(),
            capabilities: Some(caps),
            probe: Some(probe_from_list(&value)),
            error: None,
        }),
        AdapterResult::Failed(msg) => {
            if error_is_auth_expired(&msg) {
                return Ok(expired(Some(caps)));
            }
            Ok(ProbeOutcome {
                ok: false,
                auth: base_auth.to_string(),
                capabilities: Some(caps),
                probe: None,
                error: Some(super::TestError {
                    code: adapter_error_code(&msg).to_string(),
                    message: sanitize(&msg),
                }),
            })
        }
    }
}

/// Build the "expired credential" outcome.
pub(super) fn expired(capabilities: Option<Capabilities>) -> ProbeOutcome {
    ProbeOutcome {
        ok: false,
        auth: "expired".to_string(),
        capabilities,
        probe: None,
        error: Some(super::TestError {
            code: "auth_expired".to_string(),
            message: "the account credential is expired or was rejected".to_string(),
        }),
    }
}

/// Invoke one adapter operation synchronously, inline (never a job).
#[allow(clippy::too_many_arguments)]
async fn invoke_adapter(
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
    use crate::handlers::functions::helpers::{build_loaded_function, load_function_code};
    use raisin_functions::{ExecutionContext, FunctionExecutor};

    let input = json!({
        "operation": operation,
        "params": params,
        "credential": credential.clone().unwrap_or(Value::Null),
        "mount": mount.clone(),
    });

    let code = load_function_code(state, tenant_id, repo, node).await?;
    let loaded = build_loaded_function(node, code)?;

    let context = ExecutionContext::new(tenant_id, repo, FUNCTIONS_BRANCH, "system")
        .with_workspace(FUNCTIONS_WORKSPACE)
        .with_input(input)
        .with_admin_escalation(true);

    let api = build_function_api(
        state,
        tenant_id,
        repo,
        loaded.metadata.network_policy.clone(),
        None,
    );
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
) -> Result<Option<Node>, ApiError> {
    let svc = state.node_service_for_context(
        tenant_id,
        repo,
        FUNCTIONS_BRANCH,
        FUNCTIONS_WORKSPACE,
        Some(system_auth()),
    );
    if let Some(node) = svc.get_by_path(adapter_path).await? {
        return Ok(Some(node));
    }
    // Fallback: match on the trailing function name.
    let name = adapter_path.rsplit('/').next().unwrap_or(adapter_path);
    let nodes = svc.list_by_type("raisin:Function").await?;
    Ok(nodes.into_iter().find(|n| {
        n.name == name
            || super::support::string_prop(n, "name").as_deref() == Some(name)
            || n.path == adapter_path
    }))
}
