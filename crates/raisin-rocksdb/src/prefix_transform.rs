//! Custom prefix transformers for RocksDB column families

use rocksdb::SliceTransform;

/// Extract prefix up to 6th null byte for ORDERED_CHILDREN keys
///
/// For keys: `{tenant}\0{repo}\0{branch}\0{workspace}\0ordered\0{parent_id}\0{order_label}\0{~rev}\0{child_id}`
///
/// Returns prefix up to (and including) `parent_id`.
fn extract_parent_prefix(key: &[u8]) -> &[u8] {
    let mut delimiters_seen = 0;
    for (i, &byte) in key.iter().enumerate() {
        if byte == 0 {
            delimiters_seen += 1;
            if delimiters_seen == 6 {
                // Return slice up to (but not including) this delimiter
                return &key[..i];
            }
        }
    }
    // If we don't find 6 delimiters, use the full key
    key
}

/// Create a prefix extractor for ORDERED_CHILDREN column family
///
/// Extracts the prefix up to the 6th null byte (after parent_id).
/// This enables prefix bloom filters for efficient scans of children under a parent.
pub fn create_ordered_children_prefix() -> SliceTransform {
    SliceTransform::create(
        "ordered_children_prefix",
        extract_parent_prefix,
        None, // in_domain: always returns true (all keys valid)
    )
}

/// Extract prefix up to 6th null byte for SPATIAL_INDEX keys
///
/// For keys: `{tenant}\0{repo}\0{branch}\0{workspace}\0geo\0{property}\0{geohash}\0{~rev}\0{node_id}`
///
/// Returns prefix up to (and including) `property` name.
/// This enables efficient range scans by geohash prefix for a specific property.
fn extract_spatial_property_prefix(key: &[u8]) -> &[u8] {
    let mut delimiters_seen = 0;
    for (i, &byte) in key.iter().enumerate() {
        if byte == 0 {
            delimiters_seen += 1;
            if delimiters_seen == 6 {
                // Return slice up to (but not including) this delimiter
                // This gives us: tenant/repo/branch/workspace/geo/property
                return &key[..i];
            }
        }
    }
    // If we don't find 6 delimiters, use the full key
    key
}

/// Create a prefix extractor for SPATIAL_INDEX column family
///
/// Extracts the prefix up to the 6th null byte (after property name).
/// This enables efficient geohash range scans for proximity queries.
///
/// With this prefix, queries like "find all nodes within 100m of point X for property 'location'"
/// can efficiently scan geohash cells matching a specific prefix without reading unrelated data.
pub fn create_spatial_index_prefix() -> SliceTransform {
    SliceTransform::create(
        "spatial_index_prefix",
        extract_spatial_property_prefix,
        None, // in_domain: always returns true (all keys valid)
    )
}
