//! Common types, constants, and pure helper functions for transaction module

use raisin_models::nodes::properties::{PropertyValue, RaisinReference};
use std::collections::HashMap;

/// Tombstone marker for deleted entries
pub(crate) const TOMBSTONE: &[u8] = b"T";

/// Check if a value is a tombstone marker
#[inline]
pub(crate) fn is_tombstone(value: &[u8]) -> bool {
    value == TOMBSTONE
}

/// Extract references from node properties
///
/// This recursively traverses property values to find all RaisinReference values.
pub(crate) fn extract_references(
    properties: &HashMap<String, PropertyValue>,
) -> Vec<(String, RaisinReference)> {
    let mut refs = Vec::new();

    fn visit_value(path: &str, value: &PropertyValue, refs: &mut Vec<(String, RaisinReference)>) {
        match value {
            PropertyValue::Reference(r) => {
                refs.push((path.to_string(), r.clone()));
            }
            PropertyValue::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    visit_value(&format!("{}.{}", path, i), item, refs);
                }
            }
            PropertyValue::Object(obj) => {
                for (key, val) in obj {
                    let new_path = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{}.{}", path, key)
                    };
                    visit_value(&new_path, val, refs);
                }
            }
            _ => {}
        }
    }

    for (key, value) in properties {
        visit_value(key, value, &mut refs);
    }

    refs
}
