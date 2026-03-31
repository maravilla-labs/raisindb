// SPDX-License-Identifier: BSL-1.1

//! Routes for admin/management operations, AI configuration, embeddings,
//! search/SQL, processing rules, replication, and system updates.
//!
//! All routes in this module require the `storage-rocksdb` feature.

use axum::Router;

use crate::state::AppState;

/// Build admin, AI, search, replication, and system-update routes (RocksDB only).
#[cfg(feature = "storage-rocksdb")]
pub(crate) fn admin_routes(state: &AppState) -> Router<AppState> {
    use crate::middleware::optional_auth_middleware;
    use axum::middleware::from_fn_with_state;
    use axum::routing::{get, post};

    Router::new()
        // ----------------------------------------------------------------
        // Replication synchronization
        // ----------------------------------------------------------------
        .route(
            "/api/replication/{tenant_id}/{repo_id}/operations",
            get(crate::handlers::replication::get_operations),
        )
        .route(
            "/api/replication/{tenant_id}/{repo_id}/operations/batch",
            post(crate::handlers::replication::apply_operations_batch),
        )
        .route(
            "/api/replication/{tenant_id}/{repo_id}/vector-clock",
            get(crate::handlers::replication::get_vector_clock),
        )
        // ----------------------------------------------------------------
        // Tenant embedding configuration
        // ----------------------------------------------------------------
        .route(
            "/api/tenants/{tenant_id}/embeddings/config",
            get(crate::handlers::embeddings::get_tenant_embedding_config)
                .post(crate::handlers::embeddings::set_tenant_embedding_config),
        )
        .route(
            "/api/tenants/{tenant_id}/embeddings/config/test",
            post(crate::handlers::embeddings::test_embedding_connection),
        )
        // ----------------------------------------------------------------
        // Unified AI configuration (tenant-scoped)
        // ----------------------------------------------------------------
        .route(
            "/api/tenants/{tenant_id}/ai/config",
            get(crate::handlers::ai::get_ai_config).put(crate::handlers::ai::set_ai_config),
        )
        .route(
            "/api/tenants/{tenant_id}/ai/providers",
            get(crate::handlers::ai::list_providers),
        )
        .route(
            "/api/tenants/{tenant_id}/ai/providers/{provider}/test",
            post(crate::handlers::ai::test_provider_connection),
        )
        .route(
            "/api/tenants/{tenant_id}/ai/models",
            get(crate::handlers::ai::list_all_models),
        )
        .route(
            "/api/tenants/{tenant_id}/ai/models/{use_case}",
            get(crate::handlers::ai::list_models_by_use_case),
        )
        .route(
            "/api/tenants/{tenant_id}/ai/providers/{provider}/models/{model}/capabilities",
            get(crate::handlers::ai::get_model_capabilities),
        )
        // HuggingFace model management
        .route(
            "/api/tenants/{tenant_id}/ai/models/huggingface",
            get(crate::handlers::ai::list_huggingface_models),
        )
        .route(
            "/api/tenants/{tenant_id}/ai/models/huggingface/{model_id}",
            get(crate::handlers::ai::get_huggingface_model)
                .delete(crate::handlers::ai::delete_huggingface_model),
        )
        .route(
            "/api/tenants/{tenant_id}/ai/models/huggingface/{model_id}/download",
            post(crate::handlers::ai::download_huggingface_model),
        )
        // Local model registry (tenant-independent)
        .route(
            "/api/ai/models/local/caption",
            get(crate::handlers::ai::list_local_caption_models),
        )
        // ----------------------------------------------------------------
        // Processing Rules (repository-scoped)
        // ----------------------------------------------------------------
        .route(
            "/api/repository/{repo}/ai/rules",
            get(crate::handlers::processing_rules::list_rules)
                .post(crate::handlers::processing_rules::create_rule),
        )
        .route(
            "/api/repository/{repo}/ai/rules/reorder",
            axum::routing::put(crate::handlers::processing_rules::reorder_rules),
        )
        .route(
            "/api/repository/{repo}/ai/rules/test",
            post(crate::handlers::processing_rules::test_rule_match),
        )
        .route(
            "/api/repository/{repo}/ai/rules/{rule_id}",
            get(crate::handlers::processing_rules::get_rule)
                .put(crate::handlers::processing_rules::update_rule)
                .delete(crate::handlers::processing_rules::delete_rule),
        )
        // ----------------------------------------------------------------
        // Full-text search and SQL query
        // ----------------------------------------------------------------
        .route(
            "/api/repository/{repo}/{branch}/fulltext/search",
            post(crate::handlers::repo::fulltext_search),
        )
        // Hybrid search (fulltext + vector with RRF)
        .route(
            "/api/search/{repo}",
            get(crate::handlers::hybrid_search::hybrid_search),
        )
        .route(
            "/api/sql/{repo}",
            post(crate::handlers::sql::execute_sql_query)
                .layer(from_fn_with_state(state.clone(), optional_auth_middleware)),
        )
        .route(
            "/api/sql/{repo}/{branch}",
            post(crate::handlers::sql::execute_sql_query_with_branch)
                .layer(from_fn_with_state(state.clone(), optional_auth_middleware)),
        )
        // ----------------------------------------------------------------
        // Management API - Database Level (repository-specific indexes)
        // ----------------------------------------------------------------
        .route(
            "/api/admin/management/database/{tenant}/{repo}/fulltext/verify",
            post(crate::handlers::management::verify_fulltext_index),
        )
        .route(
            "/api/admin/management/database/{tenant}/{repo}/fulltext/rebuild",
            post(crate::handlers::management::rebuild_fulltext_index),
        )
        .route(
            "/api/admin/management/database/{tenant}/{repo}/fulltext/optimize",
            post(crate::handlers::management::optimize_fulltext_index),
        )
        .route(
            "/api/admin/management/database/{tenant}/{repo}/fulltext/purge",
            post(crate::handlers::management::purge_fulltext_index),
        )
        .route(
            "/api/admin/management/database/{tenant}/{repo}/fulltext/health",
            get(crate::handlers::management::get_fulltext_health),
        )
        .route(
            "/api/admin/management/database/{tenant}/{repo}/vector/verify",
            post(crate::handlers::management::verify_vector_index),
        )
        .route(
            "/api/admin/management/database/{tenant}/{repo}/vector/rebuild",
            post(crate::handlers::management::rebuild_vector_index),
        )
        .route(
            "/api/admin/management/database/{tenant}/{repo}/vector/regenerate",
            post(crate::handlers::management::regenerate_vector_embeddings),
        )
        .route(
            "/api/admin/management/database/{tenant}/{repo}/vector/optimize",
            post(crate::handlers::management::optimize_vector_index),
        )
        .route(
            "/api/admin/management/database/{tenant}/{repo}/vector/restore",
            post(crate::handlers::management::restore_vector_index),
        )
        .route(
            "/api/admin/management/database/{tenant}/{repo}/vector/health",
            get(crate::handlers::management::get_vector_health),
        )
        // RocksDB index reindex
        .route(
            "/api/admin/management/database/{tenant}/{repo}/reindex/start",
            post(crate::handlers::management::reindex_start),
        )
        // Relation index integrity
        .route(
            "/api/admin/management/database/{tenant}/{repo}/relations/verify",
            post(crate::handlers::management::verify_relation_integrity),
        )
        .route(
            "/api/admin/management/database/{tenant}/{repo}/relations/repair",
            post(crate::handlers::management::repair_relation_integrity),
        )
        // ----------------------------------------------------------------
        // Management API - Global Level (RocksDB operations)
        // ----------------------------------------------------------------
        .route(
            "/api/admin/management/global/rocksdb/compact",
            post(crate::handlers::management::compact_rocksdb),
        )
        .route(
            "/api/admin/management/global/rocksdb/backup",
            post(crate::handlers::management::backup_rocksdb),
        )
        .route(
            "/api/admin/management/global/rocksdb/stats",
            get(crate::handlers::management::get_rocksdb_stats),
        )
        // ----------------------------------------------------------------
        // Management API - Tenant Level
        // ----------------------------------------------------------------
        .route(
            "/api/admin/management/tenant/{tenant}/cleanup",
            post(crate::handlers::management::cleanup_tenant),
        )
        .route(
            "/api/admin/management/tenant/{tenant}/stats",
            get(crate::handlers::management::get_tenant_stats),
        )
        // ----------------------------------------------------------------
        // System Updates (check for and apply built-in updates)
        // ----------------------------------------------------------------
        .route(
            "/api/management/repositories/{tenant_id}/{repo_id}/system-updates",
            get(crate::handlers::system_updates::get_pending_updates),
        )
        .route(
            "/api/management/repositories/{tenant_id}/{repo_id}/system-updates/apply",
            post(crate::handlers::system_updates::apply_updates),
        )
}
