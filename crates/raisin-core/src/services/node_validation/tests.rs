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
        index: None,
        meta: None,
        encrypted: None,
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
        spatial: None,
        encrypted: None,
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

/// Regression test: two nodes that already share a unique property value
/// (a pre-existing authoring collision, stored directly without going
/// through the validator) must not block an update to either node that
/// leaves that property's value unchanged. Only a genuine change to the
/// unique value should be re-checked against other nodes.
#[tokio::test]
async fn test_unique_property_unchanged_on_update_is_not_reverified() {
    let storage = setup_test_storage().await;
    let validator = NodeValidator::new(
        storage.clone(),
        "default".to_string(),
        "default".to_string(),
        "main".to_string(),
    );

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

    // Store two nodes that already share the same unique "email" value,
    // bypassing the validator -- this mirrors a pre-existing authoring
    // collision already live in storage before this check ever ran.
    let mut props1 = HashMap::new();
    props1.insert(
        "email".to_string(),
        PropertyValue::String("shared@example.com".to_string()),
    );
    props1.insert(
        "name".to_string(),
        PropertyValue::String("User 1".to_string()),
    );
    let node1 = create_test_node("test:User", props1);
    storage
        .nodes()
        .create(
            StorageScope::new("default", "default", "main", "ws1"),
            node1,
            raisin_storage::CreateNodeOptions::default(),
        )
        .await
        .unwrap();

    let mut props2 = HashMap::new();
    props2.insert(
        "email".to_string(),
        PropertyValue::String("shared@example.com".to_string()), // same as node1
    );
    props2.insert(
        "name".to_string(),
        PropertyValue::String("User 2".to_string()),
    );
    let mut node2 = create_test_node("test:User", props2);
    node2.id = "test-node-2".to_string();
    storage
        .nodes()
        .create(
            StorageScope::new("default", "default", "main", "ws1"),
            node2.clone(),
            raisin_storage::CreateNodeOptions::default(),
        )
        .await
        .unwrap();

    // Update node2 with an unrelated field change; "email" stays the same
    // value it already has in storage. This must succeed even though
    // node1 still has the identical "email" value.
    node2.properties.insert(
        "name".to_string(),
        PropertyValue::String("User 2 renamed".to_string()),
    );
    assert!(validator.validate_node("ws1", &node2).await.is_ok());

    // But updating an EXISTING node's unique value to something new that
    // collides with a different node must still fail -- store node2's
    // unrelated "name" change first (an update with email unchanged, as
    // above), then attempt to change node2's own email to a THIRD value
    // that collides with a freshly-created node4.
    storage
        .nodes()
        .update(
            StorageScope::new("default", "default", "main", "ws1"),
            node2.clone(),
            raisin_storage::UpdateNodeOptions::default(),
        )
        .await
        .unwrap();

    let mut props4 = HashMap::new();
    props4.insert(
        "email".to_string(),
        PropertyValue::String("distinct@example.com".to_string()),
    );
    props4.insert(
        "name".to_string(),
        PropertyValue::String("User 4".to_string()),
    );
    let mut node4 = create_test_node("test:User", props4);
    node4.id = "test-node-4".to_string();
    storage
        .nodes()
        .create(
            StorageScope::new("default", "default", "main", "ws1"),
            node4,
            raisin_storage::CreateNodeOptions::default(),
        )
        .await
        .unwrap();

    node2.properties.insert(
        "email".to_string(),
        PropertyValue::String("distinct@example.com".to_string()), // now collides with node4
    );
    let result = validator.validate_node("ws1", &node2).await;
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Property 'email' must be unique"));
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

/// The virtual-mount materializer stamps `__pushed_state` on every node of a
/// mount that declares mutable fields, and `check_strict_mode` exempts only keys
/// starting with `$` — a `__` prefix buys nothing. `raisin:Event` is
/// `strict: true`, so before the `raisin:VirtualNode` mixin existed the first
/// writable calendar mount had EVERY synced write rejected.
///
/// This asserts both halves: the mixin makes the write validate, and stripping
/// it puts the rejection straight back.
#[tokio::test]
async fn strict_event_accepts_the_stamped_reserved_properties_via_the_mixin() {
    let storage = setup_test_storage().await;
    crate::nodetype_init::init_repository_nodetypes(storage.clone(), "default", "default", "main")
        .await
        .unwrap();

    let validator = NodeValidator::new(
        storage.clone(),
        "default".to_string(),
        "default".to_string(),
        "main".to_string(),
    );

    let mut props = HashMap::new();
    props.insert(
        "title".to_string(),
        PropertyValue::String("Standup".to_string()),
    );
    // Everything the materializer stamps, including the one the type omitted.
    props.insert("__virtual".to_string(), PropertyValue::Boolean(true));
    props.insert(
        "__mount_id".to_string(),
        PropertyValue::String("mount-1".to_string()),
    );
    props.insert(
        "__external_id".to_string(),
        PropertyValue::String("evt-1".to_string()),
    );
    props.insert(
        "__etag".to_string(),
        PropertyValue::String("W/\"1\"".to_string()),
    );
    props.insert(
        "__synced_at".to_string(),
        PropertyValue::String("2026-08-05T09:00:00Z".to_string()),
    );
    props.insert(
        "__pushed_state".to_string(),
        PropertyValue::Object(HashMap::from([(
            "my_response".to_string(),
            PropertyValue::String("accepted".to_string()),
        )])),
    );
    let node = create_test_node("raisin:Event", props);

    validator
        .validate_node("ws1", &node)
        .await
        .expect("a strict raisin:Event must accept the reserved properties the sync stamps");

    // Now take the mixin away and prove it was what made the difference.
    let scope = BranchScope::new("default", "default", "main");
    let mut event_type = storage
        .node_types()
        .get(scope.clone(), "raisin:Event", None)
        .await
        .unwrap()
        .expect("raisin:Event must be installed");
    assert_eq!(
        event_type.mixins,
        vec!["raisin:VirtualNode".to_string()],
        "the reserved properties must come from the mixin, not be re-declared inline"
    );
    event_type.mixins.clear();
    storage
        .node_types()
        .put(scope, event_type, CommitMetadata::system("strip the mixin"))
        .await
        .unwrap();

    // Narrowed to the one property the pre-mixin type omitted, so the rejection
    // can only be about it (strict mode reports the first offender it finds).
    let mut narrow = HashMap::new();
    narrow.insert(
        "title".to_string(),
        PropertyValue::String("Standup".to_string()),
    );
    narrow.insert(
        "__pushed_state".to_string(),
        PropertyValue::Object(HashMap::new()),
    );
    let err = validator
        .validate_node("ws1", &create_test_node("raisin:Event", narrow))
        .await
        .expect_err("without the mixin the stamped reserved properties are undefined properties");
    assert!(
        err.to_string().contains("__pushed_state"),
        "expected the strict-mode rejection to name the stamped property, got: {err}"
    );
}

/// A STRICT archetype must not reject the engine's own reserved properties.
///
/// The write path stamps `$mixins` / `$supertypes` onto every node, and the
/// transaction layer then re-validates the stamped node. Archetype strict mode
/// checked every property against the archetype's declared fields with no
/// exemption for `$` keys — which `check_strict_mode` (NodeType strict mode)
/// has always had — so every REST write to a strict archetype failed with
/// "Undefined property '$mixins'". The node was unwritable through its own
/// server.
#[tokio::test]
async fn strict_archetype_accepts_the_engine_stamped_reserved_properties() {
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
        id: "arch-strict".to_string(),
        name: "test:StrictHero".to_string(),
        extends: None,
        icon: None,
        title: None,
        description: None,
        base_node_type: Some("test:Page".to_string()),
        fields: Some(vec![ElementFieldSchema::TextField {
            base: make_field_base("hero_title", false, false),
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
        strict: Some(true),
        previous_version: None,
    };

    storage
        .archetypes()
        .upsert(
            BranchScope::new("default", "default", "main"),
            archetype,
            CommitMetadata::system("create strict hero archetype"),
        )
        .await
        .unwrap();

    let mut props = HashMap::new();
    props.insert(
        "hero_title".to_string(),
        PropertyValue::String("Welcome".to_string()),
    );
    let mut node = create_test_node("test:Page", props);
    node.archetype = Some("test:StrictHero".to_string());
    // Exactly what `validate_and_stamp` leaves on the node.
    node.set_effective_types(
        vec!["test:SomeMixin".to_string()],
        vec!["test:Page".to_string()],
    );

    validator
        .validate_node("ws1", &node)
        .await
        .expect("a strict archetype must accept the engine's own $ properties");

    // An UNDECLARED ordinary property is still rejected — the exemption is for
    // reserved keys only, not a hole in strict mode.
    node.properties.insert(
        "not_declared".to_string(),
        PropertyValue::String("x".to_string()),
    );
    let err = validator.validate_node("ws1", &node).await.unwrap_err();
    assert!(
        err.to_string().contains("not_declared"),
        "strict archetype must still reject an undeclared property, got: {err}"
    );
}

/// `validate_and_stamp` is the shared write-path entry point: it strips
/// client-supplied reserved keys, validates, and materializes the membership
/// sets. Before it lived on the validator, the transaction layer only
/// validated, so a node created by a child POST or by SQL DML carried no
/// `$mixins` / `$supertypes` and `has_mixin()` / `is_a()` answered false.
#[tokio::test]
async fn validate_and_stamp_materializes_membership_and_ignores_client_supplied_sets() {
    let storage = setup_test_storage().await;
    let validator = NodeValidator::new(
        storage.clone(),
        "default".to_string(),
        "default".to_string(),
        "main".to_string(),
    );

    create_node_type(&storage, "test:Page", vec![], false).await;

    let mut node = create_test_node("test:Page", HashMap::new());
    // A client trying to grant itself a mixin it does not have.
    node.properties.insert(
        "$mixins".to_string(),
        PropertyValue::Array(vec![PropertyValue::String("test:Forged".to_string())]),
    );

    validator
        .validate_and_stamp("ws1", &mut node)
        .await
        .unwrap();

    assert!(
        !node.has_mixin("test:Forged"),
        "a client-supplied membership set must be stripped, not trusted"
    );
    assert!(
        node.is_a("test:Page"),
        "the node's own type must be in the materialized supertype set"
    );
    assert!(
        node.properties.contains_key("$supertypes"),
        "$supertypes must be stamped"
    );
}

/// The admin console read an integration node, PUT its properties back
/// unchanged, and the save was refused:
///
///     Property 'capabilities_checked_at' on NodeType 'raisin:Integration'
///     is declared String but the value is Date
///
/// for a value the server itself had just written. `PropertyValue` is
/// `#[serde(untagged)]` with `Date` ahead of `String`, so a timestamp that goes
/// out as JSON and comes back always returns as a `Date` — which made
/// `type: String` unsatisfiable for any client, while server code building
/// `PropertyValue::String(rfc3339)` in memory never met that deserializer.
///
/// The declarations are `Date` now, and this pins the round trip: a timestamp
/// submitted in EITHER spelling validates, and both end up as the same `Date`.
#[tokio::test]
async fn a_timestamp_validates_in_either_spelling_on_a_date_property() {
    let storage = setup_test_storage().await;
    let validator = NodeValidator::new(
        storage.clone(),
        "default".to_string(),
        "default".to_string(),
        "main".to_string(),
    );

    create_node_type(
        &storage,
        "test:Integration",
        vec![create_property_schema(
            "capabilities_checked_at",
            PropertyType::Date,
            false,
            false,
        )],
        true,
    )
    .await;

    let stamp = "2026-09-10T23:43:51+00:00";

    // The wire shape: JSON round-tripping makes it a Date before it ever
    // reaches validation. This is what the console sends.
    let from_json: PropertyValue =
        serde_json::from_value(serde_json::json!(stamp)).expect("RFC3339 string parses");
    assert!(
        matches!(from_json, PropertyValue::Date(_)),
        "untagged deserialization must yield Date, not String — if this fails the \
         variant order in PropertyValue changed and the coercion below is moot"
    );

    let mut props = HashMap::new();
    props.insert("capabilities_checked_at".to_string(), from_json);
    let mut node = create_test_node("test:Integration", props);
    validator
        .validate_and_stamp("ws1", &mut node)
        .await
        .expect("a Date on a Date-declared property must validate");

    // The in-memory shape: server code that builds the string itself, and
    // values already at rest written before the declaration was corrected.
    // These coerce rather than being refused, so no migration is needed.
    let mut props = HashMap::new();
    props.insert(
        "capabilities_checked_at".to_string(),
        PropertyValue::String(stamp.to_string()),
    );
    let mut node = create_test_node("test:Integration", props);
    validator
        .validate_and_stamp("ws1", &mut node)
        .await
        .expect("an RFC3339 string must coerce, not be refused");
    assert!(
        matches!(
            node.properties.get("capabilities_checked_at"),
            Some(PropertyValue::Date(_))
        ),
        "the string spelling must be coerced to Date, so both doors agree"
    );

    // A string that is not a timestamp is still refused, by name — the same
    // posture as the decimal coercion, rather than storing a pseudo-date.
    let mut props = HashMap::new();
    props.insert(
        "capabilities_checked_at".to_string(),
        PropertyValue::String("last tuesday".to_string()),
    );
    let mut node = create_test_node("test:Integration", props);
    let err = validator
        .validate_and_stamp("ws1", &mut node)
        .await
        .expect_err("a non-timestamp string must not be coerced");
    assert!(
        err.to_string().contains("capabilities_checked_at"),
        "the error must name the property, got: {err}"
    );
}

/// The MIRROR of the test above: a `Date` value on a property whose declaration
/// still says `String`.
///
/// This is the direction that actually broke production, and it is the one the
/// original coercion did not cover. The binary's declarations were corrected to
/// `Date`, but a NodeType lives in storage PER TENANT AND REPO and only adopts a
/// corrected declaration when it resyncs. In the window before that — or
/// indefinitely, behind a stale definitions overlay — the server writes `Date`
/// into a schema that still declares `String`, and the whole node write is
/// refused.
///
/// The blast radius was nowhere near the property involved. On one tenant's
/// connector it discarded completed OAuth grants (the user consented, the
/// provider issued tokens, the node could not be saved) and failed every token
/// refresh for hours — each attempt having already rotated the refresh token at
/// the provider. A bookkeeping timestamp must never be able to veto a
/// credential write.
#[tokio::test]
async fn a_date_value_coerces_on_a_property_still_declared_string() {
    let storage = setup_test_storage().await;
    let validator = NodeValidator::new(
        storage.clone(),
        "default".to_string(),
        "default".to_string(),
        "main".to_string(),
    );

    // The UNMIGRATED declaration, exactly as a repo that has not resynced still
    // holds it.
    create_node_type(
        &storage,
        "test:StaleIntegration",
        vec![create_property_schema(
            "capabilities_checked_at",
            PropertyType::String,
            false,
            false,
        )],
        true,
    )
    .await;

    let mut props = HashMap::new();
    props.insert(
        "capabilities_checked_at".to_string(),
        PropertyValue::Date(raisin_models::timestamp::StorageTimestamp::now()),
    );
    let mut node = create_test_node("test:StaleIntegration", props);

    validator
        .validate_and_stamp("ws1", &mut node)
        .await
        .expect("a Date against a String declaration must coerce, not refuse the write");

    match node.properties.get("capabilities_checked_at") {
        Some(PropertyValue::String(rendered)) => {
            assert!(
                chrono::DateTime::parse_from_rfc3339(rendered).is_ok(),
                "the coerced value must be a real RFC3339 timestamp, got: {rendered}"
            );
        }
        other => panic!("expected a coerced String, got {other:?}"),
    }
}

/// Every built-in `_at` property must be declared `Date`.
///
/// Declaring one `String` is unsatisfiable over the wire (see the test above),
/// and it stayed invisible for as long as `type:` was documentation. Seven of
/// them had drifted across five NodeTypes before type enforcement surfaced the
/// first one. This is the cheap guard that keeps the next one from shipping.
#[test]
fn no_builtin_declares_a_timestamp_property_as_a_string() {
    // Two independent detectors, because neither alone is enough. `raisin:Event`
    // spelled its instants `_utc` and `raisin:Mail` called one simply `date`, so
    // an `_at`-only sweep declared itself clean while both were unsatisfiable;
    // and a description-only sweep misses any property nobody documented.
    //
    // `_local` is the deliberate exception: wall-clock with no offset is NOT
    // RFC3339, loses the untagged race to `String`, and so really is one.
    fn looks_like_a_timestamp(name: &str, description: &str) -> bool {
        if name.ends_with("_local") {
            return false;
        }
        let by_name = name.ends_with("_at")
            || name.ends_with("_utc")
            || name.ends_with("_time")
            || name == "date";
        let lower = description.to_ascii_lowercase();
        let by_description = ["rfc 3339", "rfc3339", "iso 8601", "iso8601", "utc instant"]
            .iter()
            .any(|needle| lower.contains(needle));
        by_name || by_description
    }

    let offenders: Vec<String> = crate::nodetype_init::load_global_nodetypes()
        .iter()
        .flat_map(|nt| {
            let type_name = nt.name.clone();
            nt.properties.iter().flatten().filter_map(move |p| {
                let name = p.name.as_deref()?;
                if p.property_type != PropertyType::String {
                    return None;
                }
                let description = p
                    .meta
                    .as_ref()
                    .and_then(|m| m.get("description"))
                    .and_then(|v| match v {
                        PropertyValue::String(s) => Some(s.as_str()),
                        _ => None,
                    })
                    .unwrap_or_default();
                looks_like_a_timestamp(name, description).then(|| format!("{type_name}.{name}"))
            })
        })
        .collect();

    assert!(
        offenders.is_empty(),
        "these timestamp properties are declared String and can never be written \
         as one over the wire — declare them Date: {offenders:?}"
    );
}

// ---------------------------------------------------------------------------
// A String-declared property that holds a number
//
// Postal codes, jersey numbers, article numbers and phone numbers are strings
// whose spelling is the point. They still reach the validator as numbers — read
// back from storage as a `Decimal` (a decimal and a string are the same bytes on
// the wire), or sent as a JSON number by a caller. Refusing them with
//
//     Property 'postal_code' on NodeType 'party:Address'
//     is declared String but the value is Decimal
//
// was the wrong answer: rendering a number as its own string loses nothing and
// is exactly what the declaration asked for.
// ---------------------------------------------------------------------------

fn resolved_with(
    properties: Vec<PropertyValueSchema>,
) -> crate::services::node_type_resolver::ResolvedNodeType {
    crate::services::node_type_resolver::ResolvedNodeType {
        node_type: raisin_models::nodes::types::NodeType {
            id: Some("party:Address".to_string()),
            name: "party:Address".to_string(),
            strict: None,
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
            immutable: None,
            publishable: Some(true),
            auditable: Some(false),
            indexable: Some(true),
            index_types: None,
            created_at: None,
            updated_at: None,
            published_at: None,
            published_by: None,
            previous_version: None,
            compound_indexes: None,
            is_mixin: None,
        },
        resolved_properties: properties,
        resolved_allowed_children: vec![],
        resolved_mixins: vec![],
        resolved_indexable: true,
        resolved_index_types: vec![],
        resolved_compound_indexes: vec![],
        inheritance_chain: vec!["party:Address".to_string()],
    }
}

/// Coerce, then type-check — the order `NodeValidator` itself uses.
fn coerce_and_check(
    properties: HashMap<String, PropertyValue>,
    schema: Vec<PropertyValueSchema>,
) -> Result<raisin_models::nodes::Node, String> {
    let mut node = create_test_node("party:Address", properties);
    let resolved = resolved_with(schema);
    super::property_checks::coerce_declared_decimals(&mut node, &resolved)
        .map_err(|e| e.to_string())?;
    super::property_checks::coerce_declared_dates(&mut node, &resolved)
        .map_err(|e| e.to_string())?;
    super::property_checks::check_property_types(&node, &resolved).map_err(|e| e.to_string())?;
    Ok(node)
}

#[test]
fn a_numeric_postal_code_is_accepted_on_a_string_property() {
    let schema = vec![create_property_schema(
        "postal_code",
        PropertyType::String,
        false,
        false,
    )];

    // As it comes back from storage, and as a caller may send it.
    for value in [
        PropertyValue::Decimal("76133".parse().unwrap()),
        PropertyValue::Integer(76133),
        PropertyValue::String("76133".to_string()),
    ] {
        let mut properties = HashMap::new();
        properties.insert("postal_code".to_string(), value.clone());
        let node = coerce_and_check(properties, schema.clone())
            .unwrap_or_else(|e| panic!("{value:?} was refused: {e}"));
        assert_eq!(
            node.properties.get("postal_code"),
            Some(&PropertyValue::String("76133".to_string())),
            "{value:?} did not land as the string 76133"
        );
    }
}

#[test]
fn a_jersey_number_keeps_its_spelling() {
    let schema = vec![create_property_schema(
        "jersey_number",
        PropertyType::String,
        false,
        false,
    )];
    let mut properties = HashMap::new();
    // "05" never becomes a Decimal any more (the variant refuses a spelling it
    // would change), so it arrives here still a string and must stay one.
    properties.insert(
        "jersey_number".to_string(),
        PropertyValue::String("05".to_string()),
    );
    let node = coerce_and_check(properties, schema).expect("accepted");
    assert_eq!(
        node.properties.get("jersey_number"),
        Some(&PropertyValue::String("05".to_string())),
        "the leading zero was lost"
    );
}

#[test]
fn a_decimal_keeps_its_scale_when_rendered_onto_a_string_property() {
    let schema = vec![create_property_schema(
        "price_label",
        PropertyType::String,
        false,
        false,
    )];
    let mut properties = HashMap::new();
    properties.insert(
        "price_label".to_string(),
        PropertyValue::Decimal("19.90".parse().unwrap()),
    );
    let node = coerce_and_check(properties, schema).expect("accepted");
    assert_eq!(
        node.properties.get("price_label"),
        Some(&PropertyValue::String("19.90".to_string())),
        "the trailing zero of the scale was dropped"
    );
}

#[test]
fn a_declared_decimal_is_untouched_by_the_string_coercion() {
    let schema = vec![create_property_schema(
        "amount",
        PropertyType::Decimal,
        false,
        false,
    )];
    let mut properties = HashMap::new();
    properties.insert(
        "amount".to_string(),
        PropertyValue::String("19.90".to_string()),
    );
    let node = coerce_and_check(properties, schema).expect("accepted");
    assert_eq!(
        node.properties.get("amount"),
        Some(&PropertyValue::Decimal("19.90".parse().unwrap())),
        "a declared Decimal must still become a Decimal"
    );
}

#[test]
fn numbers_on_number_properties_are_left_alone() {
    let schema = vec![
        create_property_schema("count", PropertyType::Integer, false, false),
        create_property_schema("ratio", PropertyType::Float, false, false),
    ];
    let mut properties = HashMap::new();
    properties.insert("count".to_string(), PropertyValue::Integer(5));
    properties.insert("ratio".to_string(), PropertyValue::Float(2.5));
    let node = coerce_and_check(properties, schema).expect("accepted");
    assert_eq!(
        node.properties.get("count"),
        Some(&PropertyValue::Integer(5))
    );
    assert_eq!(
        node.properties.get("ratio"),
        Some(&PropertyValue::Float(2.5))
    );
}

#[test]
fn a_declared_decimal_still_refuses_a_json_number() {
    // The existing guarantee this change must not weaken: a JSON number has
    // already been through an f64, so it is refused rather than rounded.
    let schema = vec![create_property_schema(
        "amount",
        PropertyType::Decimal,
        false,
        false,
    )];
    let mut properties = HashMap::new();
    properties.insert("amount".to_string(), PropertyValue::Float(19.90));
    let err = coerce_and_check(properties, schema).expect_err("must be refused");
    assert!(err.contains("must be sent as a STRING"), "got: {err}");
}

#[test]
fn a_decimal_is_only_stored_where_a_declaration_says_decimal() {
    // The invariant that stops a guess being recorded as a fact. The legacy
    // read rule has to classify a package-authored `rate: '10'` as a Decimal,
    // but its output is only trusted where a declaration confirms it.
    let schema = vec![
        create_property_schema("rate", PropertyType::Decimal, false, false),
        create_property_schema("postal_code", PropertyType::String, false, false),
    ];

    let mut properties = HashMap::new();
    // Declared Decimal: stays a Decimal (this is the package-install path).
    properties.insert("rate".to_string(), PropertyValue::String("10".to_string()));
    // Declared String: rendered back.
    properties.insert(
        "postal_code".to_string(),
        PropertyValue::Decimal("76133".parse().unwrap()),
    );
    // UNDECLARED, holding a Decimal the heuristic produced: rendered back to the
    // string it was spelled as, rather than cemented as a decimal.
    properties.insert(
        "legacy_ref".to_string(),
        PropertyValue::Decimal("00123".parse().unwrap_or_else(|_| "123".parse().unwrap())),
    );

    let node = coerce_and_check(properties, schema).expect("accepted");
    assert_eq!(
        node.properties.get("rate"),
        Some(&PropertyValue::Decimal("10".parse().unwrap())),
        "a package-authored decimal must still classify against its declaration"
    );
    assert_eq!(
        node.properties.get("postal_code"),
        Some(&PropertyValue::String("76133".to_string()))
    );
    assert_eq!(
        node.properties.get("legacy_ref"),
        Some(&PropertyValue::String("123".to_string())),
        "an undeclared Decimal must not be recorded as a fact"
    );
}

#[test]
fn a_package_authored_decimal_string_still_becomes_a_decimal() {
    // Regression guard for the install path: packages write `rate: '10'` as a
    // plain quoted YAML string, so bare-str -> losslessness -> Decimal is
    // load-bearing and permanent, not a transition measure.
    let schema = vec![create_property_schema(
        "rate",
        PropertyType::Decimal,
        false,
        false,
    )];
    for raw in ["0", "2.6", "3.8", "7", "8.1", "10", "19", "20"] {
        let mut properties = HashMap::new();
        properties.insert("rate".to_string(), PropertyValue::String(raw.to_string()));
        let node = coerce_and_check(properties, schema.clone())
            .unwrap_or_else(|e| panic!("package decimal {raw:?} refused: {e}"));
        assert_eq!(
            node.properties.get("rate"),
            Some(&PropertyValue::Decimal(raw.parse().unwrap())),
            "{raw:?} must classify as a Decimal against its declaration"
        );
    }
}
