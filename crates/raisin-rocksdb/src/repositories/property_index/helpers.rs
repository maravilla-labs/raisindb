//! Shared helpers for property index operations
//!
//! Contains tombstone handling, key extraction, and key parsing utilities
//! used by the query, scan, and write submodules.

/// Tombstone marker for deleted property index entries
pub(super) const TOMBSTONE: &[u8] = b"T";

/// Check if a value is a tombstone marker
#[inline]
pub(super) fn is_tombstone(value: &[u8]) -> bool {
    value == TOMBSTONE
}

/// Extract node_id from a property index key
///
/// Works for both versioned and non-versioned keys because node_id is always
/// the last component after the final null byte separator.
///
/// Key formats:
/// - Versioned: `{prefix}\0{value_hash}\0{16_byte_HLC}\0{node_id}`
/// - Legacy: `{prefix}\0{value_hash}\0{node_id}`
///
/// **Important**: Cannot use `split('\0')` because HLC's 16 binary bytes
/// may contain `0x00` values that would incorrectly split the key.
#[inline]
pub(super) fn extract_node_id_from_key(key: &[u8]) -> Option<String> {
    // Find the last null byte - node_id is everything after it
    let last_null = key.iter().rposition(|&b| b == 0)?;
    let node_id_bytes = &key[last_null + 1..];
    if node_id_bytes.is_empty() {
        return None;
    }
    String::from_utf8(node_id_bytes.to_vec()).ok()
}

/// Parse (node_id, property_value) from a property index key
///
/// Key format:
/// - Versioned: `{tenant}\0{repo}\0{branch}\0{workspace}\0prop\0{property_name}\0{value}\0{16_byte_HLC}\0{node_id}`
/// - Legacy: `{tenant}\0{repo}\0{branch}\0{workspace}\0prop\0{property_name}\0{value}\0{node_id}`
///
/// **Important**: Cannot use `split('\0')` because HLC's 16 binary bytes may contain 0x00.
///
/// Strategy:
/// 1. node_id is always last (after final null)
/// 2. Walk through first 6 null bytes to reach value_hash position
/// 3. value_hash extends until the next null byte
pub(super) fn parse_entry_components(key: &[u8]) -> Option<(String, String)> {
    // Extract node_id from the end (last component after final null)
    let last_null = key.iter().rposition(|&b| b == 0)?;
    let node_id_bytes = &key[last_null + 1..];
    if node_id_bytes.is_empty() {
        return None;
    }
    let node_id = String::from_utf8(node_id_bytes.to_vec()).ok()?;

    // Walk through the key to find the 6th null byte (after property_name)
    // Parts: tenant(0), repo(1), branch(2), workspace(3), tag(4), property_name(5), value(6)
    let mut null_count = 0;
    let mut value_start = 0;

    for (i, &byte) in key.iter().enumerate() {
        if byte == 0 {
            null_count += 1;
            if null_count == 6 {
                // After 6th null, value_hash starts at i+1
                value_start = i + 1;
                break;
            }
        }
    }

    if null_count < 6 || value_start >= key.len() {
        return None;
    }

    // Find end of value_hash (next null byte after value_start)
    let value_end = key[value_start..].iter().position(|&b| b == 0)?;
    let value_bytes = &key[value_start..value_start + value_end];

    // Check if this is a binary timestamp (8 bytes) vs string value
    let value_str = if value_bytes.len() == 8 {
        // Binary i64 timestamp (big-endian) - convert to string representation
        // This handles __created_at, __updated_at, __published_at indexes
        let timestamp_bytes: [u8; 8] = value_bytes.try_into().ok()?;
        let timestamp_micros = i64::from_be_bytes(timestamp_bytes);
        timestamp_micros.to_string()
    } else {
        // Regular string value - decode as UTF-8
        String::from_utf8_lossy(value_bytes).to_string()
    };

    Some((node_id, value_str))
}
