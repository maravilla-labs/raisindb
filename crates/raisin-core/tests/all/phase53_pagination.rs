//! Phase 5.3 cursor-based pagination tests
//!
//! Tests paginated tree queries for large repositories

use raisin_hlc::HLC;

#[tokio::test]
async fn test_cursor_encoding_decoding() {
    // Test that PageCursor can be encoded and decoded correctly
    let revision = HLC::new(42, 0);
    let cursor = raisin_models::tree::PageCursor::new("test_key".to_string(), Some(revision));

    let encoded = cursor.encode().unwrap();
    assert!(!encoded.is_empty());

    let decoded = raisin_models::tree::PageCursor::decode(&encoded).unwrap();
    assert_eq!(decoded.last_key, "test_key");
    assert_eq!(decoded.revision, Some(revision));
}

#[tokio::test]
async fn test_page_structure() {
    // Test that Page struct works correctly
    let items = vec![1, 2, 3];
    let revision = HLC::new(1, 0);
    let cursor = raisin_models::tree::PageCursor::new("key_3".to_string(), Some(revision));

    let page = raisin_models::tree::Page::new(items, Some(cursor));

    assert_eq!(page.items.len(), 3);
    assert!(page.next_cursor.is_some());
    assert_eq!(page.next_cursor.unwrap().last_key, "key_3");
    assert!(page.total.is_none());
}

#[tokio::test]
async fn test_page_with_total() {
    let items = vec![1, 2, 3];
    let page = raisin_models::tree::Page::new(items, None).with_total(10);

    assert_eq!(page.total, Some(10));
    assert!(page.next_cursor.is_none());
}
