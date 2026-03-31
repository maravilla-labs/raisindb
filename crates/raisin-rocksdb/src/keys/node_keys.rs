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
pub fn decode_revision_from_path_index_key(key: &[u8]) -> Option<HLC> {
    let last_sep = key.iter().rposition(|&b| b == 0)?;
    let hlc_bytes = &key[last_sep + 1..];
    if hlc_bytes.len() == 16 {
        HLC::decode_descending(hlc_bytes).ok()
    } else {
        None
    }
}

/// Decode path from path_index key
pub fn decode_path_from_path_index_key(key: &[u8]) -> Option<String> {
    let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
    if parts.len() >= 7 && parts[4] == b"path" {
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

    #[test]
    fn test_node_key() {
        #[allow(deprecated)]
        let key = node_key("tenant1", "repo1", "main", "workspace1", "node123");
        let expected = b"tenant1\0repo1\0main\0workspace1\0nodes\0node123".to_vec();
        assert_eq!(key, expected);
    }
}
