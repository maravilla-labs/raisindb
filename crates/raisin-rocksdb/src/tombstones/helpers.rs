//! Helper functions for tombstone operations

use raisin_models::nodes::properties::PropertyValue;

/// Hash a property value for indexing — the SAME encoding the index writers
/// use, or a tombstone would not land on the row it is meant to hide.
pub(super) fn hash_property_value(value: &PropertyValue) -> String {
    crate::repositories::hash_property_value(value)
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
