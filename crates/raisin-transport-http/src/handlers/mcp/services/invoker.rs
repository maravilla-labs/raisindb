// SPDX-License-Identifier: BSL-1.1

//! [`FunctionInvoker`] backed by the real function-execution pipeline.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use raisin_functions::{ExecutionContext, ExecutionResult, FunctionExecutor};
use raisin_mcp::{FunctionInvoker, McpError, McpIdentity};
use serde_json::Value;

use crate::handlers::functions::{build_loaded_function, find_function_node, load_function_code};
use crate::state::AppState;

use super::super::api_factory::build_mcp_function_api;

/// Resolves and executes `raisin:Function` nodes for MCP custom tools.
///
/// Mirrors the inline-invocation path of the functions HTTP handler: it resolves
/// the function node by name (RLS-scoped to the caller), loads its entry-file
/// code, builds the [`raisin_functions::LoadedFunction`], and runs it via
/// [`FunctionExecutor::execute`] as the calling identity. The returned
/// [`ExecutionResult`] is the executor output verbatim — a `success: false`
/// result is a function-level failure the engine surfaces as a tool error.
pub(in crate::handlers::mcp) struct HttpFunctionInvoker {
    state: AppState,
    tenant_id: String,
    repo: String,
    branch: String,
}

impl HttpFunctionInvoker {
    /// Bind an invoker to the request's tenant / repo / branch scope.
    pub(in crate::handlers::mcp) fn new(
        state: AppState,
        tenant_id: impl Into<String>,
        repo: impl Into<String>,
        branch: impl Into<String>,
    ) -> Self {
        Self {
            state,
            tenant_id: tenant_id.into(),
            repo: repo.into(),
            branch: branch.into(),
        }
    }

    async fn run(
        &self,
        identity: &McpIdentity,
        name: &str,
        input: Value,
    ) -> raisin_mcp::Result<ExecutionResult> {
        let auth = identity.to_auth_context();

        // Resolve the function node (auth MUST be forwarded or RLS hides it).
        let node = find_function_node(&self.state, &self.tenant_id, &self.repo, name, Some(&auth))
            .await
            .map_err(|e| McpError::not_found(format!("function `{name}`: {e}")))?;

        let code = load_function_code(&self.state, &self.tenant_id, &self.repo, &node)
            .await
            .map_err(|e| McpError::protocol(format!("loading function `{name}`: {e}")))?;
        let loaded = build_loaded_function(&node, code)
            .map_err(|e| McpError::protocol(format!("building function `{name}`: {e}")))?;

        let actor = identity
            .subject
            .clone()
            .unwrap_or_else(|| "system".to_string());
        let mut context = ExecutionContext::new(&self.tenant_id, &self.repo, &self.branch, &actor)
            .with_workspace("functions")
            .with_input(input)
            .with_auth(auth.clone());
        // Custom MCP tools never auto-escalate to admin.
        context = context.with_admin_escalation(false);

        let api = build_mcp_function_api(
            &self.state,
            &self.tenant_id,
            &self.repo,
            &self.branch,
            "functions",
            loaded.metadata.network_policy.clone(),
            Some(auth),
        );

        FunctionExecutor::new()
            .execute(&loaded, context, api)
            .await
            .map_err(McpError::from)
    }
}

impl FunctionInvoker for HttpFunctionInvoker {
    fn invoke<'a>(
        &'a self,
        identity: &'a McpIdentity,
        name: &'a str,
        input: Value,
    ) -> Pin<Box<dyn Future<Output = raisin_mcp::Result<ExecutionResult>> + Send + 'a>> {
        Box::pin(self.run(identity, name, input))
    }
}
