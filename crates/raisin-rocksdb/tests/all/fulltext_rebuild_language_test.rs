//! A fulltext REBUILD indexes base content in the repository's default
//! language — the same language the event path uses — not a hard-coded "en".
//!
//! Before, rebuilding a German repository filed its pages under English:
//! `FULLTEXT_SEARCH('flugplan', 'de')` found nothing and `'en'` found the
//! German pages.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_error::Result;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_models::nodes::Node;
use raisin_models::workspace::Workspace;
use raisin_rocksdb::fractional_index;
use raisin_rocksdb::{RocksDBConfig, RocksDBStorage};
use raisin_storage::scope::BranchScope;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{
    BranchRepository, CommitMetadata, FullTextSearchQuery, IndexingEngine, NodeTypeRepository,
    RegistryRepository, RepositoryManagementRepository, Storage,
};
use tempfile::TempDir;
use uuid::Uuid;

const TENANT: &str = "ftlang-test";
const REPO: &str = "site";
const BRANCH: &str = "main";
const WORKSPACE: &str = "default";

fn build_node(path: &str, title: &str) -> Node {
    let name = path.trim_start_matches('/').to_string();
    let mut properties = HashMap::new();
    properties.insert(
        "title".to_string(),
        PropertyValue::String(title.to_string()),
    );

    Node {
        id: Uuid::new_v4().to_string(),
        name,
        path: path.to_string(),
        node_type: "raisin:Folder".to_string(),
        archetype: None,
        properties,
        children: Vec::new(),
        order_key: fractional_index::first(),
        has_children: Some(false),
        parent: Some("/".to_string()),
        version: 1,
        created_at: Some(chrono::Utc::now()),
        updated_at: None,
        published_at: None,
        published_by: None,
        updated_by: Some("user".to_string()),
        created_by: Some("user".to_string()),
        translations: None,
        tenant_id: Some(TENANT.to_string()),
        workspace: Some(WORKSPACE.to_string()),
        owner_id: None,
        relations: Vec::new(),
    }
}

async fn setup_storage(default_language: &str) -> Result<(Arc<RocksDBStorage>, TempDir)> {
    let temp_dir = TempDir::new().unwrap();
    let mut config = RocksDBConfig::default();
    config.path = temp_dir.path().to_path_buf();
    let storage = Arc::new(RocksDBStorage::with_config(config)?);

    storage
        .registry()
        .register_tenant(TENANT, HashMap::new())
        .await?;

    let repo_config = RepositoryConfig {
        default_language: default_language.to_string(),
        supported_languages: vec![default_language.to_string(), "en".to_string()],
        locale_fallback_chains: HashMap::new(),
        default_branch: BRANCH.to_string(),
        description: Some("Fulltext rebuild language test".to_string()),
        tags: HashMap::new(),
        localized_names: Default::default(),
    };
    storage
        .repository_management()
        .create_repository(TENANT, REPO, repo_config)
        .await?;

    storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "system", None, None, false, false)
        .await?;

    // Seed the node type BEFORE creating the workspace: WorkspaceService::put
    // bootstraps a ROOT node for new workspaces, which requires the default
    // folder type to already be registered.
    let folder_type = NodeType {
        id: Some("raisin:Folder".to_string()),
        strict: Some(false),
        name: "raisin:Folder".to_string(),
        extends: None,
        mixins: Vec::new(),
        overrides: None,
        description: None,
        icon: None,
        version: Some(1),
        properties: None,
        allowed_children: vec!["*".to_string()],
        required_nodes: Vec::new(),
        initial_structure: None,
        versionable: Some(true),
        immutable: None,
        publishable: Some(true),
        auditable: Some(false),
        indexable: Some(true),
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
        .upsert(
            BranchScope::new(TENANT, REPO, BRANCH),
            folder_type,
            CommitMetadata::system("seed folder type"),
        )
        .await?;

    let mut workspace = Workspace::new(WORKSPACE.to_string());
    workspace.config.default_branch = BRANCH.to_string();
    WorkspaceService::new(storage.clone())
        .put(TENANT, REPO, workspace)
        .await?;

    Ok((storage, temp_dir))
}

fn search(
    engine: &raisin_indexer::tantivy_engine::TantivyIndexingEngine,
    language: &str,
    query: &str,
) -> usize {
    engine
        .search(&FullTextSearchQuery {
            tenant_id: TENANT.to_string(),
            repo_id: REPO.to_string(),
            workspace_ids: None,
            branch: BRANCH.to_string(),
            language: language.to_string(),
            query: query.to_string(),
            limit: 10,
            revision: None,
            shape_types: None,
        })
        .map(|hits| hits.len())
        .unwrap_or_else(|e| panic!("search failed: {e}"))
}

#[tokio::test]
async fn rebuild_indexes_base_content_in_the_repository_default_language() -> Result<()> {
    let (storage, dir) = setup_storage("de").await?;

    let tx = storage.begin_context().await?;
    tx.set_tenant_repo(TENANT, REPO)?;
    tx.set_branch(BRANCH)?;
    tx.set_actor("test")?;
    tx.set_message("seed")?;
    tx.set_auth_context(AuthContext::system())?;
    tx.set_validate_schema(false)?;
    tx.add_node(WORKSPACE, &build_node("/fluege", "Flugplan und Abflüge"))
        .await?;
    tx.commit().await?;

    let engine = Arc::new(raisin_indexer::tantivy_engine::TantivyIndexingEngine::new(
        dir.path().join("tantivy"),
        10,
    )?);
    let stats =
        raisin_rocksdb::management::rebuild_fulltext_index(&storage, &engine, TENANT, REPO, BRANCH)
            .await?;
    assert!(stats.items_processed >= 1, "{stats:?}");

    // The rebuilt index serves searches from the same engine (its reader
    // reloads on commit, asynchronously).
    let mut found = 0;
    for _ in 0..50 {
        found = search(&engine, "de", "flu*");
        if found > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(found >= 1, "German content must be found in German");
    assert_eq!(
        search(&engine, "en", "flu*"),
        0,
        "and not filed under English"
    );
    Ok(())
}
