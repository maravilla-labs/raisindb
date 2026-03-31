use super::*;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::translations::{JsonPointer, LocaleCode};
use raisin_storage::BranchRepository;
use raisin_storage_memory::InMemoryStorage;
use std::collections::HashMap;
use std::sync::Arc;

async fn setup_storage_with_branch() -> Arc<InMemoryStorage> {
    let storage = Arc::new(InMemoryStorage::default());
    storage
        .branches()
        .create_branch(
            "tenant1", "repo1", "main", "system", None, None, false, false,
        )
        .await
        .unwrap();
    storage
}

#[tokio::test]
async fn test_update_translation() {
    let storage = setup_storage_with_branch().await;
    let service = TranslationService::new(storage);

    let mut translations = HashMap::new();
    translations.insert(
        JsonPointer::new("/title"),
        PropertyValue::String("Translated Title".to_string()),
    );

    let locale = LocaleCode::parse("fr").unwrap();

    let result = service
        .update_translation(
            "tenant1",
            "repo1",
            "main",
            "workspace1",
            "node-123",
            &locale,
            translations,
            "user1",
            Some("Initial French translation".to_string()),
        )
        .await
        .unwrap();

    assert_eq!(result.node_id, "node-123");
    assert_eq!(result.locale.as_str(), "fr");
    assert!(
        result.revision.timestamp_ms > 0,
        "Revision should be allocated"
    );
}

#[tokio::test]
async fn test_update_translation_empty_fails() {
    let storage = Arc::new(InMemoryStorage::default());
    let service = TranslationService::new(storage);

    let translations = HashMap::new(); // Empty!
    let locale = LocaleCode::parse("fr").unwrap();

    let result = service
        .update_translation(
            "tenant1",
            "repo1",
            "main",
            "workspace1",
            "node-123",
            &locale,
            translations,
            "user1",
            None,
        )
        .await;

    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), Error::Validation(_)));
}

#[tokio::test]
async fn test_hide_node() {
    let storage = setup_storage_with_branch().await;
    let service = TranslationService::new(storage);

    let locale = LocaleCode::parse("de").unwrap();

    let result = service
        .hide_node(
            "tenant1",
            "repo1",
            "main",
            "workspace1",
            "node-123",
            &locale,
            "user1",
            Some("Content not available in German".to_string()),
        )
        .await
        .unwrap();

    assert_eq!(result.node_id, "node-123");
    assert_eq!(result.locale.as_str(), "de");
    assert!(
        result.revision.timestamp_ms > 0,
        "Revision should be allocated"
    );
}
