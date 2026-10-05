//! Plan Phase 12, SQL surface: `RESOLVE_PATH(workspace, locale, path)`, the
//! `__node_name` / `__localized_path` columns, the `LocalizedPathLookup` operator
//! for `WHERE locale = … AND __localized_path = …`, and a bound locale
//! parameter — all over the one storage lookup the HTTP and WS surfaces use.

use futures::StreamExt;
use raisin_context::RepositoryConfig;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::translations::{JsonPointer, LocaleCode};
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    BranchRepository, CreateNodeOptions, NodeRepository, RegistryRepository, RepoScope,
    RepositoryManagementRepository, Storage, StorageScope, WorkspaceRepository,
};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

pub(crate) type Store = raisin_rocksdb::RocksDBStorage;
pub(crate) const T: &str = "lp_tenant";
pub(crate) const R: &str = "lp_repo";
pub(crate) const B: &str = "main";
pub(crate) const WS: &str = "pages";

pub(crate) fn config() -> RepositoryConfig {
    RepositoryConfig {
        default_language: "en".to_string(),
        supported_languages: vec!["en".into(), "fr".into()],
        default_branch: B.to_string(),
        ..RepositoryConfig::default()
    }
}

async fn setup() -> (Arc<Store>, TempDir, String) {
    setup_with(config()).await
}

/// `/products` (fr `produits`) and `/products/chair` (fr `chaise`) under
/// `repository`; returns the chair's id.
pub(crate) async fn setup_with(repository: RepositoryConfig) -> (Arc<Store>, TempDir, String) {
    let dir = TempDir::new().unwrap();
    let storage = Arc::new(Store::new(dir.path()).unwrap());
    storage
        .registry()
        .register_tenant(T, HashMap::new())
        .await
        .unwrap();
    storage
        .repository_management()
        .create_repository(T, R, repository)
        .await
        .unwrap();
    storage
        .branches()
        .create_branch(T, R, B, "test", None, None, false, false)
        .await
        .unwrap();
    storage
        .workspaces()
        .put(
            RepoScope::new(T, R),
            raisin_models::workspace::Workspace::new(WS.to_string()),
        )
        .await
        .unwrap();
    let mut chair = String::new();
    for (path, translated) in [("/products", "produits"), ("/products/chair", "chaise")] {
        let node = Node {
            id: uuid::Uuid::new_v4().to_string(),
            name: path.rsplit('/').next().unwrap().to_string(),
            path: path.to_string(),
            parent: (path.matches('/').count() > 1).then(|| "products".to_string()),
            node_type: "raisin:Folder".to_string(),
            created_at: Some(chrono::Utc::now()),
            ..Node::default()
        };
        let id = node.id.clone();
        storage
            .nodes()
            .create(
                StorageScope::new(T, R, B, WS),
                node,
                CreateNodeOptions {
                    validate_schema: false,
                    validate_parent_allows_child: false,
                    validate_workspace_allows_type: false,
                    operation_meta: None,
                },
            )
            .await
            .unwrap();
        let data = HashMap::from([(
            JsonPointer::new("/__node_name"),
            PropertyValue::String(translated.to_string()),
        )]);
        raisin_core::TranslationService::new(storage.clone())
            .update_translation(
                T,
                R,
                B,
                WS,
                &id,
                &LocaleCode::parse("fr").unwrap(),
                data,
                "t",
                None,
            )
            .await
            .unwrap();
        chair = id;
    }
    (storage, dir, chair)
}

pub(crate) fn engine(storage: &Arc<Store>) -> QueryEngine<Store> {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    QueryEngine::new(storage.clone(), T, R, B)
        .with_catalog(Arc::new(catalog))
        .with_repository_config(config())
        .with_auth(AuthContext::system())
}

pub(crate) async fn rows(
    engine: &QueryEngine<Store>,
    sql: &str,
) -> Vec<HashMap<String, PropertyValue>> {
    let mut stream = engine
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("query failed [{sql}]: {e}"));
    let mut out = Vec::new();
    while let Some(row) = stream.next().await {
        let row = row.unwrap_or_else(|e| panic!("row error [{sql}]: {e}"));
        out.push(row.columns.into_iter().collect());
    }
    out
}

fn text(row: &HashMap<String, PropertyValue>, column: &str) -> Option<String> {
    row.iter()
        .find(|(k, _)| k.as_str() == column || k.ends_with(&format!(".{column}")))
        .and_then(|(_, v)| match v {
            PropertyValue::String(s) => Some(s.clone()),
            _ => None,
        })
}

#[tokio::test]
async fn resolve_path_returns_the_id() {
    let (storage, _dir, chair) = setup().await;
    let engine = engine(&storage);
    let out = rows(
        &engine,
        &format!(
            "SELECT RESOLVE_PATH('{WS}', 'fr', '/produits/chaise') AS id FROM '{WS}' \
             WHERE path = '/products'"
        ),
    )
    .await;
    assert_eq!(text(&out[0], "id"), Some(chair));
    let none = rows(
        &engine,
        &format!(
            "SELECT RESOLVE_PATH('{WS}', 'fr', '/produits/nope') AS id FROM '{WS}' \
             WHERE path = '/products'"
        ),
    )
    .await;
    assert_eq!(text(&none[0], "id"), None);
}

#[tokio::test]
async fn node_name_and_localized_path_columns() {
    let (storage, _dir, chair) = setup().await;
    let engine = engine(&storage);
    let out = rows(
        &engine,
        &format!(
            "SELECT id, __node_name, __localized_path FROM '{WS}' \
             WHERE locale = 'fr' AND path = '/products/chair'"
        ),
    )
    .await;
    assert_eq!(out.len(), 1);
    assert_eq!(text(&out[0], "id"), Some(chair));
    assert_eq!(text(&out[0], "__node_name").as_deref(), Some("chaise"));
    assert_eq!(
        text(&out[0], "__localized_path").as_deref(),
        Some("/produits/chaise")
    );
    // Opt-in: not part of `SELECT *`.
    let star = rows(
        &engine,
        &format!("SELECT * FROM '{WS}' WHERE path = '/products/chair'"),
    )
    .await;
    assert!(text(&star[0], "__localized_path").is_none());
}

#[tokio::test]
async fn localized_path_lookup_operator_and_bound_locale() {
    let (storage, _dir, chair) = setup().await;
    let engine = engine(&storage);
    let sql = raisin_sql::substitute_params(
        &format!("SELECT id FROM '{WS}' WHERE locale = $1 AND __localized_path = $2"),
        &[
            serde_json::json!("fr"),
            serde_json::json!("/produits/chaise"),
        ],
    )
    .unwrap();
    let plan = rows(&engine, &format!("EXPLAIN {sql}")).await;
    let plan = format!("{plan:?}");
    assert!(plan.contains("LocalizedPathLookup"), "{plan}");
    let out = rows(&engine, &sql).await;
    assert_eq!(out.len(), 1, "{out:?}");
    assert_eq!(text(&out[0], "id"), Some(chair));
    // The operator IS the predicate: a canonical name where the node has a
    // translated name is not its localized path.
    let canonical = rows(
        &engine,
        &format!(
            "SELECT id FROM '{WS}' WHERE locale = 'fr' AND __localized_path = '/products/chair'"
        ),
    )
    .await;
    assert!(canonical.is_empty(), "{canonical:?}");
}

#[tokio::test]
async fn a_node_name_set_through_update_for_locale_resolves() {
    let (storage, _dir, chair) = setup().await;
    let engine = engine(&storage);
    rows(
        &engine,
        &format!(
            "UPDATE {WS} FOR LOCALE 'fr' SET __node_name = 'siege' WHERE path = '/products/chair'"
        ),
    )
    .await;
    let out = rows(
        &engine,
        &format!(
            "SELECT RESOLVE_PATH('{WS}', 'fr', '/produits/siege') AS id FROM '{WS}' \
             WHERE path = '/products'"
        ),
    )
    .await;
    assert_eq!(text(&out[0], "id"), Some(chair));
}
