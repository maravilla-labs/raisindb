//! Tests for tombstone operations

use super::helpers::{extract_node_id_from_key, hash_property_value};
use super::DELETION_COLUMN_FAMILIES;
use raisin_models::nodes::properties::PropertyValue;

#[test]
fn test_deletion_column_families_count() {
    // Ensure we're tracking all 10 column families
    assert_eq!(DELETION_COLUMN_FAMILIES.len(), 10);
}

#[test]
fn test_extract_node_id_from_key() {
    let key = b"tenant\0repo\0branch\0workspace\0prefix\0node123";
    assert_eq!(extract_node_id_from_key(key), Some("node123".to_string()));
}

#[test]
fn test_extract_node_id_from_empty_suffix() {
    let key = b"tenant\0repo\0branch\0";
    assert_eq!(extract_node_id_from_key(key), None);
}

#[test]
fn test_hash_property_value_string() {
    let value = PropertyValue::String("test".to_string());
    assert_eq!(hash_property_value(&value), "test");
}

#[test]
fn test_hash_property_value_integer() {
    let value = PropertyValue::Integer(42);
    assert_eq!(hash_property_value(&value), "42");
}
