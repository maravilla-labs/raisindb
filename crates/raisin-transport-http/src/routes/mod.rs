// SPDX-License-Identifier: BSL-1.1

//! HTTP route definitions for the RaisinDB transport layer.
//!
//! Routes are organized into submodules by domain:
//! - [`repository`]: Uploads, workspaces, repo CRUD, queries, audit
//! - [`management`]: Schema types, branches, tags, revisions, repositories, registry
//! - [`auth`]: Authentication, identity, workspace access, user management (RocksDB)
//! - [`admin`]: Admin ops, AI config, embeddings, search, replication (RocksDB)
//! - [`packages`]: Package upload, install, browse, commands
//! - [`functions`]: Serverless functions, flows, webhooks, triggers

mod admin;
mod auth;
mod functions;
mod management;
mod packages;
mod repository;

use axum::Router;

use crate::state::AppState;

/// Maximum upload size: 50GB to support large content migration packages
const MAX_UPLOAD_SIZE: usize = 50 * 1024 * 1024 * 1024;

use crate::middleware::ensure_tenant_middleware;
#[cfg(feature = "storage-rocksdb")]
use crate::middleware::unified_cors_middleware;

/// Build the complete HTTP router for RaisinDB.
///
/// Composes all route groups and applies global middleware layers:
/// 1. CORS middleware (RocksDB only, hierarchical: repo > tenant > global)
/// 2. Tenant middleware (outermost, applied to ALL routes)
pub fn routes(state: AppState) -> Router {
    let mut router = Router::new();

    // Repository content routes (uploads, workspaces, queries, audit)
    router = router.merge(repository::repository_routes(&state));

    // Schema and management routes (nodetypes, branches, tags, repos, registry)
    router = router.merge(management::management_routes(&state));

    // RocksDB-specific routes
    #[cfg(feature = "storage-rocksdb")]
    {
        // Auth, identity, workspace access, user management
        router = router.merge(auth::auth_routes(&state));

        // Admin ops, AI config, embeddings, search, replication, system updates
        router = router.merge(admin::admin_routes(&state));
    }

    // Package management routes
    router = router.merge(packages::package_routes(&state));

    // Functions, flows, webhooks, triggers
    router = router.merge(functions::function_routes(&state));

    // Apply unified CORS middleware for all routes (RocksDB only)
    // Implements hierarchical CORS resolution: Repo -> Tenant -> Global
    #[cfg(feature = "storage-rocksdb")]
    {
        use axum::middleware::from_fn_with_state;

        router = router.layer(from_fn_with_state(state.clone(), unified_cors_middleware));
    }

    // Apply tenant middleware LAST (outermost) to ensure ALL routes go through it
    router
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            ensure_tenant_middleware,
        ))
        .with_state(state)
}
