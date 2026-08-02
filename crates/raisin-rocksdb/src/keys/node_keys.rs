//! Node, path, and node_path key functions
//!
//! Keys for storing nodes, path-to-node indexes, and node-to-path reverse indexes.

use super::KeyBuilder;
use raisin_hlc::HLC;

/// Node key: {tenant}\0{repo}\0{branch}\0{workspace}\0nodes\0{node_id}
#[deprecated(
    since = "0.1.0",
    note = "Use node_key_versioned for revision-aware storage"
)]
pub fn node_key(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push("nodes")
        .push(node_id)
        .build()
}

/// Revision-aware node key: {tenant}\0{repo}\0{branch}\0{workspace}\0nodes\0{node_id}\0{~revision}
pub fn node_key_versioned(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    revision: &HLC,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push("nodes")
        .push(node_id)
        .push_revision(revision)
        .build()
}

/// Node adjacency key: {tenant}\0{repo}\0{branch}\0{workspace}\0nodes\0{node_id}\0adj\0{~revision}
pub fn node_adjacency_key_versioned(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    revision: &HLC,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push("nodes")
        .push(node_id)
        .push("adj")
        .push_revision(revision)
        .build()
}

/// Node key prefix (without revision): {tenant}\0{repo}\0{branch}\0{workspace}\0nodes\0{node_id}\0
pub fn node_key_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push("nodes")
        .push(node_id)
        .build_prefix()
}

/// Path index key: {tenant}\0{repo}\0{branch}\0{workspace}\0path\0{path}
#[deprecated(
    since = "0.1.0",
    note = "Use path_index_key_versioned for revision-aware storage"
)]
pub fn path_index_key(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    path: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push("path")
        .push(path)
        .build()
}

/// Revision-aware path index key: {tenant}\0{repo}\0{branch}\0{workspace}\0path\0{path}\0{~revision}
pub fn path_index_key_versioned(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    path: &str,
    revision: &HLC,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push("path")
        .push(path)
        .push_revision(revision)
        .build()
}

/// Path index key prefix (without revision): {tenant}\0{repo}\0{branch}\0{workspace}\0path\0{path}\0
pub fn path_index_key_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    path: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push("path")
        .push(path)
        .build_prefix()
}

/// Decode HLC from path_index key
///
/// The descending HLC is a FIXED 16-byte trailer, and it may itself contain
/// `0x00` bytes: `encode_descending` stores `!timestamp_ms`, so any timestamp
/// byte equal to `0xFF` becomes a NUL (and `!counter` is all-NUL at
/// `u64::MAX`). Locating it by searching for the last separator therefore
/// truncated it for ~2% of revisions, the decode returned `None`, and the
/// caller SKIPPED that index entry — a live node silently vanished from
/// `path LIKE 'prefix%'` scans. Take the fixed-width suffix instead, and only
/// require that the byte in front of it is the separator.
pub fn decode_revision_from_path_index_key(key: &[u8]) -> Option<HLC> {
    let sep_pos = key.len().checked_sub(17)?;
    if key[sep_pos] != 0 {
        return None;
    }
    HLC::decode_descending(&key[sep_pos + 1..]).ok()
}

/// Decode path from path_index key
///
/// Strips the fixed 16-byte HLC trailer BEFORE splitting: the HLC can contain
/// `0x00` bytes (see [`decode_revision_from_path_index_key`]), so splitting the
/// whole key produces a variable number of trailing parts.
pub fn decode_path_from_path_index_key(key: &[u8]) -> Option<String> {
    let sep_pos = key.len().checked_sub(17)?;
    if key[sep_pos] != 0 {
        return None;
    }
    let parts: Vec<&[u8]> = key[..sep_pos].split(|&b| b == 0).collect();
    if parts.len() >= 6 && parts[4] == b"path" {
        return String::from_utf8(parts[5].to_vec()).ok();
    }
    None
}

/// Revision-aware node_path key: {tenant}\0{repo}\0{branch}\0{workspace}\0node_path\0{node_id}\0{~revision}
pub fn node_path_key_versioned(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    revision: &HLC,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push("node_path")
        .push(node_id)
        .push_revision(revision)
        .build()
}

/// Node_path key prefix (without revision): {tenant}\0{repo}\0{branch}\0{workspace}\0node_path\0{node_id}\0
pub fn node_path_key_prefix(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push("node_path")
        .push(node_id)
        .build_prefix()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_key_with_hlc() {
        let hlc = HLC::new(1705843009213693952, 42);
        let key = node_key_versioned("tenant1", "repo1", "main", "workspace1", "node123", &hlc);
        assert!(!key.is_empty());
        let hlc_encoded = hlc.encode_descending();
        assert!(key
            .windows(hlc_encoded.len())
            .any(|window| window == hlc_encoded));
    }

    #[test]
    fn test_decode_revision_from_path_index_key() {
        let hlc = HLC::new(1705843009213693952, 42);
        let key = path_index_key_versioned("t1", "r1", "main", "ws1", "/foo", &hlc);
        let decoded = decode_revision_from_path_index_key(&key);
        assert_eq!(decoded, Some(hlc));
    }

    /// A descending HLC whose encoding CONTAINS `0x00` bytes must still decode,
    /// and the path must still parse. `!timestamp_ms` turns every `0xFF`
    /// timestamp byte into a NUL, so this is ~2% of real revisions — locating
    /// the trailer by the last separator dropped them, and the scan then
    /// skipped a live node.
    #[test]
    fn decodes_path_index_key_whose_hlc_contains_nul_bytes() {
        // timestamp byte 0xFF -> NUL after negation; counter u64::MAX -> 8 NULs.
        let hlc = HLC::new(0x0000_01FF_00FF_FF00, u64::MAX);
        let encoded = hlc.encode_descending();
        assert!(encoded.contains(&0), "fixture must exercise embedded NULs");

        let key = path_index_key_versioned("t1", "r1", "main", "ws1", "/joba/t-000", &hlc);
        assert_eq!(decode_revision_from_path_index_key(&key), Some(hlc));
        assert_eq!(
            decode_path_from_path_index_key(&key),
            Some("/joba/t-000".to_string())
        );
    }

    #[test]
    fn test_decode_path_from_path_index_key() {
        let hlc = HLC::new(1705843009213693952, 42);
        let key = path_index_key_versioned("t1", "r1", "main", "ws1", "/foo/bar", &hlc);
        assert_eq!(
            decode_path_from_path_index_key(&key),
            Some("/foo/bar".to_string())
        );
    }

    #[test]
    fn test_node_key() {
        #[allow(deprecated)]
        let key = node_key("tenant1", "repo1", "main", "workspace1", "node123");
        let expected = b"tenant1\0repo1\0main\0workspace1\0nodes\0node123".to_vec();
        assert_eq!(key, expected);
    }
}
