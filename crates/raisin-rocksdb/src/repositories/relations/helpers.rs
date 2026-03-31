//! Helper functions for relation repository operations
//!
//! This module provides shared utilities for:
//! - Serialization/deserialization of relations
//! - Key parsing and component extraction
//! - Iteration helpers with tombstone checking
//! - Common error handling patterns

use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use raisin_models::nodes::{FullRelation, RelationRef};
use rocksdb::{ColumnFamily, DB};
use std::collections::HashSet;

use crate::keys::decode_descending_revision;

/// Compact relation representation for packed storage
///
/// This struct is optimized for storage efficiency when packing multiple relations
/// into a single value. It mirrors the essential fields of FullRelation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompactRelation {
    pub relation_type: String,
    pub target_id: String,
    pub target_workspace: String,
    pub target_node_type: String,
    pub weight: Option<f32>,
}

/// Tombstone marker for deleted relations
pub const TOMBSTONE: &[u8] = b"T";

/// Check if a value is a tombstone marker
#[inline]
pub fn is_tombstone(value: &[u8]) -> bool {
    value == TOMBSTONE
}

/// Serialize a RelationRef to MessagePack bytes
pub(super) fn serialize_relation_ref(relation: &RelationRef) -> Result<Vec<u8>> {
    rmp_serde::to_vec(relation)
        .map_err(|e| Error::storage(format!("Failed to serialize relation: {}", e)))
}

/// Serialize a FullRelation to MessagePack bytes
pub(super) fn serialize_full_relation(relation: &FullRelation) -> Result<Vec<u8>> {
    rmp_serde::to_vec(relation)
        .map_err(|e| Error::storage(format!("Failed to serialize full relation: {}", e)))
}

/// Deserialize a RelationRef from MessagePack bytes
pub fn deserialize_relation_ref(bytes: &[u8]) -> Result<RelationRef> {
    rmp_serde::from_slice(bytes)
        .map_err(|e| Error::storage(format!("Failed to deserialize relation: {}", e)))
}

/// Deserialize a FullRelation from MessagePack bytes
pub fn deserialize_full_relation(bytes: &[u8]) -> Result<FullRelation> {
    rmp_serde::from_slice(bytes)
        .map_err(|e| Error::storage(format!("Failed to deserialize full relation: {}", e)))
}

/// Serialize a list of CompactRelations to MessagePack bytes
pub(super) fn serialize_compact_relations(relations: &[CompactRelation]) -> Result<Vec<u8>> {
    rmp_serde::to_vec(relations)
        .map_err(|e| Error::storage(format!("Failed to serialize compact relations: {}", e)))
}

/// Deserialize a list of CompactRelations from MessagePack bytes
pub(super) fn deserialize_compact_relations(bytes: &[u8]) -> Result<Vec<CompactRelation>> {
    rmp_serde::from_slice(bytes)
        .map_err(|e| Error::storage(format!("Failed to deserialize compact relations: {}", e)))
}

/// Get the RELATION_INDEX column family handle
pub fn get_relation_cf(db: &DB) -> Result<&ColumnFamily> {
    crate::cf_handle(db, crate::cf::RELATION_INDEX)
}

/// Components extracted from a forward relation key
pub(super) struct ForwardKeyComponents {
    pub relation_type: String,
    pub revision: HLC,
    pub target_id: String,
}

/// Parse components from a forward relation key
///
/// Key structure: {tenant}\0{repo}\0{branch}\0{workspace}\0rel\0{source_node_id}\0{relation_type}\0{~revision}\0{target_node_id}
///
/// NOTE: The HLC revision is a fixed 16-byte field that may contain null bytes internally.
/// We cannot use simple split(0) parsing; instead, we locate the first 7 separators,
/// then extract the next 16 bytes as the HLC, then parse the remaining bytes.
pub(super) fn parse_forward_key(key: &[u8]) -> Result<ForwardKeyComponents> {
    // Find positions of first 7 null byte separators
    let mut sep_positions = Vec::new();
    for (i, &byte) in key.iter().enumerate() {
        if byte == 0 {
            sep_positions.push(i);
            if sep_positions.len() == 7 {
                break;
            }
        }
    }

    if sep_positions.len() < 7 {
        return Err(Error::storage(format!(
            "Invalid forward key format: expected 7+ separators, got {}",
            sep_positions.len()
        )));
    }

    // Extract relation_type (between separator 5 and 6)
    let relation_type_start = sep_positions[5] + 1;
    let relation_type_end = sep_positions[6];
    let relation_type =
        String::from_utf8_lossy(&key[relation_type_start..relation_type_end]).to_string();

    // Extract HLC (next 16 bytes after separator 6)
    let hlc_start = sep_positions[6] + 1;
    let hlc_end = hlc_start + 16;

    if hlc_end > key.len() {
        return Err(Error::storage(format!(
            "Invalid forward key format: key too short for HLC (need {} bytes, got {})",
            hlc_end,
            key.len()
        )));
    }

    let revision_bytes = &key[hlc_start..hlc_end];
    let revision = decode_descending_revision(revision_bytes)
        .map_err(|e| Error::storage(format!("Failed to decode revision: {}", e)))?;

    // Extract target_id (after HLC + separator)
    let target_id_start = hlc_end + 1; // Skip the separator after HLC
    if target_id_start >= key.len() {
        return Err(Error::storage(
            "Invalid forward key format: missing target_id after HLC".to_string(),
        ));
    }

    let target_id = String::from_utf8_lossy(&key[target_id_start..]).to_string();

    Ok(ForwardKeyComponents {
        relation_type,
        revision,
        target_id,
    })
}

/// Components extracted from a reverse relation key
pub(super) struct ReverseKeyComponents {
    pub relation_type: String,
    pub revision: HLC,
    pub source_id: String,
}

/// Parse components from a reverse relation key
///
/// Key structure: {tenant}\0{repo}\0{branch}\0{workspace}\0rel_rev\0{target_node_id}\0{relation_type}\0{~revision}\0{source_node_id}
///
/// NOTE: The HLC revision is a fixed 16-byte field that may contain null bytes internally.
/// We cannot use simple split(0) parsing; instead, we locate the first 7 separators,
/// then extract the next 16 bytes as the HLC, then parse the remaining bytes.
pub(super) fn parse_reverse_key(key: &[u8]) -> Result<ReverseKeyComponents> {
    // Find positions of first 7 null byte separators
    let mut sep_positions = Vec::new();
    for (i, &byte) in key.iter().enumerate() {
        if byte == 0 {
            sep_positions.push(i);
            if sep_positions.len() == 7 {
                break;
            }
        }
    }

    if sep_positions.len() < 7 {
        return Err(Error::storage(format!(
            "Invalid reverse key format: expected 7+ separators, got {}",
            sep_positions.len()
        )));
    }

    // Extract relation_type (between separator 5 and 6)
    let relation_type_start = sep_positions[5] + 1;
    let relation_type_end = sep_positions[6];
    let relation_type =
        String::from_utf8_lossy(&key[relation_type_start..relation_type_end]).to_string();

    // Extract HLC (next 16 bytes after separator 6)
    let hlc_start = sep_positions[6] + 1;
    let hlc_end = hlc_start + 16;

    if hlc_end > key.len() {
        return Err(Error::storage(format!(
            "Invalid reverse key format: key too short for HLC (need {} bytes, got {})",
            hlc_end,
            key.len()
        )));
    }

    let revision_bytes = &key[hlc_start..hlc_end];
    let revision = decode_descending_revision(revision_bytes)
        .map_err(|e| Error::storage(format!("Failed to decode revision: {}", e)))?;

    // Extract source_id (after HLC + separator)
    let source_id_start = hlc_end + 1; // Skip the separator after HLC
    if source_id_start >= key.len() {
        return Err(Error::storage(
            "Invalid reverse key format: missing source_id after HLC".to_string(),
        ));
    }

    let source_id = String::from_utf8_lossy(&key[source_id_start..]).to_string();

    Ok(ReverseKeyComponents {
        relation_type,
        revision,
        source_id,
    })
}

/// Components extracted from a global relation key
pub(super) struct GlobalKeyComponents {
    pub relation_type: String,
    pub revision: HLC,
    pub source_workspace: String,
    pub source_id: String,
    pub target_workspace: String,
    pub target_id: String,
}

/// Parse components from a global relation key
///
/// Key structure: {tenant}\0{repo}\0{branch}\0rel_global\0{relation_type}\0{~revision}\0{source_workspace}\0{source_node_id}\0{target_workspace}\0{target_node_id}
///
/// NOTE: The HLC revision is a fixed 16-byte field that may contain null bytes internally.
/// We cannot use simple split(0) parsing; instead, we locate the first 5 separators,
/// then extract the next 16 bytes as the HLC, then parse the remaining bytes.
pub(super) fn parse_global_key(key: &[u8]) -> Result<GlobalKeyComponents> {
    // Find positions of first 5 null byte separators
    let mut sep_positions = Vec::new();
    for (i, &byte) in key.iter().enumerate() {
        if byte == 0 {
            sep_positions.push(i);
            if sep_positions.len() == 5 {
                break;
            }
        }
    }

    if sep_positions.len() < 5 {
        return Err(Error::storage(format!(
            "Invalid global key format: expected 5+ separators, got {}",
            sep_positions.len()
        )));
    }

    // Extract relation_type (between separator 3 and 4)
    let relation_type_start = sep_positions[3] + 1;
    let relation_type_end = sep_positions[4];
    let relation_type =
        String::from_utf8_lossy(&key[relation_type_start..relation_type_end]).to_string();

    // Extract HLC (next 16 bytes after separator 4)
    let hlc_start = sep_positions[4] + 1;
    let hlc_end = hlc_start + 16;

    if hlc_end > key.len() {
        return Err(Error::storage(format!(
            "Invalid global key format: key too short for HLC (need {} bytes, got {})",
            hlc_end,
            key.len()
        )));
    }

    let revision_bytes = &key[hlc_start..hlc_end];
    let revision = decode_descending_revision(revision_bytes)
        .map_err(|e| Error::storage(format!("Failed to decode revision: {}", e)))?;

    // After HLC, we need to parse: source_workspace, source_id, target_workspace, target_id
    // Find the next 3 separators after the HLC
    let remaining_start = hlc_end + 1; // Skip separator after HLC
    if remaining_start >= key.len() {
        return Err(Error::storage(
            "Invalid global key format: missing components after HLC".to_string(),
        ));
    }

    let remaining = &key[remaining_start..];
    let mut remaining_sep_positions = Vec::new();
    for (i, &byte) in remaining.iter().enumerate() {
        if byte == 0 {
            remaining_sep_positions.push(i);
            if remaining_sep_positions.len() == 3 {
                break;
            }
        }
    }

    if remaining_sep_positions.len() < 3 {
        return Err(Error::storage(format!(
            "Invalid global key format: expected 3 separators after HLC, got {}",
            remaining_sep_positions.len()
        )));
    }

    // Extract source_workspace (from start to first separator)
    let source_workspace =
        String::from_utf8_lossy(&remaining[0..remaining_sep_positions[0]]).to_string();

    // Extract source_id (between first and second separator)
    let source_id = String::from_utf8_lossy(
        &remaining[remaining_sep_positions[0] + 1..remaining_sep_positions[1]],
    )
    .to_string();

    // Extract target_workspace (between second and third separator)
    let target_workspace = String::from_utf8_lossy(
        &remaining[remaining_sep_positions[1] + 1..remaining_sep_positions[2]],
    )
    .to_string();

    // Extract target_id (after third separator to end)
    let target_id =
        String::from_utf8_lossy(&remaining[remaining_sep_positions[2] + 1..]).to_string();

    Ok(GlobalKeyComponents {
        relation_type,
        revision,
        source_workspace,
        source_id,
        target_workspace,
        target_id,
    })
}

/// Iterator wrapper that handles prefix validation, tombstone checking, and deduplication
pub(super) struct RelationIterator<'a, I>
where
    I: Iterator<Item = std::result::Result<(Box<[u8]>, Box<[u8]>), rocksdb::Error>>,
{
    iter: I,
    prefix: &'a [u8],
    max_revision: &'a HLC,
    seen_keys: HashSet<String>,
}

impl<'a, I> RelationIterator<'a, I>
where
    I: Iterator<Item = std::result::Result<(Box<[u8]>, Box<[u8]>), rocksdb::Error>>,
{
    pub fn new(iter: I, prefix: &'a [u8], max_revision: &'a HLC) -> Self {
        Self {
            iter,
            prefix,
            max_revision,
            seen_keys: HashSet::new(),
        }
    }

    /// Check if we've already seen this unique key
    pub fn is_duplicate(&mut self, unique_key: String) -> bool {
        if self.seen_keys.contains(&unique_key) {
            true
        } else {
            self.seen_keys.insert(unique_key);
            false
        }
    }

    /// Check if revision is within bounds
    pub fn is_revision_valid(&self, revision: &HLC) -> bool {
        revision <= self.max_revision
    }

    /// Check if key matches prefix
    pub fn has_prefix(&self, key: &[u8]) -> bool {
        key.starts_with(self.prefix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_tombstone() {
        assert!(is_tombstone(TOMBSTONE));
        assert!(is_tombstone(b"T"));
        assert!(!is_tombstone(b""));
        assert!(!is_tombstone(b"data"));
    }
}
