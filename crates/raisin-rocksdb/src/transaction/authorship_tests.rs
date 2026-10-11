//! Authorship is a server stamp: a client can never set who created a node.

use crate::RocksDBStorage;
use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_models::nodes::Node;
use raisin_models::permissions::ResolvedPermissions;
use raisin_models::workspace::Workspace;
use raisin_storage::scope::{BranchScope, StorageScope};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{
    BranchRepository, CommitMetadata, NodeRepository, NodeTypeRepository, RegistryRepository,
    RepositoryManagementRepository, Storage,
};
use std::collections::HashMap;
use std::sync::Arc;

const T: &str = "auth-t";
const R: &str = "auth-r";
const B: &str = "main";
const WS: &str = "content";

async fn storage() -> (tempfile::TempDir, Arc<RocksDBStorage>) {
    let dir = tempfile::tempdir().unwrap();
    let s = Arc::new(RocksDBStorage::new(dir.path()).unwrap());
    s.registry()
        .register_tenant(T, HashMap::new())
        .await
        .unwrap();
    s.repository_management()
        .create_repository(
            T,
            R,
            RepositoryConfig {
                default_language: "en".into(),
                supported_languages: vec!["en".into()],
                locale_fallback_chains: HashMap::new(),
                default_branch: B.into(),
                description: None,
                tags: HashMap::new(),
                localized_names: Default::default(),
            },
        )
        .await
        .unwrap();
    s.branches()
        .create_branch(T, R, B, "system", None, None, false, false)
        .await
        .unwrap();
    let folder: NodeType = serde_json::from_value(serde_json::json!({
        "name": "raisin:Folder", "strict": false, "allowed_children": ["*"]
    }))
    .unwrap();
    s.node_types()
        .upsert(
            BranchScope::new(T, R, B),
            folder,
            CommitMetadata::system("seed"),
        )
        .await
        .unwrap();
    let mut w = Workspace::new(WS.to_string());
    w.config.default_branch = B.to_string();
    WorkspaceService::new(s.clone()).put(T, R, w).await.unwrap();
    (dir, s)
}

fn person(id: &str) -> AuthContext {
    AuthContext::for_user(id).with_permissions(ResolvedPermissions::system_admin())
}

async fn write(s: &RocksDBStorage, as_user: &str, node: &Node, trusted: bool, create: bool) {
    let tx = s.begin_context().await.unwrap();
    tx.set_tenant_repo(T, R).unwrap();
    tx.set_branch(B).unwrap();
    tx.set_actor(as_user).unwrap();
    tx.set_auth_context(person(as_user)).unwrap();
    tx.set_validate_schema(false).unwrap();
    tx.set_trusted_import(trusted).unwrap();
    if create {
        tx.add_node(WS, node).await.unwrap();
    } else {
        tx.put_node(WS, node).await.unwrap();
    }
    tx.commit().await.unwrap();
}

fn node(id: &str, name: &str) -> Node {
    Node {
        id: id.into(),
        name: name.into(),
        path: format!("/{name}"),
        parent: Some("/".into()),
        node_type: "raisin:Folder".into(),
        ..Default::default()
    }
}

async fn read(s: &RocksDBStorage, path: &str) -> Node {
    s.nodes()
        .get_by_path(StorageScope::new(T, R, B, WS), path, None)
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn a_client_cannot_claim_someone_else_wrote_a_node() {
    let (_dir, s) = storage().await;

    // Create, claiming alice wrote it.
    let mut forged = node("n1", "forged");
    forged.created_by = Some("alice".into());
    forged.updated_by = Some("alice".into());
    write(&s, "mallory", &forged, false, true).await;
    let got = read(&s, "/forged").await;
    assert_eq!(got.created_by.as_deref(), Some("mallory"));
    assert_eq!(got.updated_by.as_deref(), Some("mallory"));

    // Update (put), trying to rewrite the creator.
    let mut again = got.clone();
    again.created_by = Some("alice".into());
    write(&s, "bob", &again, false, false).await;
    let got = read(&s, "/forged").await;
    assert_eq!(got.created_by.as_deref(), Some("mallory"));
    assert_eq!(got.updated_by.as_deref(), Some("bob"));

    // A put that creates is stamped too.
    let mut put_created = node("n2", "put-created");
    put_created.created_by = Some("alice".into());
    write(&s, "mallory", &put_created, false, false).await;
    assert_eq!(
        read(&s, "/put-created").await.created_by.as_deref(),
        Some("mallory")
    );
}

#[tokio::test]
async fn a_trusted_import_keeps_supplied_authorship() {
    let (_dir, s) = storage().await;
    let mut imported = node("n3", "imported");
    imported.created_by = Some("alice".into());
    imported.updated_by = Some("alice".into());
    write(&s, "importer", &imported, true, true).await;
    let got = read(&s, "/imported").await;
    assert_eq!(got.created_by.as_deref(), Some("alice"));
    assert_eq!(got.updated_by.as_deref(), Some("alice"));
}
