// SPDX-License-Identifier: BSL-1.1

//! Property extraction and flattening for full-text indexing.

use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::fulltext::FullTextIndexJob;
use std::collections::HashMap;

/// Flattens node properties into searchable text using schema-driven approach.
pub(crate) fn flatten_properties(
    job: &FullTextIndexJob,
    properties: &HashMap<String, PropertyValue>,
) -> String {
    if let Some(props_to_index) = &job.properties_to_index {
        let mut parts = Vec::new();
        for prop_name in props_to_index {
            if let Some(prop_value) = properties.get(prop_name) {
                if let Some(text) = property_value_to_text(prop_value) {
                    parts.push(format!("{}: {}", prop_name, text));
                }
            }
        }
        tracing::trace!(
            properties = ?props_to_index,
            extracted_count = parts.len(),
            "Extracted schema-defined fulltext properties"
        );
        return parts.join(" ");
    }

    tracing::trace!("No schema-defined properties, indexing all string properties");
    properties
        .iter()
        .filter_map(|(k, v)| match v {
            PropertyValue::String(s) => Some(format!("{}: {}", k, s)),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Convert property value to text for indexing using iterative approach.
pub(crate) fn property_value_to_text(value: &PropertyValue) -> Option<String> {
    let mut result_parts = Vec::new();
    let mut work_stack = vec![value];

    while let Some(current) = work_stack.pop() {
        match current {
            PropertyValue::String(s) => {
                if !s.is_empty() {
                    result_parts.push(s.clone());
                }
            }
            PropertyValue::Integer(n) => result_parts.push(n.to_string()),
            PropertyValue::Float(f) => result_parts.push(f.to_string()),
            PropertyValue::Boolean(b) => result_parts.push(b.to_string()),
            PropertyValue::Date(d) => result_parts.push(d.to_string()),
            PropertyValue::Decimal(dec) => result_parts.push(dec.to_string()),
            PropertyValue::Array(arr) => {
                for item in arr.iter().rev() {
                    work_stack.push(item);
                }
            }
            PropertyValue::Object(obj) => {
                for value in obj.values() {
                    work_stack.push(value);
                }
            }
            PropertyValue::Composite(bc) => {
                for block in &bc.items {
                    for value in block.content.values() {
                        work_stack.push(value);
                    }
                }
            }
            PropertyValue::Element(block) => {
                for value in block.content.values() {
                    work_stack.push(value);
                }
            }
            PropertyValue::Null
            | PropertyValue::Url(_)
            | PropertyValue::Reference(_)
            | PropertyValue::Resource(_)
            | PropertyValue::Vector(_)
            | PropertyValue::Geometry(_) => {}
        }
    }

    if result_parts.is_empty() {
        None
    } else {
        Some(result_parts.join(" "))
    }
}
