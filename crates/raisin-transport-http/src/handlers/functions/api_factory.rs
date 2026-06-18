// SPDX-License-Identifier: BSL-1.1

//! Function API factory for building `RaisinFunctionApi` instances.
//!
//! Constructs the callback-based API that JavaScript/Starlark functions
//! use to interact with the database (node CRUD, SQL, transactions,
//! HTTP, and AI completion).
//!
//! Delegates to the canonical `create_production_callbacks` from
//! `raisin-functions` for all callback wiring.

use std::sync::Arc;

use raisin_functions::{
    execution::callbacks::create_production_callbacks, execution::ExecutionDependencies,
    ExecutionContext, NetworkPolicy, RaisinFunctionApi,
};
use raisin_models::auth::AuthContext;

use crate::state::AppState;

use super::{DEFAULT_BRANCH, FUNCTIONS_WORKSPACE};

/// Build the `RaisinFunctionApi` used by function execution.
///
/// Wires up all callback closures for node access, SQL queries,
/// transaction management, HTTP requests, and AI completion via
/// the canonical `create_production_callbacks`.
///
/// `tenant_id` scopes every callback (node CRUD, SQL, transactions) to the
/// caller's tenant — passing the wrong value here causes the function body
/// to read and write a different tenant's data.
#[cfg(feature = "storage-rocksdb")]
pub(crate) fn build_function_api(
    state: &AppState,
    tenant_id: &str,
    repo: &str,
    network_policy: NetworkPolicy,
    auth_context: Option<AuthContext>,
) -> Arc<RaisinFunctionApi> {
    let repo_id = repo.to_string();
    let tenant = tenant_id.to_string();
    let branch = DEFAULT_BRANCH.to_string();

    // Create AI config store from storage
    let ai_config_store: Option<Arc<dyn raisin_ai::TenantAIConfigStore>> =
        Some(Arc::new(state.storage.tenant_ai_config_repository()));

    // Create ExecutionDependencies from AppState
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

    // Build all callbacks via canonical factory
    let callbacks = create_production_callbacks(deps, tenant, repo_id, branch, auth_context);

    Arc::new(RaisinFunctionApi::new(
        ExecutionContext::new(tenant_id, repo, DEFAULT_BRANCH, "system")
            .with_workspace(FUNCTIONS_WORKSPACE),
        network_policy,
        callbacks,
    ))
}
