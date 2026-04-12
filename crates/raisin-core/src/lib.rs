// TODO(v0.2): Update deprecated API usages to new methods and clean up unused code
#![allow(deprecated)]
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(unexpected_cfgs)]
#![allow(mismatched_lifetime_syntaxes)]

//! Core business logic and services for RaisinDB.
//!
//! This crate provides the main service layer for managing nodes, workspaces,
//! node types, and validation. It sits between the storage layer and transport layer.
//!
//! # Main Components
//!
//! - [`NodeService`] - CRUD operations and tree management for nodes
//! - [`WorkspaceService`] - Workspace management
//! - [`NodeTypeResolver`] - Node type definition loading and resolution
//! - [`NodeValidator`] - Schema validation for nodes
//! - [`ReferenceResolver`] - Reference resolution for node properties
//! - [`TranslationResolver`] - Multi-language translation resolution with locale fallback
//! - [`TranslationService`] - Translation management and update operations
//! - [`BlockTranslationService`] - Block-level translation management with UUID tracking
//!
//! # Example
//!
//! ```no_run
//! use raisin_core::NodeService;
//! use raisin_storage_memory::InMemoryStorage;
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let storage = Arc::new(InMemoryStorage::default());
//! let service = NodeService::new(storage);
//! # Ok(())
//! # }
//! ```

pub mod utils;
pub use utils::{sanitize_name, sign_asset_url, verify_asset_signature};
pub mod connection;
pub mod replication;
pub mod traits;
pub mod services {
    pub mod archetype_resolver;
    pub mod block_translation_service;
    pub mod element_type_resolver;
    pub mod indexing_policy;
    pub mod node_service;
    pub mod node_type_resolver;
    pub mod node_validation;
    pub mod permission_cache;
    pub mod permission_service;
    pub mod schema_stats_cache;
    pub mod reference_resolver;
    pub mod rls_filter;
    pub mod transaction;
    pub mod translation_resolver;
    pub mod translation_service;
    pub mod translation_staleness;
    pub mod ttl_cache;
    pub mod workspace_service;
}
pub mod audit_adapter;
pub mod init;
pub mod nodetype_init;
pub mod package_init;
pub mod system_updates;
pub mod workspace_init;
pub mod workspace_structure_init;

pub use audit_adapter::RepoAuditAdapter;
pub use connection::{
    NodeServiceBuilder, RaisinConnection, Repository, RepositoryManagement, ServerConfig,
    TenantScope, Workspace,
};
pub use services::archetype_resolver::{ArchetypeResolver, ResolvedArchetype};
pub use services::block_translation_service::{
    BatchBlockTranslationUpdate, BatchBlockUpdateResult, BlockTranslationService,
    BlockTranslationUpdate, BlockTranslationUpdateResult,
};
pub use services::element_type_resolver::{ElementTypeResolver, ResolvedElementType};
pub use services::indexing_policy::IndexingPolicy;
pub use services::node_service::NodeService;
pub use services::node_type_resolver::NodeTypeResolver;
pub use services::node_validation::NodeValidator;
pub use services::permission_cache::{
    new_shared_cache, new_shared_cache_default, CacheStats, PermissionCache, SharedPermissionCache,
};
pub use services::permission_service::{CachedPermissionService, PermissionService};
pub use services::schema_stats_cache::{
    new_shared_cache as new_shared_schema_stats_cache,
    new_shared_cache_default as new_shared_schema_stats_cache_default, SchemaStats,
    SchemaStatsCache, SharedSchemaStatsCache,
};
pub use services::reference_resolver::{node_to_json_value, ReferenceResolver};
pub use services::transaction::{Transaction, TxOperation};
pub use services::translation_resolver::TranslationResolver;
pub use services::translation_service::{
    BatchTranslationUpdate, BatchUpdateResult, TranslationService, TranslationUpdate,
    TranslationUpdateResult,
};
pub use services::translation_staleness::TranslationStalenessService;
pub use services::ttl_cache::{SharedTtlCache, TtlCache};
pub use services::workspace_service::WorkspaceService;
pub use traits::Audit;

// Tests remain below

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models as models;
    use raisin_storage::{NodeTypeRepository, Storage};
    use raisin_storage_memory::InMemoryStorage;
    use std::sync::Arc;

    #[tokio::test]
    async fn workspace_put_and_get() {
        let storage = Arc::new(InMemoryStorage::default());
        let svc = WorkspaceService::new(storage);
        let mut ws = models::workspace::Workspace::new("demo".into());
        ws.description = Some("Demo".into());
        svc.put("default", "main", ws.clone()).await.unwrap();
        let got = svc.get("default", "main", "demo").await.unwrap().unwrap();
        assert_eq!(got.name, "demo");
        assert_eq!(got.description.as_deref(), Some("Demo"));
        let list = svc.list("default", "main").await.unwrap();
        assert_eq!(list.len(), 1);
    }

    #[tokio::test]
    async fn node_put_normalizes_workspace() {
        let storage = Arc::new(InMemoryStorage::default());

        // Create a published NodeType first
        let node_type = models::nodes::types::NodeType {
            id: Some("t".to_string()),
            strict: Some(false),
            name: "t".to_string(),
            extends: None,
            mixins: Vec::new(),
            overrides: None,
            description: None,
            icon: None,
            version: Some(1),
            properties: None,
            allowed_children: Vec::new(),
            required_nodes: Vec::new(),
            initial_structure: None,
            versionable: Some(true),
            publishable: Some(true),
            auditable: Some(false),
            indexable: None,
            index_types: None,
            created_at: Some(chrono::Utc::now()),
            updated_at: None,
            published_at: None,
            published_by: None,
            previous_version: None,
            compound_indexes: None,
            is_mixin: None,
        };
        storage
            .node_types()
            .put(
                raisin_storage::scope::BranchScope::new("default", "default", "main"),
                node_type,
                raisin_storage::CommitMetadata::system("seed node type for tests"),
            )
            .await
            .unwrap();

        let svc = NodeService::new_with_context(
            storage,
            "default".to_string(),
            "default".to_string(),
            "main".to_string(),
            "wsx".to_string(),
        )
        .with_auth(raisin_models::auth::AuthContext::system());
        let node = models::nodes::Node {
            id: "a".into(),
            name: "a".into(),
            path: "/a".into(),
            node_type: "t".into(),
            archetype: None,
            properties: Default::default(),
            children: vec![],
            order_key: "a".to_string(),
            has_children: None,
            parent: None,
            version: 1,
            created_at: None,
            updated_at: None,
            published_at: None,
            published_by: None,
            updated_by: None,
            created_by: None,
            translations: None,
            tenant_id: None,
            workspace: None,
            owner_id: None,
            relations: Vec::new(),
        };
        // workspace path param is wsx; node has None, expect set to wsx
        svc.put(node.clone()).await.unwrap();
        let got = svc.get("a").await.unwrap().unwrap();
        assert_eq!(got.workspace.as_deref(), Some("wsx"));
    }

    #[test]
    fn sanitize_name_happy_cases() {
        assert_eq!(sanitize_name(" Hello World ").unwrap(), "hello-world");
        assert_eq!(sanitize_name("Multi   space").unwrap(), "multi-space");
        assert_eq!(sanitize_name("underscores_ok").unwrap(), "underscores_ok");
        assert_eq!(sanitize_name("UPPER and 123").unwrap(), "upper-and-123");
        assert_eq!(sanitize_name("--already--slug--").unwrap(), "already-slug");
        assert_eq!(
            sanitize_name("__leading_trailing__").unwrap(),
            "__leading_trailing__"
        );
        assert_eq!(
            sanitize_name("  tabs and newlines  ").unwrap(),
            "tabs-and-newlines"
        );
        assert_eq!(
            sanitize_name("Näme wïth ünicode").unwrap(),
            "nme-wth-nicode"
        );
    }

    #[test]
    fn sanitize_name_invalid_cases() {
        assert!(matches!(
            sanitize_name(""),
            Err(raisin_error::Error::Validation(_))
        ));
        assert!(matches!(
            sanitize_name("."),
            Err(raisin_error::Error::Validation(_))
        ));
        assert!(matches!(
            sanitize_name(".."),
            Err(raisin_error::Error::Validation(_))
        ));
        assert!(matches!(
            sanitize_name("bad/name"),
            Err(raisin_error::Error::Validation(_))
        ));
        let with_ctrl = format!("bad\nname");
        assert!(matches!(
            sanitize_name(&with_ctrl),
            Err(raisin_error::Error::Validation(_))
        ));
        // non-allowed characters are filtered; if any valid chars remain, it's okay
        assert_eq!(sanitize_name("$only$symbols$").unwrap(), "onlysymbols");
        // completely invalid should error
        assert!(matches!(
            sanitize_name("$$$"),
            Err(raisin_error::Error::Validation(_))
        ));
        // whitespace-only should error after trim
        assert!(matches!(
            sanitize_name("   \t  "),
            Err(raisin_error::Error::Validation(_))
        ));
    }
}
