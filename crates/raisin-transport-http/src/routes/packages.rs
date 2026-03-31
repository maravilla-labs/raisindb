// SPDX-License-Identifier: BSL-1.1

//! Routes for package management: upload, install/uninstall, browsing,
//! and the unified package command endpoint.

use axum::{
    extract::DefaultBodyLimit,
    middleware::from_fn_with_state,
    routing::{get, post},
    Router,
};

#[cfg(feature = "storage-rocksdb")]
use crate::middleware::optional_auth_middleware;
use crate::state::AppState;

use super::MAX_UPLOAD_SIZE;

/// Build package management routes.
///
/// Includes legacy upload, install/uninstall, listing, browsing,
/// create-from-selection, and the unified package command endpoint.
pub(crate) fn package_routes(state: &AppState) -> Router<AppState> {
    let mut router = Router::new();

    // Upload package endpoint (DEPRECATED)
    // Use the unified endpoint instead:
    //   POST /api/repository/{repo}/main/head/packages/{name}?node_type=raisin:Package
    #[allow(deprecated)]
    {
        router = router.route(
            "/api/repos/{repo}/packages/upload",
            post(crate::handlers::packages::upload_package)
                .layer(DefaultBodyLimit::max(MAX_UPLOAD_SIZE))
                .layer(from_fn_with_state(state.clone(), optional_auth_middleware)),
        );
    }

    router
        // Install/uninstall package
        .route(
            "/api/repos/{repo}/packages/{name}/install",
            post(crate::handlers::packages::install_package)
                .layer(from_fn_with_state(state.clone(), optional_auth_middleware)),
        )
        .route(
            "/api/repos/{repo}/packages/{name}/uninstall",
            post(crate::handlers::packages::uninstall_package)
                .layer(from_fn_with_state(state.clone(), optional_auth_middleware)),
        )
        // List and get packages
        .route(
            "/api/repos/{repo}/packages",
            get(crate::handlers::packages::list_packages)
                .layer(from_fn_with_state(state.clone(), optional_auth_middleware)),
        )
        .route(
            "/api/repos/{repo}/packages/{name}",
            get(crate::handlers::packages::get_package)
                .layer(from_fn_with_state(state.clone(), optional_auth_middleware)),
        )
        // Browse package contents (legacy)
        .route(
            "/api/repos/{repo}/packages/{name}/contents",
            get(crate::handlers::packages::list_package_contents)
                .layer(from_fn_with_state(state.clone(), optional_auth_middleware)),
        )
        .route(
            "/api/repos/{repo}/packages/{name}/contents/{*path}",
            get(crate::handlers::packages::get_package_file)
                .layer(from_fn_with_state(state.clone(), optional_auth_middleware)),
        )
        // Create package from selection
        .route(
            "/api/packages/{repo}/{branch}/head/raisin:create-from-selection",
            post(crate::handlers::packages::create_package_from_selection)
                .layer(from_fn_with_state(state.clone(), optional_auth_middleware)),
        )
        // Package command endpoints (dedicated /api/packages with hardcoded workspace)
        // Workspace is hardcoded as "packages" - path is relative to packages workspace
        // Path contains raisin:browse, raisin:file, or raisin:install as delimiter
        .route(
            "/api/packages/{repo}/{branch}/head/{*path}",
            get(crate::handlers::packages::handle_package_command)
                .post(crate::handlers::packages::handle_package_command)
                .layer(from_fn_with_state(state.clone(), optional_auth_middleware)),
        )
}
