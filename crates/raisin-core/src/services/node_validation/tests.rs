//! Tests for NodeValidator.

use super::*;
use raisin_models::nodes::properties::schema::{PropertyType, PropertyValueSchema};
use raisin_models::nodes::properties::value::{Composite, Element, PropertyValue};
use raisin_models::nodes::types::archetype::Archetype;
use raisin_models::nodes::types::element::element_type::ElementType;
use raisin_models::nodes::types::element::field_types::FieldSchema as ElementFieldSchema;
use raisin_models::nodes::types::element::fields::base_field::FieldTypeSchema;
use raisin_models::nodes::types::NodeType;
use raisin_storage::{
    ArchetypeRepository, BranchScope, CommitMetadata, ElementTypeRepository, NodeRepository,
    NodeTypeRepository, Storage, StorageScope,
};
use raisin_storage_memory::InMemoryStorage;
use std::collections::HashMap;
use std::sync::Arc;

async fn setup_test_storage() -> Arc<InMemoryStorage> {
    Arc::new(InMemoryStorage::default())
}

async fn create_node_type(
    storage: &InMemoryStorage,
    name: &str,
    properties: Vec<PropertyValueSchema>,
    strict: bool,
) {
    let node_type = NodeType {
        id: Some(name.to_string()),
        strict: Some(strict),
        name: name.to_string(),
        extends: None,
        mixins: Vec::new(),
        overrides: None,
        description: None,
        icon: None,
        version: Some(1),
        properties: Some(properties),
        allowed_children: Vec::new(),
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
            BranchScope::new("default", "default", "main"),
            node_type,
            CommitMetadata::system("create test node type"),
        )
        .await
        .unwrap();
}

fn make_field_base(name: &str, required: bool, multiple: bool) -> FieldTypeSchema {
    FieldTypeSchema {
        name: name.to_string(),
        title: None,
        label: None,
        required: if required { Some(true) } else { None },
        description: None,
        help_text: None,
        default_value: None,
        validations: None,
        is_hidden: None,
        multiple: if multiple { Some(true) } else { None },
        design_value: None,
        translatable: None,
    }
}

fn create_test_node(
    node_type: &str,
    properties: HashMap<String, PropertyValue>,
) -> raisin_models::nodes::Node {
    raisin_models::nodes::Node {
        id: "test-node-1".to_string(),
        name: "Test Node".to_string(),
        path: "/test-node".to_string(),
        node_type: node_type.to_string(),
        archetype: None,
        properties,
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
        workspace: Some("ws1".to_string()),
        owner_id: None,
        relations: Vec::new(),
    }
}

fn create_property_schema(
    name: &str,
    property_type: PropertyType,
    required: bool,
    unique: bool,
) -> PropertyValueSchema {
    PropertyValueSchema {
        name: Some(name.to_string()),
        property_type,
        required: Some(required),
        unique: Some(unique),
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
async fn test_required_properties_validation() {
    let storage = setup_test_storage().await;
    let validator = NodeValidator::new(
        storage.clone(),
        "default".to_string(),
        "default".to_string(),
        "main".to_string(),
    );

    // Create NodeType with required property
    create_node_type(
        &storage,
        "test:Article",
        vec![
            create_property_schema("title", PropertyType::String, true, false),
            create_property_schema("body", PropertyType::String, false, false),
        ],
        false,
    )
    .await;

    // Test: Node with required property should pass
    let mut props = HashMap::new();
    props.insert(
        "title".to_string(),
        PropertyValue::String("My Title".to_string()),
    );
    let valid_node = create_test_node("test:Article", props);

    assert!(validator.validate_node("ws1", &valid_node).await.is_ok());

    // Test: Node missing required property should fail
    let props_missing = HashMap::new();
    let invalid_node = create_test_node("test:Article", props_missing);

    let result = validator.validate_node("ws1", &invalid_node).await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Missing required property 'title'"));
}

#[tokio::test]
async fn test_strict_mode_validation() {
    let storage = setup_test_storage().await;
    let validator = NodeValidator::new(
        storage.clone(),
        "default".to_string(),
        "default".to_string(),
        "main".to_string(),
    );

    // Create strict NodeType
    create_node_type(
        &storage,
        "test:Strict",
        vec![create_property_schema(
            "title",
            PropertyType::String,
            false,
            false,
        )],
        true, // strict mode
    )
    .await;

    // Test: Node with only defined properties should pass
    let mut valid_props = HashMap::new();
    valid_props.insert(
        "title".to_string(),
        PropertyValue::String("Valid".to_string()),
    );
    let valid_node = create_test_node("test:Strict", valid_props);

    assert!(validator.validate_node("ws1", &valid_node).await.is_ok());

    // Test: Node with undefined property should fail
    let mut invalid_props = HashMap::new();
    invalid_props.insert(
        "title".to_string(),
        PropertyValue::String("Valid".to_string()),
    );
    invalid_props.insert(
        "undefined_prop".to_string(),
        PropertyValue::String("Invalid".to_string()),
    );
    let invalid_node = create_test_node("test:Strict", invalid_props);

    let result = validator.validate_node("ws1", &invalid_node).await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Undefined property 'undefined_prop'"));
}

#[tokio::test]
async fn test_unique_property_validation() {
    let storage = setup_test_storage().await;
    let validator = NodeValidator::new(
        storage.clone(),
        "default".to_string(),
        "default".to_string(),
        "main".to_string(),
    );

    // Create NodeType with unique property
    create_node_type(
        &storage,
        "test:User",
        vec![
            create_property_schema("email", PropertyType::String, true, true), // unique
            create_property_schema("name", PropertyType::String, false, false),
        ],
        false,
    )
    .await;

    // Create first node
    let mut props1 = HashMap::new();
    props1.insert(
        "email".to_string(),
        PropertyValue::String("user@example.com".to_string()),
    );
    props1.insert(
        "name".to_string(),
        PropertyValue::String("User 1".to_string()),
    );
    let node1 = create_test_node("test:User", props1.clone());

    // Store first node
    storage
        .nodes()
        .create(
            StorageScope::new("default", "default", "main", "ws1"),
            node1.clone(),
            raisin_storage::CreateNodeOptions::default(),
        )
        .await
        .unwrap();

    // Test: First node should validate successfully
    assert!(validator.validate_node("ws1", &node1).await.is_ok());

    // Test: Second node with same email should fail
    let mut props2 = HashMap::new();
    props2.insert(
        "email".to_string(),
        PropertyValue::String("user@example.com".to_string()), // duplicate!
    );
    props2.insert(
        "name".to_string(),
        PropertyValue::String("User 2".to_string()),
    );
    let mut node2 = create_test_node("test:User", props2);
    node2.id = "test-node-2".to_string(); // different ID

    let result = validator.validate_node("ws1", &node2).await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Property 'email' must be unique"));

    // Test: Node with different email should pass
    let mut props3 = HashMap::new();
    props3.insert(
        "email".to_string(),
        PropertyValue::String("another@example.com".to_string()),
    );
    props3.insert(
        "name".to_string(),
        PropertyValue::String("User 3".to_string()),
    );
    let mut node3 = create_test_node("test:User", props3);
    node3.id = "test-node-3".to_string();

    assert!(validator.validate_node("ws1", &node3).await.is_ok());
}

#[tokio::test]
async fn test_archetype_required_field_validation() {
    let storage = setup_test_storage().await;
    let validator = NodeValidator::new(
        storage.clone(),
        "default".to_string(),
        "default".to_string(),
        "main".to_string(),
    );

    create_node_type(
        &storage,
        "test:Page",
        vec![create_property_schema(
            "hero_title",
            PropertyType::String,
            false,
            false,
        )],
        false,
    )
    .await;

    let archetype = Archetype {
        id: "arch-hero".to_string(),
        name: "test:Hero".to_string(),
        extends: None,
        icon: None,
        title: None,
        description: None,
        base_node_type: Some("test:Page".to_string()),
        fields: Some(vec![ElementFieldSchema::TextField {
            base: make_field_base("hero_title", true, false),
            config: None,
        }]),
        initial_content: None,
        layout: None,
        meta: None,
        version: Some(1),
        created_at: None,
        updated_at: None,
        published_at: None,
        published_by: None,
        publishable: Some(true),
        strict: None,
        previous_version: None,
    };

    storage
        .archetypes()
        .upsert(
            BranchScope::new("default", "default", "main"),
            archetype,
            CommitMetadata::system("create hero archetype"),
        )
        .await
        .unwrap();

    let missing_props = HashMap::new();
    let mut node = create_test_node("test:Page", missing_props);
    node.archetype = Some("test:Hero".to_string());

    let result = validator.validate_node("ws1", &node).await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Missing required field 'hero_title'"));

    let mut filled_props = HashMap::new();
    filled_props.insert(
        "hero_title".to_string(),
        PropertyValue::String("Welcome".to_string()),
    );
    let mut valid_node = create_test_node("test:Page", filled_props);
    valid_node.archetype = Some("test:Hero".to_string());

    assert!(validator.validate_node("ws1", &valid_node).await.is_ok());
}

#[tokio::test]
async fn test_element_type_required_field_validation() {
    let storage = setup_test_storage().await;
    let validator = NodeValidator::new(
        storage.clone(),
        "default".to_string(),
        "default".to_string(),
        "main".to_string(),
    );

    create_node_type(
        &storage,
        "test:Page",
        vec![create_property_schema(
            "content",
            PropertyType::Composite,
            false,
            false,
        )],
        false,
    )
    .await;

    let element_type = ElementType {
        id: "elem-hero".to_string(),
        name: "test:Block".to_string(),
        extends: None,
        title: None,
        icon: None,
        description: None,
        fields: vec![ElementFieldSchema::TextField {
            base: make_field_base("headline", true, false),
            config: None,
        }],
        initial_content: None,
        layout: None,
        meta: None,
        version: Some(1),
        created_at: None,
        updated_at: None,
        published_at: None,
        published_by: None,
        publishable: Some(true),
        strict: None,
        previous_version: None,
    };

    storage
        .element_types()
        .upsert(
            BranchScope::new("default", "default", "main"),
            element_type,
            CommitMetadata::system("create block element type"),
        )
        .await
        .unwrap();

    let block = Element {
        uuid: "el-1".to_string(),
        element_type: "test:Block".to_string(),
        content: HashMap::new(),
    };
    let composite_value = PropertyValue::Composite(Composite {
        uuid: "cmp-1".to_string(),
        items: vec![block],
    });

    let mut node_props = HashMap::new();
    node_props.insert("content".to_string(), composite_value);
    let node = create_test_node("test:Page", node_props);

    let result = validator.validate_node("ws1", &node).await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Missing required field 'headline'"));

    let mut block_content = HashMap::new();
    block_content.insert(
        "headline".to_string(),
        PropertyValue::String("Hero Title".to_string()),
    );
    let enriched_block = Element {
        uuid: "el-2".to_string(),
        element_type: "test:Block".to_string(),
        content: block_content,
    };
    let composite_ok = PropertyValue::Composite(Composite {
        uuid: "cmp-2".to_string(),
        items: vec![enriched_block],
    });
    let mut valid_props = HashMap::new();
    valid_props.insert("content".to_string(), composite_ok);
    let valid_node = create_test_node("test:Page", valid_props);

    assert!(validator.validate_node("ws1", &valid_node).await.is_ok());
}

#[tokio::test]
async fn test_validation_with_inheritance() {
    let storage = setup_test_storage().await;
    let validator = NodeValidator::new(
        storage.clone(),
        "default".to_string(),
        "default".to_string(),
        "main".to_string(),
    );

    // Create base NodeType
    let base_type = NodeType {
        id: Some("test:Base".to_string()),
        strict: Some(false),
        name: "test:Base".to_string(),
        extends: None,
        mixins: Vec::new(),
        overrides: None,
        description: None,
        icon: None,
        version: Some(1),
        properties: Some(vec![create_property_schema(
            "id",
            PropertyType::String,
            true,
            true,
        )]),
        allowed_children: Vec::new(),
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
            BranchScope::new("default", "default", "main"),
            base_type,
            CommitMetadata::system("create base node type"),
        )
        .await
        .unwrap();

    // Create child NodeType that extends base
    let child_type = NodeType {
        id: Some("test:Child".to_string()),
        strict: Some(false),
        name: "test:Child".to_string(),
        extends: Some("test:Base".to_string()),
        mixins: Vec::new(),
        overrides: None,
        description: None,
        icon: None,
        version: Some(1),
        properties: Some(vec![create_property_schema(
            "title",
            PropertyType::String,
            true,
            false,
        )]),
        allowed_children: Vec::new(),
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
            BranchScope::new("default", "default", "main"),
            child_type,
            CommitMetadata::system("create child node type"),
        )
        .await
        .unwrap();

    // Test: Node must have both inherited (id) and own (title) required properties
    let mut valid_props = HashMap::new();
    valid_props.insert("id".to_string(), PropertyValue::String("123".to_string()));
    valid_props.insert(
        "title".to_string(),
        PropertyValue::String("Test".to_string()),
    );
    let valid_node = create_test_node("test:Child", valid_props);

    assert!(validator.validate_node("ws1", &valid_node).await.is_ok());

    // Test: Missing inherited required property should fail
    let mut missing_id = HashMap::new();
    missing_id.insert(
        "title".to_string(),
        PropertyValue::String("Test".to_string()),
    );
    let invalid_node = create_test_node("test:Child", missing_id);

    let result = validator.validate_node("ws1", &invalid_node).await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Missing required property 'id'"));
}

#[tokio::test]
async fn test_validate_node_with_nonexistent_nodetype() {
    let storage = setup_test_storage().await;
    let validator = NodeValidator::new(
        storage.clone(),
        "default".to_string(),
        "default".to_string(),
        "main".to_string(),
    );

    // Create a node referencing a non-existent NodeType
    let mut props = HashMap::new();
    props.insert(
        "title".to_string(),
        PropertyValue::String("Test".to_string()),
    );
    let node = create_test_node("test:NonExistent", props);

    // Validation should fail because the NodeType doesn't exist
    let result = validator.validate_node("ws1", &node).await;
    assert!(result.is_err());

    let error = result.unwrap_err();
    let error_msg = error.to_string();

    // Should be a NotFound error
    assert!(
        error_msg.contains("not found")
            || error_msg.contains("NotFound")
            || error_msg.contains("test:NonExistent"),
        "Expected NotFound error, got: {}",
        error_msg
    );
}
