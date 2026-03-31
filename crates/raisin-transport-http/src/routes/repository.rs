// SPDX-License-Identifier: BSL-1.1

//! Routes for repository content access: uploads, workspaces, repo CRUD,
//! query/DSL, audit, and revision-based read-only endpoints.

use axum::{
    extract::DefaultBodyLimit,
    middleware::from_fn,
    routing::{get, patch, post},
    Router,
};

use crate::middleware::raisin_parsing_middleware;
use crate::state::AppState;

use super::MAX_UPLOAD_SIZE;

/// Build routes for resumable uploads, workspaces, repository content access,
/// queries, and audit endpoints.
///
/// These routes all go through `raisin_parsing_middleware` and, when the
/// `storage-rocksdb` feature is enabled, `optional_auth_middleware`.
pub(crate) fn repository_routes(state: &AppState) -> Router<AppState> {
    let mut router = Router::new()
        // ----------------------------------------------------------------
        // Resumable uploads
        // ----------------------------------------------------------------
        .route(
            "/api/uploads",
            post(crate::handlers::uploads::create_upload),
        )
        .route(
            "/api/uploads/{upload_id}",
            patch(crate::handlers::uploads::upload_chunk)
                .head(crate::handlers::uploads::get_upload_progress)
                .get(crate::handlers::uploads::get_upload_status)
                .delete(crate::handlers::uploads::cancel_upload),
        )
        .route(
            "/api/uploads/{upload_id}/complete",
            post(crate::handlers::uploads::complete_upload),
        )
        // ----------------------------------------------------------------
        // Workspaces (repository-scoped)
        // ----------------------------------------------------------------
        .route(
            "/api/workspaces/{repo}",
            get(crate::handlers::workspaces::list_workspaces),
        )
        .route(
            "/api/workspaces/{repo}/{name}",
            get(crate::handlers::workspaces::get_workspace)
                .put(crate::handlers::workspaces::put_workspace),
        )
        .route(
            "/api/workspaces/{repo}/{name}/config",
            get(crate::handlers::workspaces::get_workspace_config)
                .put(crate::handlers::workspaces::update_workspace_config),
        )
        // ----------------------------------------------------------------
        // Query endpoints (JSON filter and DSL)
        // ----------------------------------------------------------------
        .route(
            "/api/repository/{repo}/{branch}/head/{ws}/query",
            post(crate::handlers::query::post_query),
        )
        .route(
            "/api/repository/{repo}/{branch}/head/{ws}/query/dsl",
            post(crate::handlers::query::post_query_dsl),
        )
        // ----------------------------------------------------------------
        // Repository HEAD routes (current state, mutable)
        // ----------------------------------------------------------------
        .route(
            "/api/repository/{repo}/{branch}/head/{ws}/",
            get(crate::handlers::repo::repo_get_root).post(crate::handlers::repo::repo_post_root),
        )
        .route(
            "/api/repository/{repo}/{branch}/head/{ws}/$ref/{id}",
            get(crate::handlers::repo::repo_get_by_id),
        )
        .route(
            "/api/repository/{repo}/{branch}/head/{ws}/{*node_path}",
            get(crate::handlers::repo::repo_get)
                .post(crate::handlers::repo::repo_post)
                .put(crate::handlers::repo::repo_put)
                .delete(crate::handlers::repo::repo_delete)
                .layer(DefaultBodyLimit::max(MAX_UPLOAD_SIZE)),
        )
        // ----------------------------------------------------------------
        // Revision routes (historic snapshot, read-only)
        // ----------------------------------------------------------------
        .route(
            "/api/repository/{repo}/{branch}/rev/{revision}/{ws}/",
            get(crate::handlers::repo::repo_get_root_at_revision),
        )
        .route(
            "/api/repository/{repo}/{branch}/rev/{revision}/{ws}/$ref/{id}",
            get(crate::handlers::repo::repo_get_by_id_at_revision),
        )
        .route(
            "/api/repository/{repo}/{branch}/rev/{revision}/{ws}/{*node_path}",
            get(crate::handlers::repo::repo_get_at_revision),
        )
        .layer(from_fn(raisin_parsing_middleware));

    // Apply optional auth middleware (RocksDB only)
    #[cfg(feature = "storage-rocksdb")]
    {
        use crate::middleware::optional_auth_middleware;
        use axum::middleware::from_fn_with_state;

        router = router.layer(from_fn_with_state(state.clone(), optional_auth_middleware));
    }

    // ----------------------------------------------------------------
    // Audit REST endpoints
    // ----------------------------------------------------------------
    router = router
        .route(
            "/api/audit/{repo}/{branch}/{ws}/by-id/{id}",
            get(crate::handlers::audit::audit_get_by_id),
        )
        .route(
            "/api/audit/{repo}/{branch}/{ws}/{*node_path}",
            get(crate::handlers::audit::audit_get_by_path),
        );

    router
}
