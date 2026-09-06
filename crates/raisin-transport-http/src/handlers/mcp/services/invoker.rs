// SPDX-License-Identifier: BSL-1.1

//! [`FunctionInvoker`] backed by the real function-execution pipeline.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use raisin_functions::{ExecutionContext, ExecutionResult, FunctionExecutor};
use raisin_mcp::{FunctionInvoker, McpError, McpIdentity};
use serde_json::Value;

use raisin_models::auth::AuthContext;

use crate::handlers::functions::{build_loaded_function, load_function_code_on_branch};
use crate::state::AppState;

use super::super::api_factory::build_mcp_function_api;

/// Resolves and executes `raisin:Function` nodes for MCP custom tools.
///
/// Mirrors the inline-invocation path of the functions HTTP handler, with one
/// deliberate split: the function node and its code are *resolved* with a system
/// context (routing metadata for an already-declared, already-scope-gated tool),
/// then *executed* strictly as the calling identity — RLS scopes every node the
/// function touches to that user, and admin escalation is off. The returned
/// [`ExecutionResult`] is the executor output verbatim — a `success: false`
/// result is a function-level failure the engine surfaces as a tool error.
pub(in crate::handlers::mcp) struct HttpFunctionInvoker {
    state: AppState,
    tenant_id: String,
    repo: String,
    branch: String,
    /// The request's resolved [`AuthContext`] — the SAME one the data tools get.
    ///
    /// This must be the middleware-resolved context, not one rebuilt from
    /// [`McpIdentity`]: `McpIdentity::to_auth_context` copies `roles` but leaves
    /// `resolved_permissions: None`, and RLS reads `None` as DENY-ALL. Rebuilding
    /// it made every custom tool deny-all for non-system callers — reads silently
    /// returned zero rows, writes failed with `Permission denied: Cannot create …`
    /// — while the built-in data tools, which receive this context, read the very
    /// same paths fine. Same SQL, same RLS rules, different `AuthContext`.
    ///
    /// `None` for system/anonymous callers, where the identity mapping is exact.
    auth: Option<AuthContext>,
    /// Provenance marker (`mcp:<slug>`) stamped onto the executing context so
    /// every node this tool writes is auditable as an MCP-originated change.
    /// Held separately from `auth` so the identity-derived fallback below gets it
    /// too — otherwise anonymous/system tool writes would lose the marker.
    agent: String,
}

impl HttpFunctionInvoker {
    /// Bind an invoker to the request's tenant / repo / branch scope and the
    /// caller's resolved auth context (see the field docs — pass the request's
    /// context, never one derived from the identity).
    pub(in crate::handlers::mcp) fn new(
        state: AppState,
        tenant_id: impl Into<String>,
        repo: impl Into<String>,
        branch: impl Into<String>,
        auth: Option<AuthContext>,
        agent: String,
    ) -> Self {
        Self {
            state,
            tenant_id: tenant_id.into(),
            repo: repo.into(),
            branch: branch.into(),
            auth,
            agent,
        }
    }

    async fn run(
        &self,
        identity: &McpIdentity,
        name: &str,
        input: Value,
    ) -> raisin_mcp::Result<ExecutionResult> {
        // The request's resolved context when we have it; the identity mapping is
        // only a fallback for system/anonymous callers, where it is exact.
        // Deriving it from the identity for a real user drops the resolved
        // permissions and RLS then denies everything — see the field docs.
        // `self.auth` already carries the agent marker; the identity-derived
        // fallback is built fresh here, so mark it too.
        let auth = self
            .auth
            .clone()
            .unwrap_or_else(|| identity.to_auth_context().with_agent(self.agent.clone()));

        // ── Resolution: system context. Reading the `raisin:Function` node that
        // backs an already-declared tool is *routing metadata*, exactly like
        // reading the `raisin:McpServer` declaration itself — which `dispatch()`
        // already resolves with `AuthContext::system()` for the same reason.
        //
        // Without this, discovery (system) advertises a tool that invocation
        // (caller) cannot resolve, because RLS on the `functions` workspace
        // rarely grants a content author read on platform code. The tool is
        // listed and then 404s — indistinguishable from a missing deployment.
        //
        // This widens nothing: the tool was already advertised to this caller,
        // and the registry has already enforced the tool's own `scopes` gate
        // before we get here. Only *which* function node we may look up changes.
        //
        // ── Execution: the caller's own context, below. `ExecutionContext`
        // carries `auth`, the function's data API is built with `auth`, and
        // admin escalation stays off — so every node the function reads or
        // writes is RLS-scoped to the authenticated user, never to system.
        // `function` is the node's PATH in the functions workspace, used
        // verbatim: one direct read. It used to be a name, which has no index
        // behind it, so resolving it listed every `raisin:Function` node in the
        // workspace — materializing and access-filtering each one — and then
        // linear-searched. That ran on every tool call, so invoking one tool got
        // slower for everyone each time a package shipped another function.
        let resolve_auth = AuthContext::system();
        let node = self
            .state
            .node_service_for_context(
                &self.tenant_id,
                &self.repo,
                &self.branch,
                "functions",
                Some(resolve_auth),
            )
            .get_by_path(name)
            .await
            .map_err(|e| McpError::protocol(format!("resolving function `{name}`: {e}")))?
            .ok_or_else(|| McpError::not_found(format!("function `{name}` not found")))?;

        let code = load_function_code_on_branch(
            &self.state,
            &self.tenant_id,
            &self.repo,
            &self.branch,
            &node,
        )
        .await
        .map_err(|e| McpError::protocol(format!("loading function `{name}`: {e}")))?;
        let mut loaded = build_loaded_function(&node, code)
            .map_err(|e| McpError::protocol(format!("building function `{name}`: {e}")))?;

        // Module files for ES6 `import` resolution. `build_loaded_function` only
        // carries the entry source, and the QuickJS loader is rebuilt per
        // execution against `loaded.files` — so without this an MCP tool whose
        // entry imports anything dies at declare time with
        // `Error resolving module '…' from 'entry'`, while the very same
        // function works when run from a trigger or flow (that path goes through
        // `code_loader` in the job executor). Tools sharing a helper library are
        // the normal case, so the two paths have to agree.
        loaded.files = crate::handlers::functions::load_function_modules_on_branch(
            &self.state,
            &self.tenant_id,
            &self.repo,
            &self.branch,
            &node.path,
            loaded.metadata.entry_file_path(),
            loaded.code.as_text().unwrap_or(""),
        )
        .await;

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
            loaded.metadata.secret_policy.clone(),
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
