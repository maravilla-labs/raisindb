//! Report-only: does the HEAD property index agree with the node blobs?
//!
//! The revision-bounded reader decides each `(value, node)` pair on its own,
//! so an orphan live entry — left behind by a historically buggy writer (the
//! replica writer missing membership entries, merge apply, the old in-place
//! writer) — reads as a match. For custom properties a JSON residual filter
//! downstream masks that; pseudo-properties (`__node_type`, `__name`,
//! `__created_at`, `__updated_at`, membership) have no such residual, so an
//! orphan there is a phantom row.
//!
//! This answers "how many, and which" against a live database before anything
//! is repaired. It writes nothing and is deliberately not a job.

use super::reader::{display_value, encode_value, PropertyIndexReader};
use crate::RocksDBStorage;
use raisin_error::Result;
use raisin_models::nodes::{Node, INDEXED_MIXIN_KEY, INDEXED_SUPERTYPE_KEY};
use raisin_storage::{NodeRepository, Storage, StorageScope};

/// Why an index entry does not match its node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyIndexOrphanReason {
    /// The entry names a node that does not exist at HEAD.
    NodeMissing,
    /// The node exists, but does not carry the indexed value. `actual` is
    /// what the node would be indexed under.
    ValueMismatch { actual: Vec<String> },
    /// The entry sits under the draft tag for a published node, or the
    /// published tag for a draft one.
    TagMismatch,
}

/// One live index entry at HEAD that the node blob contradicts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyIndexOrphan {
    pub node_id: String,
    /// The indexed value, rendered as `PropertyScanEntry::property_value` is.
    pub indexed_value: String,
    pub reason: PropertyIndexOrphanReason,
}

/// Compare every live HEAD entry of `property_name` (under the draft tag, or
/// the published one when `published`) with the node it names; return at most
/// `limit` mismatches.
pub async fn detect_property_index_orphans(
    storage: &RocksDBStorage,
    scope: StorageScope<'_>,
    property_name: &str,
    published: bool,
    limit: usize,
) -> Result<Vec<PropertyIndexOrphan>> {
    let db = storage.db();
    let reader = PropertyIndexReader::new(
        db,
        scope.tenant_id,
        scope.repo_id,
        scope.branch,
        scope.workspace,
        property_name,
        published,
        None,
    )?;
    let mut pairs: Vec<(Vec<u8>, String)> = Vec::new();
    reader.for_each_live_pair(|value, node_id| {
        pairs.push((value.to_vec(), node_id.to_string()));
        Ok(true)
    })?;

    let mut orphans = Vec::new();
    for (value, node_id) in pairs {
        if orphans.len() >= limit {
            break;
        }
        let reason = match storage.nodes().get(scope, &node_id, None).await? {
            None => Some(PropertyIndexOrphanReason::NodeMissing),
            Some(node) if node.published_at.is_some() != published => {
                Some(PropertyIndexOrphanReason::TagMismatch)
            }
            Some(node) => {
                let expected = indexed_values(&node, property_name);
                (!expected.contains(&value)).then(|| PropertyIndexOrphanReason::ValueMismatch {
                    actual: expected
                        .iter()
                        .map(|v| display_value(property_name, v))
                        .collect(),
                })
            }
        };
        if let Some(reason) = reason {
            orphans.push(PropertyIndexOrphan {
                node_id,
                indexed_value: display_value(property_name, &value),
                reason,
            });
        }
    }
    Ok(orphans)
}

/// Every value the writers index `node` under for `property_name`, in index
/// key encoding. Mirrors `add_system_property_indexes` for pseudo-properties.
fn indexed_values(node: &Node, property_name: &str) -> Vec<Vec<u8>> {
    fn text(s: &str) -> Vec<u8> {
        s.as_bytes().to_vec()
    }
    fn micros(t: &chrono::DateTime<chrono::Utc>) -> Vec<u8> {
        t.timestamp_micros().to_be_bytes().to_vec()
    }
    fn texts<'a>(values: impl IntoIterator<Item = &'a String>) -> Vec<Vec<u8>> {
        values.into_iter().map(|v| text(v)).collect()
    }
    match property_name {
        "__node_type" => vec![text(&node.node_type)],
        "__name" => vec![text(&node.name)],
        "__archetype" => texts(&node.archetype),
        "__created_by" => texts(&node.created_by),
        "__updated_by" => texts(&node.updated_by),
        "__created_at" => node.created_at.iter().map(micros).collect(),
        "__updated_at" => node.updated_at.iter().map(micros).collect(),
        INDEXED_SUPERTYPE_KEY => texts(&node.effective_supertypes()),
        INDEXED_MIXIN_KEY => texts(&node.effective_mixins()),
        _ => node
            .properties
            .get(property_name)
            .map(|value| encode_value(property_name, value))
            .into_iter()
            .collect(),
    }
}
