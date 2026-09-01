//! Tests for asset processing helpers and types

use super::helpers::{extract_mime_type, extract_storage_key, is_extractable_mime, is_image_mime};
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
fn every_image_is_extractable_because_ocr_reads_it() {
    // The vocabulary and the dispatch must agree. A mime listed here with no
    // branch in `process_extractable` is the specific failure this pairing was
    // built to prevent: the job reports success and stores nothing.
    for mime in [
        "image/png",
        "image/jpeg",
        "image/webp",
        "image/gif",
        "image/tiff",
    ] {
        assert!(
            is_extractable_mime(&Some(mime.to_string())),
            "{mime} must be OCR-able, or a scanned document is findable only \
             by its filename"
        );
    }

    assert!(is_extractable_mime(&Some("application/pdf".to_string())));

    // Still deliberately absent: nothing in THIS binary reads them, and
    // claiming otherwise would report a silent success instead of the
    // `unsupported` record a backfill needs.
    assert!(!is_extractable_mime(&Some("video/mp4".to_string())));
    assert!(!is_extractable_mime(&Some(
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document".to_string()
    )));
    assert!(!is_extractable_mime(&None));
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
        extracted_text_stored: true,
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

// ---------------------------------------------------------------------------
// Extraction fingerprint — the gate that makes the write-back terminate.
//
// Extraction lands in a node property; writing a node property emits
// `node:updated`; `node:updated` is what enqueues asset processing. The
// fingerprint is the only thing standing between one upload and an infinite
// extract/write loop, so its two load-bearing properties are asserted directly:
//
//   1. it does NOT change when our own write-back adds text to the node, and
//   2. it DOES change when the underlying binary is replaced.
// ---------------------------------------------------------------------------

use super::helpers::{asset_fingerprint, extract_content_hash};

/// An asset whose `file` Resource carries a storage key and a content hash.
fn asset_with_file(storage_key: &str, content_hash: &str) -> Node {
    let mut node = Node::default();
    node.node_type = "raisin:Asset".to_string();
    let mut metadata = HashMap::new();
    metadata.insert(
        "storage_key".to_string(),
        PropertyValue::String(storage_key.to_string()),
    );
    metadata.insert(
        "content_hash".to_string(),
        PropertyValue::String(content_hash.to_string()),
    );
    node.properties.insert(
        "file".to_string(),
        PropertyValue::Resource(test_resource(Some(metadata))),
    );
    node
}

#[test]
fn fingerprint_survives_the_write_back_it_guards() {
    let node = asset_with_file("uploads/t/doc.pdf", "sha256-aaa");
    let before = asset_fingerprint(&node);

    // Exactly what `persist_extraction_artifact` writes — through the ONE
    // writer, so a drift between this test and the real write is not
    // expressible.
    let mut after_write = node.clone();
    super::ExtractionArtifact::extracted(
        before.clone(),
        "core-pdf",
        "Applying the brake pedal...".to_string(),
    )
    .apply(&mut after_write.properties, true);

    assert_eq!(
        asset_fingerprint(&after_write),
        before,
        "the write-back must not change the fingerprint, or extraction never terminates"
    );
}

/// The single most important property of the extraction artifact: a binary
/// nothing could read leaves a DURABLE, QUERYABLE record that it was skipped.
///
/// Before this, a `.docx` uploaded with no media plugin loaded produced a node
/// with no text and no trace of the attempt — indistinguishable from an empty
/// document forever, so the day a plugin gained the format there was no way to
/// find the assets it should now be run over.
#[test]
fn an_unsupported_upload_is_recorded_and_does_not_re_extract_forever() {
    use raisin_models::nodes::{extract_status, ExtractStatus, EXTRACT_STATUS_PROP};

    let node = asset_with_file("uploads/t/report.docx", "sha256-ccc");
    let before = asset_fingerprint(&node);

    let mut after_write = node.clone();
    super::ExtractionArtifact::unsupported(
        before.clone(),
        "no extractor on this server handles \
         application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    )
    .apply(&mut after_write.properties, true);

    // Queryable: this is the row a backfill selects on.
    assert_eq!(
        extract_status(&after_write.properties),
        Some(ExtractStatus::Unsupported)
    );
    assert!(matches!(
        after_write.properties.get(EXTRACT_STATUS_PROP),
        Some(PropertyValue::String(v)) if v == "unsupported"
    ));

    // And the write-back does not re-open the enqueue gate, so recording the
    // skip does not cost an extraction loop.
    assert_eq!(
        asset_fingerprint(&after_write),
        before,
        "recording a skip must not re-trigger the job that recorded it"
    );

    // Replacing the file still re-opens it: new bytes deserve a new attempt.
    let replaced = asset_with_file("uploads/t/report.docx", "sha256-ddd");
    assert_ne!(asset_fingerprint(&replaced), before);
}

/// The named spec that embeds the artifact must be a name the index-id grammar
/// accepts, or every vector it writes gets an id whose `node_id` cannot be
/// fetched.
#[test]
fn the_extracted_text_spec_name_is_a_legal_index_id_component() {
    use crate::jobs::handlers::embedding::EXTRACTED_TEXT_SPEC;

    assert!(raisin_hnsw::is_valid_spec_name(EXTRACTED_TEXT_SPEC));

    let id = raisin_hnsw::chunk_source_id("nodeA", Some(EXTRACTED_TEXT_SPEC), 0, 1);
    let parsed = raisin_hnsw::parse_index_id(&id);
    assert_eq!(
        parsed.node_id, "nodeA",
        "a search hit must name a fetchable node"
    );
    assert_eq!(parsed.spec.as_deref(), Some(EXTRACTED_TEXT_SPEC));
}

#[test]
fn fingerprint_changes_when_the_binary_is_replaced() {
    let original = asset_fingerprint(&asset_with_file("uploads/t/doc.pdf", "sha256-aaa"));

    // Same key, new bytes (re-upload in place).
    assert_ne!(
        asset_fingerprint(&asset_with_file("uploads/t/doc.pdf", "sha256-bbb")),
        original,
        "new content hash must re-open the extraction gate"
    );

    // New key, same hash.
    assert_ne!(
        asset_fingerprint(&asset_with_file("uploads/t/doc-v2.pdf", "sha256-aaa")),
        original,
        "new storage key must re-open the extraction gate"
    );
}

#[test]
fn fingerprint_is_defined_with_neither_hash_nor_key() {
    // A metadata-only mail attachment: no bytes yet. Must not panic, and must
    // not collide with a real binary's fingerprint.
    let mut bare = Node::default();
    bare.node_type = "raisin:Asset".to_string();
    let bare_fp = asset_fingerprint(&bare);
    assert!(bare_fp.starts_with("v1:"));
    assert_ne!(
        bare_fp,
        asset_fingerprint(&asset_with_file("uploads/t/doc.pdf", "sha256-aaa"))
    );
}

#[test]
fn content_hash_is_read_from_every_spelling_the_writers_use() {
    // Resource metadata (upload path).
    assert_eq!(
        extract_content_hash(&asset_with_file("k", "sha-res")),
        Some("sha-res".to_string())
    );

    // Top-level property: `raisin:Asset` declares `content_hash`, and both the
    // package installer and the on-demand attachment fetch set it there.
    let mut top = Node::default();
    top.properties.insert(
        "content_hash".to_string(),
        PropertyValue::String("sha-top".to_string()),
    );
    assert_eq!(extract_content_hash(&top), Some("sha-top".to_string()));

    assert_eq!(extract_content_hash(&Node::default()), None);
}
