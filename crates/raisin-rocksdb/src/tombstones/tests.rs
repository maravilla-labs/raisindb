//! Tests for tombstone operations

use super::helpers::{extract_node_id_from_key, hash_property_value};
use super::DELETION_COLUMN_FAMILIES;
use raisin_models::nodes::properties::PropertyValue;

#[test]
fn test_deletion_column_families_count() {
    // Ensure we're tracking all 12 column families (TRANSLATION_DATA is not
    // one: a node delete ends its node overlays by a read rule; block
    // overlays get a materialized `T` as well, plan Phase 11c).
    assert_eq!(DELETION_COLUMN_FAMILIES.len(), 12);
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

#[test]
fn object_values_hash_the_same_whatever_their_key_order() {
    use std::collections::HashMap;
    // Enough keys that two maps built in opposite orders iterate differently.
    let keys: Vec<String> = (0..32).map(|i| format!("k{i:02}")).collect();
    let build = |order: &mut dyn Iterator<Item = &String>| {
        let mut inner = HashMap::new();
        for k in order {
            inner.insert(k.clone(), PropertyValue::Integer(1));
        }
        let mut outer = HashMap::new();
        outer.insert("nested".to_string(), PropertyValue::Object(inner.clone()));
        outer.insert(
            "list".to_string(),
            PropertyValue::Array(vec![PropertyValue::Object(inner)]),
        );
        PropertyValue::Object(outer)
    };
    let a = hash_property_value(&build(&mut keys.iter()));
    let b = hash_property_value(&build(&mut keys.iter().rev()));
    assert_eq!(a, b);
    // Keys come out sorted at every level.
    assert!(a.starts_with(r#"{"list":[{"k00":"#), "{a}");
    // The tombstone encoding is the index writers' encoding.
    assert_eq!(
        a,
        crate::repositories::hash_property_value(&build(&mut keys.iter()))
    );
}
