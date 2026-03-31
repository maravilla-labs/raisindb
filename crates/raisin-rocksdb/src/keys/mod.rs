// TODO(v0.2): Reserved key functions for future index types
#![allow(dead_code)]

//! Key encoding and decoding utilities for RocksDB.
//!
//! This module provides functions to encode and decode keys for various data structures
//! stored in RocksDB. The key format follows the specification:
//!
//! ```text
//! {tenant}\0{repo}\0{branch}\0{workspace}\0{index_tag}\0{field}\0{value}\0{~revision}\0{node_id}
//! ```
//!
//! Key features:
//! - Null-byte separated components for lexicographic ordering
//! - Descending revision encoding (~rev = bitwise_not(revision)) for newest-first ordering
//! - Prefix-based isolation for tenants, repos, and branches

mod graph_cache_keys;
mod identity_keys;
mod index_keys;
mod node_keys;
mod oplog_keys;
mod ordered_children_keys;
mod reference_keys;
mod relation_keys;
mod repository_keys;
mod schema_keys;
mod spatial_keys;

// Re-export all public items from submodules
pub use graph_cache_keys::*;
pub use identity_keys::*;
pub use index_keys::*;
pub use node_keys::*;
pub use oplog_keys::*;
pub use ordered_children_keys::*;
pub use reference_keys::*;
pub use relation_keys::*;
pub use repository_keys::*;
pub use schema_keys::*;
pub use spatial_keys::*;

use raisin_hlc::HLC;

/// Key separator (null byte)
const SEP: u8 = 0;

/// Encode an HLC in descending order for RocksDB keys
///
/// Uses bitwise NOT on both timestamp and counter components to achieve
/// descending lexicographic order (newest HLCs sort first).
pub fn encode_descending_revision(hlc: &HLC) -> Vec<u8> {
    hlc.encode_descending().to_vec()
}

/// Decode a descending-encoded HLC from RocksDB key bytes
pub fn decode_descending_revision(bytes: &[u8]) -> Result<HLC, Box<dyn std::error::Error>> {
    HLC::decode_descending(bytes).map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
}

/// Extract and decode the HLC revision from a RocksDB key
///
/// Takes the last 16 bytes of a key (where HLC is always stored)
/// and decodes the HLC from those bytes.
///
/// # Why we need this
/// HLC revisions are always stored as the last 16 bytes of versioned keys.
/// We cannot use null-byte splitting to find the revision because HLC's
/// descending encoding (bitwise NOT) can produce null bytes within the 16-byte HLC.
pub fn extract_revision_from_key(key: &[u8]) -> Result<HLC, Box<dyn std::error::Error>> {
    if key.len() < 16 {
        return Err(format!(
            "Key too short to contain HLC revision: {} bytes (need at least 16)",
            key.len()
        )
        .into());
    }

    let rev_bytes = &key[key.len() - 16..];
    decode_descending_revision(rev_bytes)
}

/// Builder for constructing RocksDB keys
#[derive(Debug, Clone)]
pub struct KeyBuilder {
    parts: Vec<Vec<u8>>,
}

impl KeyBuilder {
    pub fn new() -> Self {
        Self { parts: Vec::new() }
    }

    /// Add a string component
    pub fn push(mut self, s: &str) -> Self {
        self.parts.push(s.as_bytes().to_vec());
        self
    }

    /// Add a byte component
    pub fn push_bytes(mut self, bytes: &[u8]) -> Self {
        self.parts.push(bytes.to_vec());
        self
    }

    /// Add an HLC revision component (descending encoding)
    pub fn push_revision(mut self, hlc: &HLC) -> Self {
        self.parts.push(hlc.encode_descending().to_vec());
        self
    }

    /// Build the final key with null-byte separators
    pub fn build(self) -> Vec<u8> {
        let mut result = Vec::new();
        for (i, part) in self.parts.iter().enumerate() {
            if i > 0 {
                result.push(SEP);
            }
            result.extend_from_slice(part);
        }
        result
    }

    /// Build a prefix key (for scanning)
    pub fn build_prefix(self) -> Vec<u8> {
        let mut result = self.build();
        result.push(SEP);
        result
    }
}

impl Default for KeyBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Prefix for all keys in a tenant
pub fn tenant_prefix(tenant_id: &str) -> Vec<u8> {
    KeyBuilder::new().push(tenant_id).build_prefix()
}

/// Prefix for all keys in a repository
pub fn repo_prefix(tenant_id: &str, repo_id: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .build_prefix()
}

/// Prefix for all keys in a branch
pub fn branch_prefix(tenant_id: &str, repo_id: &str, branch: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .build_prefix()
}

/// Prefix for all keys in a workspace
pub fn workspace_prefix(tenant_id: &str, repo_id: &str, branch: &str, workspace: &str) -> Vec<u8> {
    KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .build_prefix()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_descending_revision_encoding() {
        let hlc1 = HLC::new(1000, 0);
        let hlc100 = HLC::new(1100, 0);
        let hlc1000 = HLC::new(2000, 0);

        let enc1 = encode_descending_revision(&hlc1);
        let enc100 = encode_descending_revision(&hlc100);
        let enc1000 = encode_descending_revision(&hlc1000);

        assert!(enc1000 < enc100);
        assert!(enc100 < enc1);

        assert_eq!(decode_descending_revision(&enc1).unwrap(), hlc1);
        assert_eq!(decode_descending_revision(&enc100).unwrap(), hlc100);
        assert_eq!(decode_descending_revision(&enc1000).unwrap(), hlc1000);
    }

    #[test]
    fn test_key_builder() {
        let key = KeyBuilder::new()
            .push("tenant1")
            .push("repo1")
            .push("main")
            .push("workspace1")
            .build();

        assert_eq!(key, b"tenant1\0repo1\0main\0workspace1".to_vec());
    }
}
