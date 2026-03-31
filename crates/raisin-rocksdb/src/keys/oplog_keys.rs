//! Operation log and vector clock key functions
//!
//! Keys for CRDT replication operation logs and vector clock snapshots.

use super::KeyBuilder;

/// Encode a u64 as big-endian bytes (for monotonic ordering)
pub fn encode_u64(value: u64) -> [u8; 8] {
    value.to_be_bytes()
}

/// Decode big-endian u64 bytes
pub fn decode_u64(bytes: &[u8]) -> Result<u64, std::array::TryFromSliceError> {
    let arr: [u8; 8] = bytes.try_into()?;
    Ok(u64::from_be_bytes(arr))
}

/// Operation log key: {tenant}\0{repo}\0{node_id}\0{op_seq}\0{timestamp_ms}
pub fn oplog_key(
    tenant_id: &str,
    repo_id: &str,
    node_id: &str,
    op_seq: u64,
    timestamp_ms: u64,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(node_id)
        .push_bytes(&encode_u64(op_seq))
        .push_bytes(&encode_u64(timestamp_ms))
        .build()
}

/// Operation log prefix for a specific tenant/repo: {tenant}\0{repo}\0
pub fn oplog_tenant_repo_prefix(tenant_id: &str, repo_id: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .build_prefix()
}

/// Operation log prefix for a specific node: {tenant}\0{repo}\0{node_id}\0
pub fn oplog_node_prefix(tenant_id: &str, repo_id: &str, node_id: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(node_id)
        .build_prefix()
}

/// Operation log prefix from a specific sequence: {tenant}\0{repo}\0{node_id}\0{op_seq}\0
pub fn oplog_from_seq_prefix(
    tenant_id: &str,
    repo_id: &str,
    node_id: &str,
    op_seq: u64,
) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(node_id)
        .push_bytes(&encode_u64(op_seq))
        .build_prefix()
}

/// Vector clock snapshot key: {tenant}\0{repo}\0vc_snapshot
pub fn vector_clock_snapshot_key(tenant_id: &str, repo_id: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push("vc_snapshot")
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oplog_key_encoding() {
        let key = oplog_key("tenant1", "repo1", "node-us-west", 42, 1699999999000);
        let seq_bytes = encode_u64(42);
        let timestamp_bytes = encode_u64(1699999999000);
        let expected_parts: Vec<&[u8]> = vec![
            b"tenant1" as &[u8],
            b"repo1" as &[u8],
            b"node-us-west" as &[u8],
            &seq_bytes,
            &timestamp_bytes,
        ];
        let mut expected = Vec::new();
        for (i, part) in expected_parts.iter().enumerate() {
            if i > 0 {
                expected.push(0);
            }
            expected.extend_from_slice(part);
        }
        assert_eq!(key, expected);
    }

    #[test]
    fn test_oplog_key_ordering() {
        let key1 = oplog_key("t1", "r1", "node1", 10, 1000);
        let key2 = oplog_key("t1", "r1", "node1", 20, 1000);
        let key3 = oplog_key("t1", "r1", "node1", 20, 2000);
        assert!(key2 > key1);
        assert!(key3 > key2);
    }

    #[test]
    fn test_oplog_prefixes() {
        let tenant_repo_prefix = oplog_tenant_repo_prefix("tenant1", "repo1");
        let node_prefix = oplog_node_prefix("tenant1", "repo1", "node1");
        let seq_prefix = oplog_from_seq_prefix("tenant1", "repo1", "node1", 100);
        assert!(node_prefix.starts_with(&tenant_repo_prefix[..tenant_repo_prefix.len() - 1]));
        assert!(seq_prefix.starts_with(&node_prefix[..node_prefix.len() - 1]));
    }

    #[test]
    fn test_u64_encoding_roundtrip() {
        let values = vec![0, 1, 42, 255, 65535, u64::MAX];
        for val in values {
            let encoded = encode_u64(val);
            let decoded = decode_u64(&encoded).unwrap();
            assert_eq!(val, decoded);
        }
    }

    #[test]
    fn test_vector_clock_snapshot_key_encoding() {
        let key = vector_clock_snapshot_key("tenant1", "repo1");
        let expected = b"tenant1\0repo1\0vc_snapshot".to_vec();
        assert_eq!(key, expected);
    }

    #[test]
    fn test_vector_clock_snapshot_key_isolation() {
        let key1 = vector_clock_snapshot_key("tenant1", "repo1");
        let key2 = vector_clock_snapshot_key("tenant2", "repo1");
        assert_ne!(key1, key2);

        let key3 = vector_clock_snapshot_key("tenant1", "repo1");
        let key4 = vector_clock_snapshot_key("tenant1", "repo2");
        assert_ne!(key3, key4);
    }
}
