//! The ONE derivation of a node's PROPERTY_INDEX entries.
//!
//! Every writer, tombstoner and rebuild asks [`entries_of`] which
//! `(name, value, tag)` triples a node version indexes, and turns each into a
//! key with [`PropertyEntry::key`]. Before this, the same list — custom
//! properties, the pseudo-properties, and the IS_A / HAS_MIXIN membership —
//! was spelled out by hand in the transaction writer, the repository writer,
//! the replication writer, the update tombstoner and the delete tombstoner, and
//! the replication copy had already drifted (it never wrote membership, so
//! `IS_A(...)` was empty on every replica).

use crate::indexing::IndexCtx;
use crate::keys;
use crate::repositories::hash_property_value;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_models::nodes::{INDEXED_MIXIN_KEY, INDEXED_SUPERTYPE_KEY};
use std::collections::BTreeSet;

/// The value segment of an entry: hashed text, or the fixed-width big-endian
/// microsecond encoding of the two timestamp pseudo-properties.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum EntryValue {
    Text(String),
    Micros(i64),
}

/// One PROPERTY_INDEX entry a node version holds, minus its revision.
///
/// The key of an entry is `(tag, name, value, revision, node)`, so two node
/// versions index "the same thing" exactly when their entries are equal here:
/// diffs are taken over these, never over `PropertyValue`s (two different
/// values can hash to one key, and must then count as unchanged).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct PropertyEntry {
    pub(crate) published: bool,
    pub(crate) name: String,
    pub(crate) value: EntryValue,
}

impl PropertyEntry {
    /// The entry's key at `revision`.
    pub(crate) fn key(&self, ctx: &IndexCtx<'_>, revision: &HLC, node_id: &str) -> Vec<u8> {
        match &self.value {
            EntryValue::Text(value) => keys::property_index_key_versioned(
                ctx.tenant_id,
                ctx.repo_id,
                ctx.branch,
                ctx.workspace,
                &self.name,
                value,
                revision,
                node_id,
                self.published,
            ),
            EntryValue::Micros(micros) => keys::property_index_key_versioned_timestamp(
                ctx.tenant_id,
                ctx.repo_id,
                ctx.branch,
                ctx.workspace,
                &self.name,
                *micros,
                revision,
                node_id,
                self.published,
            ),
        }
    }

    /// The key prefix shared by every revision of this entry's VALUE (all
    /// nodes): `{…}\0{tag}\0{name}\0{value}\0`. Revisions sort newest first
    /// beneath it, then node ids.
    pub(crate) fn value_prefix(&self, ctx: &IndexCtx<'_>) -> Vec<u8> {
        let tag = if self.published { "prop_pub" } else { "prop" };
        let builder = keys::KeyBuilder::new()
            .push(ctx.tenant_id)
            .push(ctx.repo_id)
            .push(ctx.branch)
            .push(ctx.workspace)
            .push(tag)
            .push(&self.name);
        match &self.value {
            EntryValue::Text(value) => builder.push(value).build_prefix(),
            EntryValue::Micros(micros) => builder.push_bytes(&micros.to_be_bytes()).build_prefix(),
        }
    }
}

/// Every PROPERTY_INDEX entry `node` indexes: custom properties (hashed),
/// `__node_type`, `__name`, `__archetype`, `__created_by`, `__updated_by`,
/// `__created_at` / `__updated_at` (micros), and one entry per supertype and
/// per mixin (`IS_A` / `HAS_MIXIN`).
///
/// Empty text pseudo-values are not indexed (`__node_type` excepted, which the
/// local writers always wrote).
pub(crate) fn entries_of(node: &Node) -> BTreeSet<PropertyEntry> {
    let published = node.published_at.is_some();
    let mut out = BTreeSet::new();
    let mut text = |name: &str, value: String| {
        out.insert(PropertyEntry {
            published,
            name: name.to_string(),
            value: EntryValue::Text(value),
        });
    };

    for (name, value) in &node.properties {
        text(name, hash_property_value(value));
    }
    text("__node_type", node.node_type.clone());
    for member in node.effective_supertypes() {
        text(INDEXED_SUPERTYPE_KEY, member);
    }
    for member in node.effective_mixins() {
        text(INDEXED_MIXIN_KEY, member);
    }
    for (name, value) in [
        ("__name", Some(&node.name)),
        ("__archetype", node.archetype.as_ref()),
        ("__created_by", node.created_by.as_ref()),
        ("__updated_by", node.updated_by.as_ref()),
    ] {
        if let Some(value) = value.filter(|v| !v.is_empty()) {
            text(name, value.clone());
        }
    }
    for (name, at) in [
        ("__created_at", node.created_at),
        ("__updated_at", node.updated_at),
    ] {
        if let Some(at) = at {
            out.insert(PropertyEntry {
                published,
                name: name.to_string(),
                value: EntryValue::Micros(at.timestamp_micros()),
            });
        }
    }
    out
}
