use super::*;
use raisin_models::nodes::properties::schema::PropertyType;
use raisin_models::workspace::Workspace;
use raisin_storage::{CommitMetadata, Storage};
use raisin_storage_memory::InMemoryStorage;

async fn create_test_node_type(
    storage: &InMemoryStorage,
    name: &str,
    extends: Option<String>,
    mixins: Option<Vec<String>>,
    properties: Option<Vec<PropertyValueSchema>>,
    allowed_children: Option<Vec<String>>,
) {
    let node_type = NodeType {
        id: Some(name.to_string()),
        strict: Some(false),
        name: name.to_string(),
        extends,
        mixins: mixins.unwrap_or_default(),
        overrides: None,
        description: None,
        icon: None,
        version: Some(1),
        properties,
        allowed_children: allowed_children.unwrap_or_default(),
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

    storage
        .node_types()
        .put(
            "test",
            "main",
            "main",
            node_type,
            CommitMetadata::system("create test node type"),
        )
        .await
        .unwrap();
}

fn create_property(name: &str, property_type: PropertyType) -> PropertyValueSchema {
    PropertyValueSchema {
        name: Some(name.to_string()),
        property_type,
        required: Some(false),
        unique: Some(false),
        default: None,
        constraints: None,
        structure: None,
        items: None,
        value: None,
        meta: None,
        is_translatable: None,
        allow_additional_properties: None,
        index: None,
    }
}

#[tokio::test]
async fn test_simple_resolution_no_inheritance() {
    let storage = Arc::new(InMemoryStorage::default());
    let resolver = NodeTypeResolver::new(
        storage.clone(),
        "test".to_string(),
        "main".to_string(),
        "main".to_string(),
    );

    create_test_node_type(
        &storage,
        "test:Simple",
        None,
        None,
        Some(vec![create_property("title", PropertyType::String)]),
        None,
    )
    .await;

    let resolved = resolver.resolve("test:Simple").await.unwrap();

    assert_eq!(resolved.node_type.name, "test:Simple");
    assert_eq!(resolved.resolved_properties.len(), 1);
    assert_eq!(
        resolved.resolved_properties[0].name,
        Some("title".to_string())
    );
    assert_eq!(resolved.inheritance_chain, vec!["test:Simple"]);
}

#[tokio::test]
async fn test_single_level_inheritance() {
    let storage = Arc::new(InMemoryStorage::default());
    let resolver = NodeTypeResolver::new(
        storage.clone(),
        "test".to_string(),
        "main".to_string(),
        "main".to_string(),
    );

    create_test_node_type(
        &storage,
        "test:Base",
        None,
        None,
        Some(vec![
            create_property("id", PropertyType::String),
            create_property("created_at", PropertyType::Date),
        ]),
        Some(vec!["test:Comment".to_string()]),
    )
    .await;

    create_test_node_type(
        &storage,
        "test:Article",
        Some("test:Base".to_string()),
        None,
        Some(vec![create_property("title", PropertyType::String)]),
        Some(vec!["test:Tag".to_string()]),
    )
    .await;

    let resolved = resolver.resolve("test:Article").await.unwrap();

    assert_eq!(resolved.resolved_properties.len(), 3);
    assert_eq!(resolved.resolved_allowed_children.len(), 2);
    assert_eq!(
        resolved.inheritance_chain,
        vec!["test:Article", "test:Base"]
    );
}

#[tokio::test]
async fn test_multi_level_inheritance() {
    let storage = Arc::new(InMemoryStorage::default());
    let resolver = NodeTypeResolver::new(
        storage.clone(),
        "test".to_string(),
        "main".to_string(),
        "main".to_string(),
    );

    create_test_node_type(
        &storage,
        "test:Entity",
        None,
        None,
        Some(vec![create_property("id", PropertyType::String)]),
        None,
    )
    .await;

    create_test_node_type(
        &storage,
        "test:Content",
        Some("test:Entity".to_string()),
        None,
        Some(vec![create_property("title", PropertyType::String)]),
        None,
    )
    .await;

    create_test_node_type(
        &storage,
        "test:BlogPost",
        Some("test:Content".to_string()),
        None,
        Some(vec![create_property("body", PropertyType::Element)]),
        None,
    )
    .await;

    let resolved = resolver.resolve("test:BlogPost").await.unwrap();

    assert_eq!(resolved.resolved_properties.len(), 3);
    assert_eq!(
        resolved.inheritance_chain,
        vec!["test:BlogPost", "test:Content", "test:Entity"]
    );
}

#[tokio::test]
async fn test_mixin_composition() {
    let storage = Arc::new(InMemoryStorage::default());
    let resolver = NodeTypeResolver::new(
        storage.clone(),
        "test".to_string(),
        "main".to_string(),
        "main".to_string(),
    );

    create_test_node_type(
        &storage,
        "test:Timestamped",
        None,
        None,
        Some(vec![
            create_property("created_at", PropertyType::Date),
            create_property("updated_at", PropertyType::Date),
        ]),
        None,
    )
    .await;

    create_test_node_type(
        &storage,
        "test:Taggable",
        None,
        None,
        Some(vec![create_property("tags", PropertyType::Array)]),
        None,
    )
    .await;

    create_test_node_type(
        &storage,
        "test:Article",
        None,
        Some(vec![
            "test:Timestamped".to_string(),
            "test:Taggable".to_string(),
        ]),
        Some(vec![create_property("title", PropertyType::String)]),
        None,
    )
    .await;

    let resolved = resolver.resolve("test:Article").await.unwrap();

    assert_eq!(resolved.resolved_properties.len(), 4);
}

#[tokio::test]
async fn test_circular_dependency_detection() {
    let storage = Arc::new(InMemoryStorage::default());
    let resolver = NodeTypeResolver::new(
        storage.clone(),
        "test".to_string(),
        "main".to_string(),
        "main".to_string(),
    );

    create_test_node_type(
        &storage,
        "test:TypeA",
        Some("test:TypeB".to_string()),
        None,
        None,
        None,
    )
    .await;

    create_test_node_type(
        &storage,
        "test:TypeB",
        Some("test:TypeA".to_string()),
        None,
        None,
        None,
    )
    .await;

    let result = resolver.resolve("test:TypeA").await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Circular dependency"));
}

#[tokio::test]
async fn test_property_override() {
    let storage = Arc::new(InMemoryStorage::default());
    let resolver = NodeTypeResolver::new(
        storage.clone(),
        "test".to_string(),
        "main".to_string(),
        "main".to_string(),
    );

    create_test_node_type(
        &storage,
        "test:Base",
        None,
        None,
        Some(vec![create_property("title", PropertyType::String)]),
        None,
    )
    .await;

    create_test_node_type(
        &storage,
        "test:Article",
        Some("test:Base".to_string()),
        None,
        Some(vec![create_property("title", PropertyType::Element)]),
        None,
    )
    .await;

    let resolved = resolver.resolve("test:Article").await.unwrap();

    assert_eq!(resolved.resolved_properties.len(), 1);
    assert_eq!(
        resolved.resolved_properties[0].property_type,
        PropertyType::Element
    );
}

#[tokio::test]
async fn test_workspace_pin_resolves_specific_version() {
    let storage = Arc::new(InMemoryStorage::default());
    let resolver = NodeTypeResolver::new(
        storage.clone(),
        "test".to_string(),
        "main".to_string(),
        "main".to_string(),
    );

    create_test_node_type(
        &storage,
        "test:Article",
        None,
        None,
        Some(vec![create_property("title", PropertyType::String)]),
        None,
    )
    .await;

    create_test_node_type(
        &storage,
        "test:Article",
        None,
        None,
        Some(vec![create_property("title", PropertyType::Element)]),
        None,
    )
    .await;

    let mut pinned_workspace = Workspace::new("pinned".to_string());
    pinned_workspace
        .config
        .node_type_pins
        .insert("test:Article".to_string(), Some(raisin_hlc::HLC::new(1, 0)));
    storage
        .workspaces()
        .put("test", "main", pinned_workspace)
        .await
        .unwrap();

    let latest_workspace = Workspace::new("latest".to_string());
    storage
        .workspaces()
        .put("test", "main", latest_workspace)
        .await
        .unwrap();

    let resolved_pinned = resolver
        .resolve_for_workspace("pinned", "test:Article")
        .await
        .unwrap();
    assert_eq!(resolved_pinned.node_type.version, Some(1));
    assert_eq!(
        resolved_pinned.resolved_properties[0].property_type,
        PropertyType::String
    );

    let resolved_latest = resolver
        .resolve_for_workspace("latest", "test:Article")
        .await
        .unwrap();
    assert_eq!(resolved_latest.node_type.version, Some(2));
    assert_eq!(
        resolved_latest.resolved_properties[0].property_type,
        PropertyType::Element
    );
}
