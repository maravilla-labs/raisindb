// SPDX-License-Identifier: BSL-1.1

//! Branch-aware `FunctionApi` factory for the MCP transport.
//!
//! The MCP server descriptor and its built-in data tools read and write content
//! through a [`RaisinFunctionApi`] — the same RLS-scoped callback API surface
//! server-side functions run against. Unlike the functions HTTP handler (which
//! hard-codes the `main` branch), the MCP route carries an explicit `{branch}`
//! path segment, so this factory threads that branch through every callback.

use std::sync::Arc;

use raisin_functions::{
    execution::callbacks::create_production_callbacks, execution::ExecutionDependencies,
    ExecutionContext, NetworkPolicy, RaisinFunctionApi,
};
use raisin_models::auth::AuthContext;

use crate::state::AppState;

/// Build a branch-scoped [`RaisinFunctionApi`] for MCP data access.
///
/// Every node CRUD / SQL / transaction callback is scoped to `tenant_id`,
/// `repo`, and `branch`, and RLS-filtered by `auth_context`. The execution
/// context's default workspace is the MCP session's `workspace`; individual
/// data tools pass an explicit workspace to each call regardless.
#[cfg(feature = "storage-rocksdb")]
pub(super) fn build_mcp_function_api(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    branch: &str,
    workspace: &str,
    network_policy: NetworkPolicy,
    auth_context: Option<AuthContext>,
) -> Arc<RaisinFunctionApi> {
    let repo_id = repo.to_string();
    let tenant = tenant_id.to_string();
    let branch_owned = branch.to_string();

    // AI config store (used by ai.* callbacks; harmless when unused by tools).
    let ai_config_store: Option<Arc<dyn raisin_ai::TenantAIConfigStore>> =
        Some(Arc::new(state.storage.tenant_ai_config_repository()));

    let deps = Arc::new(ExecutionDependencies {
        storage: state.storage.clone(),
        binary_storage: state.bin.clone(),
        indexing_engine: state.indexing_engine.clone(),
        hnsw_engine: state.hnsw_engine.clone(),
        http_client: reqwest::Client::new(),
        ai_config_store,
        job_registry: None,
        job_data_store: None,
        lock_manager: state.lock_manager.clone(),
    });

    let callbacks = create_production_callbacks(deps, tenant, repo_id, branch_owned, auth_context);

    Arc::new(RaisinFunctionApi::new(
        ExecutionContext::new(tenant_id, repo, branch, "system").with_workspace(workspace),
        network_policy,
        callbacks,
    ))
}
