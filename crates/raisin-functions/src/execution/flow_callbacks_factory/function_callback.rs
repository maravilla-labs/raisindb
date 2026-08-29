// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Function executor callback for flow execution

use super::types::FunctionExecutorCallback;
use crate::execution::ExecutionDependencies;
use raisin_binary::BinaryStorage;
use raisin_storage::{transactional::TransactionalStorage, Storage};
use std::sync::Arc;

/// Agents live in the functions workspace, like every other callable node.
const AGENTS_WORKSPACE: &str = "functions";

/// Create function executor callback - executes serverless functions
pub(super) fn create_function_executor<S, B>(
    deps: &Arc<ExecutionDependencies<S, B>>,
) -> FunctionExecutorCallback
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    let deps = deps.clone();
    Arc::new(
        move |function_ref, input, tenant_id, repo_id, branch, workspace, agent| {
            let deps = deps.clone();
            Box::pin(async move {
                tracing::debug!(
                    function_ref = %function_ref,
                    tenant_id = %tenant_id,
                    repo_id = %repo_id,
                    branch = %branch,
                    workspace = %workspace,
                    "Flow function_executor callback"
                );

                // Use the existing function executor if available
                // This reuses the same execution logic as FunctionExecutionHandler
                use crate::execution::execute_function;
                use crate::execution::FunctionExecutionConfig;

                let execution_id = nanoid::nanoid!();
                let config = FunctionExecutionConfig::default();

                execute_function(
                    &deps,
                    &config,
                    &function_ref,
                    &execution_id,
                    input,
                    &tenant_id,
                    &repo_id,
                    &branch,
                    &workspace,
                    // WHO this call is, and — when the marker names an AGENT
                    // that asked for its own rights — what it may do. See
                    // `flow_auth_context` below.
                    flow_auth_context(&deps, &tenant_id, &repo_id, &branch, agent).await?,
                    None, // no real-time log streaming for flow functions
                )
                .await
                .map_err(|e| format!("Function execution failed: {}", e))
                .map(|result| result.result.unwrap_or(serde_json::json!(null)))
            })
        },
    )
}

/// The auth context for a function called from a flow.
///
/// Attribution first: the marker is the flow's identity (and the trigger behind
/// it), so a write commits attributed to what caused it rather than to nobody.
/// Historically that was the whole job, and the context was always
/// `AuthContext::system()` — the same authority the executor already used for
/// `None`, carrying a marker that grants nothing.
///
/// PERMISSIONS second, and this is the part that was missing. When the marker
/// names an AGENT (`agent:/agents/x@flow:/flows/y`) and that agent is
/// configured to run under its own roles, the context becomes the agent's own
/// resolved permissions. Without it an agent restricted in the UI still had
/// full system rights the moment it was called from inside a flow — the
/// configuration said one thing and the engine did another.
///
/// A resolution FAILURE refuses the call rather than falling back to system:
/// silently widening an agent's rights is the one outcome nobody notices.
async fn flow_auth_context<S, B>(
    deps: &Arc<ExecutionDependencies<S, B>>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    agent: Option<String>,
) -> Result<Option<raisin_models::auth::AuthContext>, String>
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    let Some(marker) = agent else {
        return Ok(None);
    };

    // `agent:<path>` is the only marker that names a principal with rights of
    // its own; a flow, trigger or schedule marker is provenance only.
    if let Some(agent_path) = marker
        .split('@')
        .next()
        .and_then(|head| head.strip_prefix("agent:"))
        .filter(|path| path.starts_with('/'))
    {
        if let Some(ctx) = raisin_core::services::agent_auth::resolve_agent_context(
            &deps.storage,
            tenant_id,
            repo_id,
            branch,
            AGENTS_WORKSPACE,
            agent_path,
            &marker,
        )
        .await?
        {
            return Ok(Some(ctx));
        }
    }

    Ok(Some(
        raisin_models::auth::AuthContext::system().with_agent(marker),
    ))
}
