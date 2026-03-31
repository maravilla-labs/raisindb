use super::*;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::translations::{JsonPointer, LocaleCode};
use raisin_storage_memory::NoopTranslationRepo;
use std::collections::HashMap;
use std::sync::Arc;

type MockTranslationRepository = NoopTranslationRepo;

#[tokio::test]
async fn test_update_block_translation() {
    let repo = Arc::new(MockTranslationRepository::default());
    let service = BlockTranslationService::new(repo);

    let mut translations = HashMap::new();
    translations.insert(
        JsonPointer::new("/text"),
        PropertyValue::String("Texte traduit".to_string()),
    );

    let locale = LocaleCode::parse("fr").unwrap();

    let result = service
        .update_block_translation(
            "tenant1",
            "repo1",
            "main",
            "workspace1",
            "node-123",
            "block-uuid-1",
            &locale,
            translations,
            "user1",
            Some("Translate block text".to_string()),
            raisin_hlc::HLC::new(1, 0),
        )
        .await
        .unwrap();

    assert_eq!(result.block_uuid, "block-uuid-1");
    assert_eq!(result.node_id, "node-123");
    assert_eq!(result.locale.as_str(), "fr");
    assert_eq!(result.revision, raisin_hlc::HLC::new(1, 0));
}

#[tokio::test]
async fn test_update_block_translation_empty_fails() {
    let repo = Arc::new(MockTranslationRepository::default());
    let service = BlockTranslationService::new(repo);

    let translations = HashMap::new(); // Empty!
    let locale = LocaleCode::parse("fr").unwrap();

    let result = service
        .update_block_translation(
            "tenant1",
            "repo1",
            "main",
            "workspace1",
            "node-123",
            "block-uuid-1",
            &locale,
            translations,
            "user1",
            None,
            raisin_hlc::HLC::new(1, 0),
        )
        .await;

    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), Error::Validation(_)));
}

#[test]
fn test_extract_block_uuids() {
    let repo = Arc::new(MockTranslationRepository::default());
    let service = BlockTranslationService::new(repo);

    // Create a node with Composite
    let mut properties = HashMap::new();
    let mut block1 = HashMap::new();
    block1.insert(
        "uuid".to_string(),
        PropertyValue::String("block-uuid-1".to_string()),
    );
    block1.insert(
        "type".to_string(),
        PropertyValue::String("text".to_string()),
    );

    let mut block2 = HashMap::new();
    block2.insert(
        "uuid".to_string(),
        PropertyValue::String("block-uuid-2".to_string()),
    );
    block2.insert(
        "type".to_string(),
        PropertyValue::String("image".to_string()),
    );

    let blocks = vec![PropertyValue::Object(block1), PropertyValue::Object(block2)];
    properties.insert("content".to_string(), PropertyValue::Array(blocks));

    let node = Node {
        id: "node-123".to_string(),
        name: "Test Node".to_string(),
        path: "/test".to_string(),
        node_type: "raisin:page".to_string(),
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
        workspace: None,
        owner_id: None,
        relations: Vec::new(),
    };

    let uuids = service.extract_block_uuids(&node);

    assert_eq!(uuids.len(), 2);
    assert!(uuids.contains("block-uuid-1"));
    assert!(uuids.contains("block-uuid-2"));
}

#[tokio::test]
async fn test_mark_blocks_orphaned() {
    let repo = Arc::new(MockTranslationRepository::default());
    let service = BlockTranslationService::new(repo);

    let result = service
        .mark_blocks_orphaned(
            "tenant1",
            "repo1",
            "main",
            "workspace1",
            "node-123",
            vec!["block-uuid-1".to_string(), "block-uuid-2".to_string()],
            &raisin_hlc::HLC::new(5, 0),
        )
        .await;

    assert!(result.is_ok());
}
