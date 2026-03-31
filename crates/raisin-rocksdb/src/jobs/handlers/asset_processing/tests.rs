//! Tests for asset processing helpers and types

use super::helpers::{extract_mime_type, extract_storage_key, is_image_mime};
use super::types::AssetProcessingResult;
use chrono::Utc;
use raisin_models::nodes::properties::value::Resource;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use std::collections::HashMap;

/// Helper to create a test Resource with required fields
fn test_resource(metadata: Option<HashMap<String, PropertyValue>>) -> Resource {
    Resource {
        uuid: "test-uuid".to_string(),
        name: None,
        size: None,
        mime_type: None,
        url: None,
        metadata,
        is_loaded: None,
        is_external: None,
        created_at: Utc::now().into(),
        updated_at: Utc::now().into(),
    }
}

#[test]
fn test_is_image_mime() {
    assert!(is_image_mime(&Some("image/jpeg".to_string())));
    assert!(is_image_mime(&Some("image/png".to_string())));
    assert!(is_image_mime(&Some("image/webp".to_string())));
    assert!(is_image_mime(&Some("image/gif".to_string())));
    assert!(is_image_mime(&Some("image/svg+xml".to_string())));
    assert!(!is_image_mime(&Some("application/pdf".to_string())));
    assert!(!is_image_mime(&Some("text/plain".to_string())));
    assert!(!is_image_mime(&Some("video/mp4".to_string())));
    assert!(!is_image_mime(&None));
}

#[test]
fn test_extract_mime_type_from_resource() {
    let mut node = Node::default();
    let mut metadata = HashMap::new();
    metadata.insert(
        "mime_type".to_string(),
        PropertyValue::String("image/png".to_string()),
    );
    node.properties.insert(
        "file".to_string(),
        PropertyValue::Resource(test_resource(Some(metadata))),
    );

    assert_eq!(extract_mime_type(&node), Some("image/png".to_string()));
}

#[test]
fn test_extract_mime_type_from_object() {
    let mut node = Node::default();
    let mut file_obj = HashMap::new();
    file_obj.insert(
        "mime_type".to_string(),
        PropertyValue::String("application/pdf".to_string()),
    );
    node.properties
        .insert("file".to_string(), PropertyValue::Object(file_obj));

    assert_eq!(
        extract_mime_type(&node),
        Some("application/pdf".to_string())
    );
}

#[test]
fn test_extract_mime_type_from_content_type() {
    let mut node = Node::default();
    node.properties.insert(
        "contentType".to_string(),
        PropertyValue::String("text/html".to_string()),
    );

    assert_eq!(extract_mime_type(&node), Some("text/html".to_string()));
}

#[test]
fn test_extract_storage_key_from_resource() {
    let mut node = Node::default();
    node.id = "test-node".to_string();
    let mut metadata = HashMap::new();
    metadata.insert(
        "storage_key".to_string(),
        PropertyValue::String("uploads/abc123.png".to_string()),
    );
    node.properties.insert(
        "file".to_string(),
        PropertyValue::Resource(test_resource(Some(metadata))),
    );

    assert_eq!(
        extract_storage_key(&node).unwrap(),
        "uploads/abc123.png".to_string()
    );
}

#[test]
fn test_extract_storage_key_from_object() {
    let mut node = Node::default();
    node.id = "test-node".to_string();
    let mut file_obj = HashMap::new();
    file_obj.insert(
        "storageKey".to_string(),
        PropertyValue::String("uploads/def456.pdf".to_string()),
    );
    node.properties
        .insert("file".to_string(), PropertyValue::Object(file_obj));

    assert_eq!(
        extract_storage_key(&node).unwrap(),
        "uploads/def456.pdf".to_string()
    );
}

#[test]
fn test_extract_storage_key_not_found() {
    let mut node = Node::default();
    node.id = "test-node".to_string();

    let result = extract_storage_key(&node);
    assert!(result.is_err());
}

#[test]
fn test_asset_processing_result_default() {
    let result = AssetProcessingResult::default();
    assert!(result.node_id.is_empty());
    assert!(result.extracted_text.is_none());
    assert!(result.pdf_page_count.is_none());
    assert!(!result.used_ocr);
    assert!(result.caption.is_none());
    assert!(result.alt_text.is_none());
    assert!(result.keywords.is_none());
    assert!(!result.image_embedding_generated);
    assert!(result.image_embedding_dim.is_none());
    assert!(result.image_embedding.is_none());
}

#[test]
fn test_asset_processing_result_serialization() {
    let result = AssetProcessingResult {
        node_id: "node-123".to_string(),
        extracted_text: Some("Sample text".to_string()),
        pdf_page_count: Some(5),
        used_ocr: true,
        caption: Some("A beautiful landscape".to_string()),
        alt_text: Some("Beautiful landscape".to_string()),
        keywords: Some(vec!["nature".to_string(), "landscape".to_string()]),
        image_embedding_generated: true,
        image_embedding_dim: Some(512),
        image_embedding: Some(vec![0.1, 0.2, 0.3]),
    };

    let json = serde_json::to_string(&result).unwrap();
    let deserialized: AssetProcessingResult = serde_json::from_str(&json).unwrap();

    assert_eq!(result.node_id, deserialized.node_id);
    assert_eq!(result.extracted_text, deserialized.extracted_text);
    assert_eq!(result.caption, deserialized.caption);
    assert_eq!(result.image_embedding_dim, deserialized.image_embedding_dim);
}

#[test]
fn test_extract_mime_type_camel_case_variant() {
    let mut node = Node::default();
    let mut metadata = HashMap::new();
    metadata.insert(
        "mimeType".to_string(),
        PropertyValue::String("image/webp".to_string()),
    );
    node.properties.insert(
        "file".to_string(),
        PropertyValue::Resource(test_resource(Some(metadata))),
    );

    assert_eq!(extract_mime_type(&node), Some("image/webp".to_string()));
}

#[test]
fn test_extract_mime_type_from_mime_type_property() {
    let mut node = Node::default();
    node.properties.insert(
        "mimeType".to_string(),
        PropertyValue::String("video/mp4".to_string()),
    );

    assert_eq!(extract_mime_type(&node), Some("video/mp4".to_string()));
}

#[test]
fn test_extract_mime_type_empty_node() {
    let node = Node::default();
    assert_eq!(extract_mime_type(&node), None);
}

#[test]
fn test_extract_storage_key_nested_metadata() {
    let mut node = Node::default();
    node.id = "test-node".to_string();

    let mut inner_metadata = HashMap::new();
    inner_metadata.insert(
        "storage_key".to_string(),
        PropertyValue::String("nested/key/abc.jpg".to_string()),
    );

    let mut file_obj = HashMap::new();
    file_obj.insert(
        "metadata".to_string(),
        PropertyValue::Object(inner_metadata),
    );

    node.properties
        .insert("file".to_string(), PropertyValue::Object(file_obj));

    assert_eq!(
        extract_storage_key(&node).unwrap(),
        "nested/key/abc.jpg".to_string()
    );
}

#[test]
fn test_extract_storage_key_from_resource_property() {
    let mut node = Node::default();
    node.id = "test-node".to_string();

    let mut metadata = HashMap::new();
    metadata.insert(
        "storage_key".to_string(),
        PropertyValue::String("package/assets/logo.png".to_string()),
    );

    node.properties.insert(
        "resource".to_string(),
        PropertyValue::Resource(test_resource(Some(metadata))),
    );

    assert_eq!(
        extract_storage_key(&node).unwrap(),
        "package/assets/logo.png".to_string()
    );
}

#[test]
fn test_is_image_mime_edge_cases() {
    assert!(!is_image_mime(&Some("".to_string())));
    assert!(!is_image_mime(&Some("IMAGE/JPEG".to_string())));
    assert!(is_image_mime(&Some("image/x-icon".to_string())));
    assert!(is_image_mime(&Some("image/bmp".to_string())));
    assert!(is_image_mime(&Some("image/tiff".to_string())));
    assert!(is_image_mime(&Some("image/avif".to_string())));
}

#[test]
fn test_asset_processing_result_embedding_skipped_in_serialization() {
    let result = AssetProcessingResult {
        node_id: "node-123".to_string(),
        image_embedding: None,
        ..Default::default()
    };

    let json = serde_json::to_string(&result).unwrap();
    assert!(
        !json.contains("\"image_embedding\":"),
        "image_embedding should be skipped when None, got: {}",
        json
    );
}

#[test]
fn test_asset_processing_result_embedding_included_in_serialization() {
    let result = AssetProcessingResult {
        node_id: "node-123".to_string(),
        image_embedding: Some(vec![1.0, 2.0, 3.0]),
        ..Default::default()
    };

    let json = serde_json::to_string(&result).unwrap();
    assert!(
        json.contains("\"image_embedding\":[1.0,2.0,3.0]"),
        "image_embedding should be present with values, got: {}",
        json
    );
}

#[test]
fn test_extract_mime_type_priority() {
    let mut node = Node::default();

    let mut metadata = HashMap::new();
    metadata.insert(
        "mime_type".to_string(),
        PropertyValue::String("image/png".to_string()),
    );
    node.properties.insert(
        "file".to_string(),
        PropertyValue::Resource(test_resource(Some(metadata))),
    );
    node.properties.insert(
        "contentType".to_string(),
        PropertyValue::String("image/jpeg".to_string()),
    );

    // file.metadata.mime_type should take priority
    assert_eq!(extract_mime_type(&node), Some("image/png".to_string()));
}
