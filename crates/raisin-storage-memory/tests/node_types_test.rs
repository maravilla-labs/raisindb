use chrono::Utc;
use raisin_models::nodes::types::NodeType;
use raisin_storage::{scope::BranchScope, CommitMetadata, NodeTypeRepository, Storage};
use raisin_storage_memory::InMemoryStorage;

const TENANT: &str = "test-tenant";
const REPO: &str = "test-repo";
const BRANCH: &str = "main";

fn scope() -> BranchScope<'static> {
    BranchScope {
        tenant_id: TENANT,
        repo_id: REPO,
        branch: BRANCH,
    }
}

fn commit(message: &str) -> CommitMetadata {
    CommitMetadata::new(message, "test-user")
}

fn build_node_type(name: &str, id: &str) -> NodeType {
    NodeType {
        id: Some(id.to_string()),
        strict: Some(true),
        name: name.to_string(),
        extends: None,
        mixins: Vec::new(),
        overrides: None,
        description: Some(format!("NodeType {}", name)),
        icon: None,
        version: Some(1),
        properties: None,
        allowed_children: Vec::new(),
        required_nodes: Vec::new(),
        initial_structure: None,
        versionable: Some(true),
        publishable: Some(false),
        auditable: Some(true),
        indexable: None,
        index_types: None,
        created_at: Some(Utc::now()),
        updated_at: None,
        published_at: None,
        published_by: None,
        previous_version: None,
        compound_indexes: None,
            is_mixin: None,
    }
}

#[tokio::test]
async fn test_node_type_crud() {
    let storage = InMemoryStorage::default();
    let repo = storage.node_types();

    let mut node_type = build_node_type("test:Type", "test_id");
    node_type.icon = Some("icon.png".to_string());

    repo.put(scope(), node_type.clone(), commit("create test node type"))
        .await
        .expect("Failed to create NodeType");

    let fetched = repo
        .get(scope(), "test:Type", None)
        .await
        .expect("Failed to get NodeType")
        .expect("NodeType not found");
    assert_eq!(fetched.name, "test:Type");
    assert_eq!(fetched.id, Some("test_id".to_string()));

    let fetched_by_id = repo
        .get_by_id(scope(), "test_id", None)
        .await
        .expect("Failed to get NodeType by ID")
        .expect("NodeType not found by ID");
    assert_eq!(fetched_by_id.name, "test:Type");

    let all = repo
        .list(scope(), None)
        .await
        .expect("Failed to list NodeTypes");
    assert_eq!(all.len(), 1);

    let published = repo
        .list_published(scope(), None)
        .await
        .expect("Failed to list published NodeTypes");
    assert_eq!(published.len(), 0);

    let is_pub = repo
        .is_published(scope(), "test:Type", None)
        .await
        .expect("Failed to check published status");
    assert!(!is_pub);

    let validation = repo.validate_published(scope(), "test:Type", None).await;
    assert!(validation.is_err());

    let deleted = repo
        .delete(scope(), "test:Type", commit("delete test node type"))
        .await
        .expect("Failed to delete NodeType");
    assert!(deleted.is_some());

    let should_be_none = repo.get(scope(), "test:Type", None).await.unwrap();
    assert!(should_be_none.is_none());
}

#[tokio::test]
async fn test_node_type_publish_unpublish() {
    let storage = InMemoryStorage::default();
    let repo = storage.node_types();

    let node_type = build_node_type("test:PublishType", "pub_test_id");

    repo.put(scope(), node_type, commit("create publish node type"))
        .await
        .expect("Failed to create NodeType");

    assert!(!repo
        .is_published(scope(), "test:PublishType", None)
        .await
        .unwrap());
    assert_eq!(
        repo.list_published(scope(), None).await.unwrap().len(),
        0
    );

    repo.publish(scope(), "test:PublishType", commit("publish node type"))
        .await
        .expect("Failed to publish");

    assert!(repo
        .is_published(scope(), "test:PublishType", None)
        .await
        .unwrap());
    assert_eq!(
        repo.list_published(scope(), None).await.unwrap().len(),
        1
    );

    repo.validate_published(scope(), "test:PublishType", None)
        .await
        .expect("Validation should succeed");

    repo.unpublish(scope(), "test:PublishType", commit("unpublish node type"))
        .await
        .expect("Failed to unpublish");

    assert!(!repo
        .is_published(scope(), "test:PublishType", None)
        .await
        .unwrap());
    assert_eq!(
        repo.list_published(scope(), None).await.unwrap().len(),
        0
    );

    assert!(repo
        .validate_published(scope(), "test:PublishType", None)
        .await
        .is_err());
}

#[tokio::test]
async fn test_node_type_update() {
    let storage = InMemoryStorage::default();
    let repo = storage.node_types();

    let mut node_type = build_node_type("test:UpdateType", "update_id");
    node_type.description = Some("Initial description".to_string());
    node_type.versionable = Some(false);

    repo.put(scope(), node_type, commit("create update node type"))
        .await
        .unwrap();

    let mut updated_node_type = repo
        .get(scope(), "test:UpdateType", None)
        .await
        .unwrap()
        .unwrap();
    updated_node_type.description = Some("Updated description".to_string());
    updated_node_type.version = Some(2);

    repo.put(scope(), updated_node_type, commit("update node type"))
        .await
        .unwrap();

    let fetched = repo
        .get(scope(), "test:UpdateType", None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fetched.description, Some("Updated description".to_string()));
    assert_eq!(fetched.version, Some(2));
}

#[tokio::test]
async fn test_concurrent_operations() {
    use std::sync::Arc;
    let storage = Arc::new(InMemoryStorage::default());

    let handles: Vec<_> = (0..10)
        .map(|i| {
            let storage = Arc::clone(&storage);
            tokio::spawn(async move {
                let repo = storage.node_types();
                let mut node_type =
                    build_node_type(&format!("test:Type{}", i), &format!("id_{}", i));
                node_type.description = Some(format!("Type {}", i));
                node_type.publishable = Some(i % 2 == 0);

                repo.put(scope(), node_type, commit(&format!("create node type {}", i)))
                    .await
                    .unwrap();
            })
        })
        .collect();

    for handle in handles {
        handle.await.unwrap();
    }

    let repo = storage.node_types();
    let all = repo.list(scope(), None).await.unwrap();
    assert_eq!(all.len(), 10);

    let published = repo.list_published(scope(), None).await.unwrap();
    assert_eq!(published.len(), 5);
}
