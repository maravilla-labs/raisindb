//! Shared fixture for the localized name index tests (plan Phase 12).

use raisin_context::RepositoryConfig;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::translations::{JsonPointer, LocaleCode};
use raisin_rocksdb::localized_name::{config, rows, state, Availability, NameScope};
use raisin_rocksdb::management::async_indexing::repair::{
    run_repair, RepairKind, RepairOptions, RepairReport,
};
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::localized::{LocalizedResolution, LocalizedServedBy};
use raisin_storage::{
    BranchRepository, CreateNodeOptions, NodeRepository, RegistryRepository, RepoScope,
    RepositoryManagementRepository, Storage, StorageScope, UpdateNodeOptions, WorkspaceRepository,
};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

pub const T: &str = "lname-tenant";
pub const R: &str = "site";
pub const B: &str = "main";
pub const WS: &str = "pages";

pub fn repo_config() -> RepositoryConfig {
    RepositoryConfig {
        default_language: "en".to_string(),
        supported_languages: vec!["en".into(), "fr".into(), "de".into()],
        default_branch: B.to_string(),
        ..RepositoryConfig::default()
    }
}

/// Tenant, repository (`en` default; `fr`, `de`), branch and workspace on
/// `storage`.
pub async fn provision(storage: &RocksDBStorage) {
    storage
        .registry()
        .register_tenant(T, HashMap::new())
        .await
        .unwrap();
    storage
        .repository_management()
        .create_repository(T, R, repo_config())
        .await
        .unwrap();
    storage
        .branches()
        .create_branch(T, R, B, "system", None, None, false, false)
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
}

pub async fn open() -> (Arc<RocksDBStorage>, TempDir) {
    let dir = TempDir::new().unwrap();
    let storage = Arc::new(RocksDBStorage::new(dir.path()).unwrap());
    provision(&storage).await;
    (storage, dir)
}

pub fn scope(branch: &str) -> StorageScope<'_> {
    StorageScope::new(T, R, branch, WS)
}

pub fn names(branch: &str) -> NameScope<'_> {
    NameScope::new(T, R, branch, WS)
}

pub fn page(path: &str, props: &[(&str, &str)]) -> Node {
    let name = path.rsplit('/').next().unwrap().to_string();
    let parent = match path.rsplitn(2, '/').nth(1) {
        Some(p) if !p.is_empty() => Some(p.to_string()),
        _ => None,
    };
    let mut node = Node {
        id: uuid::Uuid::new_v4().to_string(),
        path: path.to_string(),
        name,
        parent,
        node_type: "raisin:Folder".to_string(),
        created_at: Some(chrono::Utc::now()),
        ..Node::default()
    };
    for (k, v) in props {
        node.properties
            .insert(k.to_string(), PropertyValue::String(v.to_string()));
    }
    node
}

pub async fn create_on(storage: &RocksDBStorage, branch: &str, node: Node) -> String {
    let id = node.id.clone();
    storage
        .nodes()
        .create(
            scope(branch),
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
    id
}

pub async fn create(storage: &RocksDBStorage, path: &str, props: &[(&str, &str)]) -> String {
    create_on(storage, B, page(path, props)).await
}

pub async fn get(storage: &RocksDBStorage, id: &str) -> Node {
    storage
        .nodes()
        .get(scope(B), id, None)
        .await
        .unwrap()
        .unwrap()
}

pub async fn update_props(storage: &RocksDBStorage, id: &str, props: &[(&str, &str)]) {
    let mut node = get(storage, id).await;
    for (k, v) in props {
        node.properties
            .insert(k.to_string(), PropertyValue::String(v.to_string()));
    }
    storage
        .nodes()
        .update(
            scope(B),
            node,
            UpdateNodeOptions {
                validate_schema: false,
                ..UpdateNodeOptions::default()
            },
        )
        .await
        .unwrap();
}

pub fn translations(
    storage: &Arc<RocksDBStorage>,
) -> raisin_core::TranslationService<RocksDBStorage> {
    raisin_core::TranslationService::new(storage.clone())
}

/// Give `id` the translated name `name` in `locale` (its `/__node_name`
/// overlay pointer).
pub async fn set_name(storage: &Arc<RocksDBStorage>, id: &str, locale: &str, name: &str) {
    try_set_name(storage, id, locale, name).await.unwrap();
}

pub async fn try_set_name(
    storage: &Arc<RocksDBStorage>,
    id: &str,
    locale: &str,
    name: &str,
) -> raisin_error::Result<()> {
    let data = HashMap::from([(
        JsonPointer::new("/__node_name"),
        PropertyValue::String(name.to_string()),
    )]);
    translations(storage)
        .update_translation(T, R, B, WS, id, &code(locale), data, "test", None)
        .await
        .map(|_| ())
}

pub async fn hide(storage: &Arc<RocksDBStorage>, id: &str, locale: &str) {
    translations(storage)
        .hide_node(T, R, B, WS, id, &code(locale), "test", None)
        .await
        .unwrap();
}

pub fn code(locale: &str) -> LocaleCode {
    LocaleCode::parse(locale).unwrap()
}

pub async fn head(storage: &RocksDBStorage, branch: &str) -> HLC {
    storage.branches().get_head(T, R, branch).await.unwrap()
}

pub fn resolve_at(
    storage: &RocksDBStorage,
    branch: &str,
    locale: &str,
    path: &str,
    at: Option<&HLC>,
) -> Option<LocalizedResolution> {
    storage
        .localized_names()
        .unwrap()
        .resolve(scope(branch), locale, path, at)
        .unwrap()
}

pub fn resolve(storage: &RocksDBStorage, locale: &str, path: &str) -> Option<LocalizedResolution> {
    resolve_at(storage, B, locale, path, None)
}

/// The id a lookup answers, asserting how it was served.
pub fn id_via(
    storage: &RocksDBStorage,
    branch: &str,
    locale: &str,
    path: &str,
    served: LocalizedServedBy,
) -> Option<String> {
    resolve_at(storage, branch, locale, path, None).map(|r| {
        assert_eq!(r.served_by, served, "{locale} {path} on {branch}");
        r.node_id
    })
}

pub fn availability(storage: &RocksDBStorage, branch: &str) -> Availability {
    let db = storage.db();
    let fingerprint = config::load(db, T, R).unwrap().unwrap().fingerprint();
    let record = state::read(db, T, R, branch, WS).unwrap();
    state::availability(record.as_ref(), &fingerprint, None)
}

pub fn build_options() -> RepairOptions {
    RepairOptions {
        check_headroom: false,
        max_bytes_per_sec: 0,
        ..RepairOptions::default()
    }
}

pub async fn build(storage: &RocksDBStorage, branch: &str) -> RepairReport {
    let mut reports = run_repair(
        storage,
        T,
        R,
        Some(branch),
        RepairKind::LocalizedNames,
        build_options(),
    )
    .await
    .unwrap();
    let report = reports.remove(0);
    assert!(report.completed, "{report:?}");
    report
}

/// The live claims on `(locale, parent, name)` at HEAD.
pub fn claimants(
    storage: &RocksDBStorage,
    branch: &str,
    locale: &str,
    parent_id: &str,
    name: &str,
) -> Vec<String> {
    rows::claims(storage.db(), names(branch), locale, parent_id, name, None)
        .unwrap()
        .into_iter()
        .map(|(_, id)| id)
        .collect()
}

pub async fn set_config(storage: &RocksDBStorage, config: RepositoryConfig) {
    storage
        .repository_management()
        .update_repository_config(T, R, config)
        .await
        .unwrap();
}

/// `/products` (fr `produits`) and `/products/chair` (fr `chaise`).
pub async fn catalog(storage: &Arc<RocksDBStorage>) -> (String, String) {
    let products = create(storage, "/products", &[]).await;
    let chair = create(storage, "/products/chair", &[]).await;
    set_name(storage, &products, "fr", "produits").await;
    set_name(storage, &chair, "fr", "chaise").await;
    (products, chair)
}

/// A `/__node_name` overlay.
pub fn name_overlay(name: &str) -> raisin_models::translations::LocaleOverlay {
    raisin_models::translations::LocaleOverlay::properties(HashMap::from([(
        JsonPointer::new("/__node_name"),
        PropertyValue::String(name.to_string()),
    )]))
}

/// A system transaction on the fixture branch.
pub async fn begin(
    storage: &RocksDBStorage,
) -> Box<dyn raisin_storage::transactional::TransactionalContext> {
    use raisin_storage::transactional::TransactionalStorage;
    let tx = storage.begin_context().await.unwrap();
    tx.set_tenant_repo(T, R).unwrap();
    tx.set_branch(B).unwrap();
    tx.set_actor("test").unwrap();
    tx.set_auth_context(raisin_models::auth::AuthContext::system())
        .unwrap();
    tx.set_validate_schema(false).unwrap();
    tx
}
