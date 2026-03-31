//! Helper functions for tombstone operations

use raisin_models::nodes::properties::PropertyValue;
use std::collections::HashMap;

/// Hash a property value for indexing
///
/// Creates a stable string representation suitable for use in property index keys.
pub(super) fn hash_property_value(value: &PropertyValue) -> String {
    match value {
        PropertyValue::Null => "null".to_string(),
        PropertyValue::String(s) => s.clone(),
        PropertyValue::Integer(i) => i.to_string(),
        PropertyValue::Float(f) => f.to_string(),
        PropertyValue::Decimal(d) => d.to_string(),
        PropertyValue::Boolean(b) => b.to_string(),
        PropertyValue::Date(d) => {
            let nanos = d.timestamp_nanos_opt().unwrap_or(0);
            format!("{:020}", nanos as i128)
        }
        PropertyValue::Url(u) => u.url.clone(),
        PropertyValue::Reference(r) => format!("ref:{}", r.id),
        PropertyValue::Resource(res) => format!("resource:{}", res.uuid),
        PropertyValue::Element(block) => format!("block:{}", block.uuid),
        PropertyValue::Composite(container) => format!("container:{}", container.uuid),
        PropertyValue::Vector(v) => format!("vector:{}d", v.len()),
        PropertyValue::Geometry(g) => {
            use raisin_models::nodes::properties::GeoJson;
            let geom_type = match g {
                GeoJson::Point { .. } => "Point",
                GeoJson::LineString { .. } => "LineString",
                GeoJson::Polygon { .. } => "Polygon",
                GeoJson::MultiPoint { .. } => "MultiPoint",
                GeoJson::MultiLineString { .. } => "MultiLineString",
                GeoJson::MultiPolygon { .. } => "MultiPolygon",
                GeoJson::GeometryCollection { .. } => "GeometryCollection",
            };
            format!("geometry:{}", geom_type)
        }
        PropertyValue::Array(_) | PropertyValue::Object(_) => {
            serde_json::to_string(value).unwrap_or_else(|_| "invalid".to_string())
        }
    }
}

/// Reference info extracted from properties
#[derive(Debug, Clone)]
pub struct ExtractedReference {
    pub workspace: String,
    pub path: String,
}

/// Extract references from node properties
///
/// Recursively walks properties to find Reference values and returns them
/// with their property path (for nested references like "items[0].ref").
pub fn extract_references(
    properties: &HashMap<String, PropertyValue>,
) -> Vec<(String, ExtractedReference)> {
    let mut refs = Vec::new();
    for (key, value) in properties {
        extract_references_recursive(key, value, &mut refs);
    }
    refs
}

fn extract_references_recursive(
    path: &str,
    value: &PropertyValue,
    refs: &mut Vec<(String, ExtractedReference)>,
) {
    match value {
        PropertyValue::Reference(r) => {
            refs.push((
                path.to_string(),
                ExtractedReference {
                    workspace: r.workspace.clone(),
                    path: r.path.clone(),
                },
            ));
        }
        PropertyValue::Array(arr) => {
            for (i, item) in arr.iter().enumerate() {
                let item_path = format!("{}[{}]", path, i);
                extract_references_recursive(&item_path, item, refs);
            }
        }
        PropertyValue::Object(obj) => {
            for (key, val) in obj {
                let nested_path = format!("{}.{}", path, key);
                extract_references_recursive(&nested_path, val, refs);
            }
        }
        _ => {}
    }
}

/// Extract node_id from a key (last component after final \0)
pub(super) fn extract_node_id_from_key(key: &[u8]) -> Option<String> {
    let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
    let node_id_bytes = parts.last()?;
    if node_id_bytes.is_empty() {
        return None;
    }
    String::from_utf8(node_id_bytes.to_vec()).ok()
}

/// Parse relation details from a forward relation key
///
/// Key format after prefix: {relation_type}\0{~revision}\0{target_id}
/// Returns (relation_type, target_workspace, target_id) if parseable
pub(super) fn parse_relation_from_forward_key(
    key: &[u8],
    prefix: &[u8],
) -> Option<(String, String, String)> {
    if key.len() <= prefix.len() {
        return None;
    }

    let suffix = &key[prefix.len()..];
    let parts: Vec<&[u8]> = suffix.split(|&b| b == 0).collect();

    if parts.len() < 3 {
        return None;
    }

    let relation_type = String::from_utf8(parts[0].to_vec()).ok()?;
    // parts[1] is the revision (skip)
    let target_id = String::from_utf8(parts[parts.len() - 1].to_vec()).ok()?;

    // For now, assume same workspace for reverse relation
    // TODO: Parse target_workspace from value if stored there
    Some((relation_type, String::new(), target_id))
}

/// Extract locale from a translation key
///
/// Key format after node prefix: {locale}\0{~revision}
pub(super) fn extract_locale_from_translation_key(key: &[u8], prefix: &[u8]) -> Option<String> {
    if key.len() <= prefix.len() {
        return None;
    }

    let suffix = &key[prefix.len()..];
    let parts: Vec<&[u8]> = suffix.split(|&b| b == 0).collect();

    if parts.is_empty() {
        return None;
    }

    String::from_utf8(parts[0].to_vec()).ok()
}
