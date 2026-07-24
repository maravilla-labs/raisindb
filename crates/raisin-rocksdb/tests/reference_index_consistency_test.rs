//! Reverse-reference index consistency tests.
//!
//! Regression tests for the `REFERENCES()` false-positive bug: the reverse
//! reference index must return EXACTLY the nodes that currently reference a
//! target — after batched transaction creates, after updates that remove or
//! retarget a reference (both the transaction and repository write paths),
//! and after deletes. See references-index-bug report (v0.1.76): 4 of 9 tags
//! over-reported and a full node rewrite never cleared the stale entries.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_core::NodeService;
use raisin_error::Result;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::{PropertyValue, RaisinReference};
use raisin_models::nodes::Node;
use raisin_models::workspace::Workspace;
use raisin_rocksdb::{fractional_index, RocksDBConfig, RocksDBStorage};
use raisin_storage::{
    BranchRepository, CreateNodeOptions, DeleteNodeOptions, NodeRepository,
    ReferenceIndexRepository, RegistryRepository, RepositoryManagementRepository, Storage,
    StorageScope, UpdateNodeOptions,
};
use tempfile::TempDir;

const TENANT: &str = "refidx-test";
const REPO: &str = "repo";
const BRANCH: &str = "main";
const TAGS_WS: &str = "tags";
const STORIES_WS: &str = "stories";

struct Env {
    _dir: TempDir,
    storage: Arc<RocksDBStorage>,
}

impl Env {
    async fn new() -> Result<Self> {
        let temp_dir = TempDir::new().unwrap();
        let mut config = RocksDBConfig::default();
        config.path = temp_dir.path().to_path_buf();
        let storage = Arc::new(RocksDBStorage::with_config(config)?);

        storage
            .registry()
            .register_tenant(TENANT, HashMap::new())
            .await?;
        storage
            .repository_management()
            .create_repository(
                TENANT,
                REPO,
                RepositoryConfig {
                    default_language: "en".to_string(),
                    supported_languages: vec!["en".to_string()],
                    locale_fallback_chains: HashMap::new(),
                    default_branch: BRANCH.to_string(),
                    description: None,
                    tags: HashMap::new(),
                },
            )
            .await?;
        storage
            .branches()
            .create_branch(TENANT, REPO, BRANCH, "system", None, None, false, false)
            .await?;

        let workspace_service = WorkspaceService::new(storage.clone());
        for ws in [TAGS_WS, STORIES_WS] {
            let mut workspace = Workspace::new(ws.to_string());
            workspace.config.default_branch = BRANCH.to_string();
            workspace_service.put(TENANT, REPO, workspace).await?;
        }

        for type_name in ["studio:Tag", "studio:Page"] {
            let node_type = raisin_models::nodes::types::node_type::NodeType {
                id: Some(type_name.to_string()),
                strict: Some(false),
                name: type_name.to_string(),
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
            raisin_storage::NodeTypeRepository::upsert(
                &*Storage::node_types(storage.as_ref()),
                raisin_storage::BranchScope::new(TENANT, REPO, BRANCH),
                node_type,
                raisin_storage::CommitMetadata::system("seed type"),
            )
            .await?;
        }

        Ok(Self {
            _dir: temp_dir,
            storage,
        })
    }
}

fn relaxed_create() -> CreateNodeOptions {
    CreateNodeOptions {
        validate_schema: false,
        validate_parent_allows_child: false,
        validate_workspace_allows_type: false,
        operation_meta: None,
    }
}

fn build_node(
    id: &str,
    path: &str,
    node_type: &str,
    workspace: &str,
    properties: HashMap<String, PropertyValue>,
) -> Node {
    let name = path.rsplit('/').next().unwrap_or(path).to_string();
    Node {
        id: id.to_string(),
        name,
        path: path.to_string(),
        node_type: node_type.to_string(),
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
        updated_by: Some("tester".to_string()),
        created_by: Some("tester".to_string()),
        translations: None,
        tenant_id: Some(TENANT.to_string()),
        workspace: Some(workspace.to_string()),
        owner_id: None,
        relations: Vec::new(),
    }
}

fn tags_property(tag_ids: &[&str]) -> HashMap<String, PropertyValue> {
    let refs = tag_ids
        .iter()
        .map(|tag_id| {
            PropertyValue::Reference(RaisinReference {
                id: tag_id.to_string(),
                workspace: TAGS_WS.to_string(),
                path: format!("/{}", tag_id),
            })
        })
        .collect();
    let mut props = HashMap::new();
    props.insert("tags".to_string(), PropertyValue::Array(refs));
    props.insert(
        "title".to_string(),
        PropertyValue::String("page".to_string()),
    );
    props
}

async fn create_tags(env: &Env, tag_ids: &[&str]) -> Result<()> {
    for tag_id in tag_ids {
        env.storage
            .nodes()
            .create(
                StorageScope::new(TENANT, REPO, BRANCH, TAGS_WS),
                build_node(
                    tag_id,
                    &format!("/{}", tag_id),
                    "studio:Tag",
                    TAGS_WS,
                    HashMap::new(),
                ),
                relaxed_create(),
            )
            .await?;
    }
    Ok(())
}

/// The exact set of source node ids currently referencing `tag_id`.
async fn referrers(env: &Env, tag_id: &str) -> Vec<String> {
    let mut ids: Vec<String> = env
        .storage
        .reference_index()
        .find_referencing_nodes(
            StorageScope::new(TENANT, REPO, BRANCH, STORIES_WS),
            TAGS_WS,
            tag_id,
            false,
        )
        .await
        .expect("reverse reference scan")
        .into_iter()
        .map(|(source_id, _prop)| source_id)
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

async fn assert_referrers(env: &Env, tag_id: &str, expected: &[&str]) {
    let mut want: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
    want.sort();
    let got = referrers(env, tag_id).await;
    assert_eq!(
        got, want,
        "referrers of tag '{}' — REFERENCES must return exactly the current referrers",
        tag_id
    );
}

fn node_service(env: &Env) -> NodeService<RocksDBStorage> {
    NodeService::new_with_context(
        env.storage.clone(),
        TENANT.to_string(),
        REPO.to_string(),
        BRANCH.to_string(),
        STORIES_WS.to_string(),
    )
    .with_auth(AuthContext::system())
}

/// Batched transaction creates (the bug report's write pattern: six sibling
/// pages in one push) must attribute each reference to exactly its own node.
#[tokio::test]
async fn batch_created_nodes_have_exact_reference_sets() -> Result<()> {
    let env = Env::new().await?;
    create_tags(&env, &["agents", "media", "deploy"]).await?;

    let service = node_service(&env);
    let mut tx = service.transaction();
    tx.create(build_node(
        "p1",
        "/p1",
        "studio:Page",
        STORIES_WS,
        tags_property(&["agents", "media"]),
    ));
    tx.create(build_node(
        "p2",
        "/p2",
        "studio:Page",
        STORIES_WS,
        tags_property(&["media", "deploy"]),
    ));
    tx.create(build_node(
        "p3",
        "/p3",
        "studio:Page",
        STORIES_WS,
        tags_property(&["deploy"]),
    ));
    tx.create(build_node(
        "p4",
        "/p4",
        "studio:Page",
        STORIES_WS,
        tags_property(&["agents"]),
    ));
    tx.commit("batch create pages", "tester").await?;

    assert_referrers(&env, "agents", &["p1", "p4"]).await;
    assert_referrers(&env, "media", &["p1", "p2"]).await;
    assert_referrers(&env, "deploy", &["p2", "p3"]).await;
    Ok(())
}

/// Removing a reference via the transaction (put_node) update path must stop
/// the node from matching — the report's "rewrite doesn't clear it" case.
#[tokio::test]
async fn tx_update_removing_reference_prunes_index() -> Result<()> {
    let env = Env::new().await?;
    create_tags(&env, &["agents", "media"]).await?;

    let service = node_service(&env);
    let mut tx = service.transaction();
    tx.create(build_node(
        "page",
        "/page",
        "studio:Page",
        STORIES_WS,
        tags_property(&["agents", "media"]),
    ));
    tx.commit("create page", "tester").await?;

    assert_referrers(&env, "agents", &["page"]).await;
    assert_referrers(&env, "media", &["page"]).await;

    // Rewrite the node keeping only the "agents" tag.
    let mut update_tx = service.transaction();
    update_tx.update(
        "page".to_string(),
        serde_json::json!({
            "tags": [
                {"raisin:ref": "agents", "raisin:workspace": TAGS_WS, "raisin:path": "/agents"}
            ]
        }),
    );
    update_tx.commit("drop media tag", "tester").await?;

    assert_referrers(&env, "agents", &["page"]).await;
    assert_referrers(&env, "media", &[]).await;
    Ok(())
}

/// Removing a reference via the direct repository update path must also prune.
#[tokio::test]
async fn repo_update_removing_reference_prunes_index() -> Result<()> {
    let env = Env::new().await?;
    create_tags(&env, &["agents", "media"]).await?;

    let scope = || StorageScope::new(TENANT, REPO, BRANCH, STORIES_WS);
    env.storage
        .nodes()
        .create(
            scope(),
            build_node(
                "page",
                "/page",
                "studio:Page",
                STORIES_WS,
                tags_property(&["agents", "media"]),
            ),
            relaxed_create(),
        )
        .await?;

    assert_referrers(&env, "agents", &["page"]).await;
    assert_referrers(&env, "media", &["page"]).await;

    let mut updated = env
        .storage
        .nodes()
        .get(scope(), "page", None)
        .await?
        .expect("page exists");
    updated.properties = tags_property(&["agents"]);
    env.storage
        .nodes()
        .update(
            scope(),
            updated,
            UpdateNodeOptions {
                validate_schema: false,
                allow_type_change: false,
                operation_meta: None,
            },
        )
        .await?;

    assert_referrers(&env, "agents", &["page"]).await;
    assert_referrers(&env, "media", &[]).await;
    Ok(())
}

/// Deleting a node must remove it from the reverse-reference index.
#[tokio::test]
async fn delete_prunes_reference_index() -> Result<()> {
    let env = Env::new().await?;
    create_tags(&env, &["agents"]).await?;

    let scope = || StorageScope::new(TENANT, REPO, BRANCH, STORIES_WS);
    env.storage
        .nodes()
        .create(
            scope(),
            build_node(
                "page",
                "/page",
                "studio:Page",
                STORIES_WS,
                tags_property(&["agents"]),
            ),
            relaxed_create(),
        )
        .await?;
    assert_referrers(&env, "agents", &["page"]).await;

    env.storage
        .nodes()
        .delete(scope(), "page", DeleteNodeOptions::default())
        .await?;
    assert_referrers(&env, "agents", &[]).await;
    Ok(())
}

/// Array references written through the direct repository path must be indexed
/// (the repo writer previously only indexed top-level references).
#[tokio::test]
async fn repo_create_indexes_array_references() -> Result<()> {
    let env = Env::new().await?;
    create_tags(&env, &["agents", "media"]).await?;

    env.storage
        .nodes()
        .create(
            StorageScope::new(TENANT, REPO, BRANCH, STORIES_WS),
            build_node(
                "page",
                "/page",
                "studio:Page",
                STORIES_WS,
                tags_property(&["agents", "media"]),
            ),
            relaxed_create(),
        )
        .await?;

    assert_referrers(&env, "agents", &["page"]).await;
    assert_referrers(&env, "media", &["page"]).await;
    Ok(())
}
