//! Property index operations (custom and system properties)

use super::super::super::helpers::{hash_property_value, TOMBSTONE};
use super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::WriteBatch;

/// Write property-index TOMBSTONES for every value-bearing entry of `old_node`
/// that `new_node` no longer carries (value changed, property removed, or the
/// published tag flipped so entries moved between `prop`/`prop_pub`).
///
/// Without these, an UPDATE leaves the OLD-value index entry live forever:
/// equality scans and index-backed COUNTs on the old value keep matching the
/// node, and the orphaned entries survive restarts. Tombstoning at the NEW
/// revision preserves MVCC history (time-travel reads below the new revision
/// still see the old entry), mirroring the compound/unique/spatial index
/// maintenance on both write paths.
///
/// Unchanged (name, value, tag) triples are intentionally skipped — writing a
/// tombstone for an unchanged value at the same revision would collide with
/// the fresh live entry (identical key) and hide the node.
#[allow(clippy::too_many_arguments)]
pub(crate) fn add_stale_property_tombstones(
    batch: &mut WriteBatch,
    cf_property: &rocksdb::ColumnFamily,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    old_node: &Node,
    new_node: &Node,
    revision: &HLC,
) {
    let was_published = old_node.published_at.is_some();
    let is_published = new_node.published_at.is_some();
    let tag_changed = was_published != is_published;

    // Custom properties: tombstone entries whose exact (name, value) pair is
    // gone (or whose tag changed).
    for (prop_name, old_value) in &old_node.properties {
        let unchanged = !tag_changed && new_node.properties.get(prop_name) == Some(old_value);
        if unchanged {
            continue;
        }
        let value_hash = hash_property_value(old_value);
        let prop_key = keys::property_index_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            prop_name,
            &value_hash,
            revision,
            &old_node.id,
            was_published,
        );
        batch.put_cf(cf_property, prop_key, TOMBSTONE);
    }

    // Pseudo-properties that support equality lookups. Timestamps
    // (__created_at/__updated_at) are range-scanned per revision and are left
    // to MVCC revision filtering.
    let mut tombstone_pseudo = |name: &str, old_value: &str| {
        let key = keys::property_index_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            name,
            old_value,
            revision,
            &old_node.id,
            was_published,
        );
        batch.put_cf(cf_property, key, TOMBSTONE);
    };

    if tag_changed || old_node.node_type != new_node.node_type {
        tombstone_pseudo("__node_type", &old_node.node_type);
    }
    if !old_node.name.is_empty() && (tag_changed || old_node.name != new_node.name) {
        tombstone_pseudo("__name", &old_node.name);
    }
    if let Some(old_archetype) = old_node.archetype.as_deref() {
        if !old_archetype.is_empty()
            && (tag_changed || new_node.archetype.as_deref() != Some(old_archetype))
        {
            tombstone_pseudo("__archetype", old_archetype);
        }
    }
    if let Some(old_created_by) = old_node.created_by.as_deref() {
        if !old_created_by.is_empty()
            && (tag_changed || new_node.created_by.as_deref() != Some(old_created_by))
        {
            tombstone_pseudo("__created_by", old_created_by);
        }
    }
    if let Some(old_updated_by) = old_node.updated_by.as_deref() {
        if !old_updated_by.is_empty()
            && (tag_changed || new_node.updated_by.as_deref() != Some(old_updated_by))
        {
            tombstone_pseudo("__updated_by", old_updated_by);
        }
    }

    // Timestamp pseudo-properties use the timestamp key encoding.
    let mut tombstone_timestamp = |name: &str, micros: i64| {
        let key = keys::property_index_key_versioned_timestamp(
            tenant_id,
            repo_id,
            branch,
            workspace,
            name,
            micros,
            revision,
            &old_node.id,
            was_published,
        );
        batch.put_cf(cf_property, key, TOMBSTONE);
    };
    if let Some(old_created_at) = old_node.created_at {
        if tag_changed || new_node.created_at != Some(old_created_at) {
            tombstone_timestamp("__created_at", old_created_at.timestamp_micros());
        }
    }
    if let Some(old_updated_at) = old_node.updated_at {
        if tag_changed || new_node.updated_at != Some(old_updated_at) {
            tombstone_timestamp("__updated_at", old_updated_at.timestamp_micros());
        }
    }
}

impl NodeRepositoryImpl {
    /// Add property indexes for all node properties
    pub(in crate::repositories) fn add_property_indexes(
        &self,
        batch: &mut WriteBatch,
        node: &Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        revision: &HLC,
    ) -> Result<()> {
        let cf_property = cf_handle(&self.db, cf::PROPERTY_INDEX)?;
        let is_published = node.published_at.is_some();

        for (prop_name, prop_value) in &node.properties {
            let value_hash = hash_property_value(prop_value);
            let prop_key = keys::property_index_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                prop_name,
                &value_hash,
                revision,
                &node.id,
                is_published,
            );
            batch.put_cf(cf_property, prop_key, node.id.as_bytes());
        }

        Ok(())
    }

    /// Add system property indexes (__name, __node_type, __archetype, etc.)
    pub(in crate::repositories) fn add_system_property_indexes(
        &self,
        batch: &mut WriteBatch,
        node: &Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        revision: &HLC,
    ) -> Result<()> {
        let cf_property = cf_handle(&self.db, cf::PROPERTY_INDEX)?;
        let is_published = node.published_at.is_some();

        // Index node_type as a pseudo-property for efficient list_by_type queries
        let node_type_key = keys::property_index_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            "__node_type",
            &node.node_type,
            revision,
            &node.id,
            is_published,
        );
        batch.put_cf(cf_property, node_type_key, node.id.as_bytes());

        // Index: name
        if !node.name.is_empty() {
            let name_key = keys::property_index_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                "__name",
                &node.name,
                revision,
                &node.id,
                is_published,
            );
            batch.put_cf(cf_property, name_key, node.id.as_bytes());
        }

        // Index: archetype
        if let Some(ref archetype) = node.archetype {
            if !archetype.is_empty() {
                let archetype_key = keys::property_index_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    "__archetype",
                    archetype,
                    revision,
                    &node.id,
                    is_published,
                );
                batch.put_cf(cf_property, archetype_key, node.id.as_bytes());
            }
        }

        // Index: created_by
        if let Some(ref created_by) = node.created_by {
            if !created_by.is_empty() {
                let created_by_key = keys::property_index_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    "__created_by",
                    created_by,
                    revision,
                    &node.id,
                    is_published,
                );
                batch.put_cf(cf_property, created_by_key, node.id.as_bytes());
            }
        }

        // Index: updated_by
        if let Some(ref updated_by) = node.updated_by {
            if !updated_by.is_empty() {
                let updated_by_key = keys::property_index_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    "__updated_by",
                    updated_by,
                    revision,
                    &node.id,
                    is_published,
                );
                batch.put_cf(cf_property, updated_by_key, node.id.as_bytes());
            }
        }

        // Index: created_at (as i64 microseconds for efficient range queries)
        if let Some(created_at) = node.created_at {
            let created_at_key = keys::property_index_key_versioned_timestamp(
                tenant_id,
                repo_id,
                branch,
                workspace,
                "__created_at",
                created_at.timestamp_micros(),
                revision,
                &node.id,
                is_published,
            );
            batch.put_cf(cf_property, created_at_key, node.id.as_bytes());
        }

        // Index: updated_at (as i64 microseconds for efficient range queries)
        if let Some(updated_at) = node.updated_at {
            let updated_at_key = keys::property_index_key_versioned_timestamp(
                tenant_id,
                repo_id,
                branch,
                workspace,
                "__updated_at",
                updated_at.timestamp_micros(),
                revision,
                &node.id,
                is_published,
            );
            batch.put_cf(cf_property, updated_at_key, node.id.as_bytes());
        }

        Ok(())
    }
}
