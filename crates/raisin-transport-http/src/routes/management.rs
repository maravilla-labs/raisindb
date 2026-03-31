// SPDX-License-Identifier: BSL-1.1

//! Routes for schema management (nodetypes, archetypes, elementtypes),
//! branch/tag/revision management, repository CRUD, and registry endpoints.

use axum::routing::{get, patch, post};
use axum::Router;

use crate::state::AppState;

/// Build routes for schema management, branch/tag/revision management,
/// repository CRUD, and registry endpoints.
pub(crate) fn management_routes(_state: &AppState) -> Router<AppState> {
    let mut router = Router::new()
        // ----------------------------------------------------------------
        // NodeType management
        // ----------------------------------------------------------------
        .route(
            "/api/management/{repo}/{branch}/nodetypes",
            post(crate::handlers::node_types::create_node_type)
                .get(crate::handlers::node_types::list_node_types),
        )
        .route(
            "/api/management/{repo}/{branch}/nodetypes/published",
            get(crate::handlers::node_types::list_published_node_types),
        )
        .route(
            "/api/management/{repo}/{branch}/nodetypes/validate",
            post(crate::handlers::node_types::validate_node),
        )
        .route(
            "/api/management/{repo}/{branch}/nodetypes/{name}",
            get(crate::handlers::node_types::get_node_type)
                .put(crate::handlers::node_types::update_node_type)
                .delete(crate::handlers::node_types::delete_node_type),
        )
        .route(
            "/api/management/{repo}/{branch}/nodetypes/{name}/resolved",
            get(crate::handlers::node_types::get_resolved_node_type),
        )
        .route(
            "/api/management/{repo}/{branch}/nodetypes/{name}/publish",
            post(crate::handlers::node_types::publish_node_type),
        )
        .route(
            "/api/management/{repo}/{branch}/nodetypes/{name}/unpublish",
            post(crate::handlers::node_types::unpublish_node_type),
        )
        // ----------------------------------------------------------------
        // Archetype management
        // ----------------------------------------------------------------
        .route(
            "/api/management/{repo}/{branch}/archetypes",
            post(crate::handlers::archetypes::create_archetype)
                .get(crate::handlers::archetypes::list_archetypes),
        )
        .route(
            "/api/management/{repo}/{branch}/archetypes/published",
            get(crate::handlers::archetypes::list_published_archetypes),
        )
        .route(
            "/api/management/{repo}/{branch}/archetypes/{name}",
            get(crate::handlers::archetypes::get_archetype)
                .put(crate::handlers::archetypes::update_archetype)
                .delete(crate::handlers::archetypes::delete_archetype),
        )
        .route(
            "/api/management/{repo}/{branch}/archetypes/{name}/resolved",
            get(crate::handlers::archetypes::get_resolved_archetype),
        )
        .route(
            "/api/management/{repo}/{branch}/archetypes/{name}/publish",
            post(crate::handlers::archetypes::publish_archetype),
        )
        .route(
            "/api/management/{repo}/{branch}/archetypes/{name}/unpublish",
            post(crate::handlers::archetypes::unpublish_archetype),
        )
        // ----------------------------------------------------------------
        // ElementType management
        // ----------------------------------------------------------------
        .route(
            "/api/management/{repo}/{branch}/elementtypes",
            post(crate::handlers::element_types::create_element_type)
                .get(crate::handlers::element_types::list_element_types),
        )
        .route(
            "/api/management/{repo}/{branch}/elementtypes/published",
            get(crate::handlers::element_types::list_published_element_types),
        )
        .route(
            "/api/management/{repo}/{branch}/elementtypes/{name}",
            get(crate::handlers::element_types::get_element_type)
                .put(crate::handlers::element_types::update_element_type)
                .delete(crate::handlers::element_types::delete_element_type),
        )
        .route(
            "/api/management/{repo}/{branch}/elementtypes/{name}/resolved",
            get(crate::handlers::element_types::get_resolved_element_type),
        )
        .route(
            "/api/management/{repo}/{branch}/elementtypes/{name}/publish",
            post(crate::handlers::element_types::publish_element_type),
        )
        .route(
            "/api/management/{repo}/{branch}/elementtypes/{name}/unpublish",
            post(crate::handlers::element_types::unpublish_element_type),
        )
        // ----------------------------------------------------------------
        // Branch management (repository-level)
        // ----------------------------------------------------------------
        .route(
            "/api/management/repositories/{tenant_id}/{repo_id}/branches",
            post(crate::handlers::branches::create_branch)
                .get(crate::handlers::branches::list_branches),
        )
        .route(
            "/api/management/repositories/{tenant_id}/{repo_id}/branches/{name}",
            get(crate::handlers::branches::get_branch)
                .delete(crate::handlers::branches::delete_branch),
        )
        .route(
            "/api/management/repositories/{tenant_id}/{repo_id}/branches/{name}/head",
            get(crate::handlers::branches::get_branch_head)
                .put(crate::handlers::branches::update_branch_head),
        );

    // Branch comparison and merge endpoints - RocksDB only
    #[cfg(feature = "storage-rocksdb")]
    {
        router = router
            .route(
                "/api/management/repositories/{tenant_id}/{repo_id}/branches/{branch}/compare/{base_branch}",
                get(crate::handlers::branches::compare_branches),
            )
            .route(
                "/api/management/repositories/{tenant_id}/{repo_id}/branches/{target_branch}/merge",
                post(crate::handlers::branches::merge_branches),
            )
            .route(
                "/api/management/repositories/{tenant_id}/{repo_id}/branches/{target_branch}/resolve-merge",
                post(crate::handlers::branches::resolve_merge_conflicts),
            )
            .route(
                "/api/management/repositories/{tenant_id}/{repo_id}/branches/{name}/upstream",
                patch(crate::handlers::branches::set_upstream_branch),
            );
    }

    router = router
        // ----------------------------------------------------------------
        // Tag management (repository-level)
        // ----------------------------------------------------------------
        .route(
            "/api/management/repositories/{tenant_id}/{repo_id}/tags",
            post(crate::handlers::tags::create_tag).get(crate::handlers::tags::list_tags),
        )
        .route(
            "/api/management/repositories/{tenant_id}/{repo_id}/tags/{name}",
            get(crate::handlers::tags::get_tag).delete(crate::handlers::tags::delete_tag),
        )
        // ----------------------------------------------------------------
        // Revision history (repository-level)
        // ----------------------------------------------------------------
        .route(
            "/api/management/repositories/{tenant_id}/{repo_id}/revisions",
            get(crate::handlers::revisions::list_revisions),
        )
        .route(
            "/api/management/repositories/{tenant_id}/{repo_id}/revisions/{revision}",
            get(crate::handlers::revisions::get_revision),
        )
        .route(
            "/api/management/repositories/{tenant_id}/{repo_id}/revisions/{revision}/changes",
            get(crate::handlers::revisions::get_revision_changes),
        )
        // ----------------------------------------------------------------
        // Repository management (tenant-level)
        // ----------------------------------------------------------------
        .route(
            "/api/repositories",
            get(crate::handlers::repositories::list_repositories)
                .post(crate::handlers::repositories::create_repository),
        )
        .route(
            "/api/repositories/{repo_id}",
            get(crate::handlers::repositories::get_repository)
                .put(crate::handlers::repositories::update_repository)
                .delete(crate::handlers::repositories::delete_repository),
        )
        // Translation configuration (repository-level)
        .route(
            "/api/repositories/{repo_id}/translation-config",
            get(crate::handlers::repositories::get_translation_config)
                .patch(crate::handlers::repositories::update_translation_config),
        )
        // ----------------------------------------------------------------
        // Registry management (tenant/deployment tracking)
        // ----------------------------------------------------------------
        .route(
            "/api/management/registry/tenants",
            get(crate::handlers::registry::list_tenants)
                .post(crate::handlers::registry::create_tenant),
        )
        .route(
            "/api/management/registry/tenants/{tenant_id}",
            get(crate::handlers::registry::get_tenant),
        )
        .route(
            "/api/management/registry/deployments",
            get(crate::handlers::registry::list_deployments)
                .post(crate::handlers::registry::create_deployment),
        )
        .route(
            "/api/management/registry/deployments/{tenant_id}/{deployment_key}",
            get(crate::handlers::registry::get_deployment),
        );

    router
}
