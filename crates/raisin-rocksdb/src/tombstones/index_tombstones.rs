//! Index tombstone functions: property, reference, relation, compound, spatial, translation

use super::helpers::{
    extract_locale_from_translation_key, extract_node_id_from_key, extract_references,
    hash_property_value, parse_relation_from_forward_key,
};
use super::{TombstoneColumnFamilies, TombstoneContext, TOMBSTONE};
use crate::keys;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::{WriteBatch, DB};

/// Tombstone all property indexes (PROPERTY_INDEX CF)
///
/// Includes both custom properties and system properties:
/// - Custom properties from node.properties
/// - __node_type
/// - __name
/// - __archetype
/// - __created_by
/// - __updated_by
pub(super) fn tombstone_property_indexes(
    batch: &mut WriteBatch,
    ctx: &TombstoneContext,
    cfs: &TombstoneColumnFamilies,
    node: &Node,
    revision: &HLC,
    is_published: bool,
) {
    // Tombstone custom property indexes
    for (prop_name, prop_value) in &node.properties {
        let value_hash = hash_property_value(prop_value);
        let prop_key = keys::property_index_key_versioned(
            ctx.tenant_id,
            ctx.repo_id,
            ctx.branch,
            ctx.workspace,
            prop_name,
            &value_hash,
            revision,
            &node.id,
            is_published,
        );
        batch.put_cf(cfs.property_index, prop_key, TOMBSTONE);
    }

    // Tombstone __node_type index (always present)
    let node_type_key = keys::property_index_key_versioned(
        ctx.tenant_id,
        ctx.repo_id,
        ctx.branch,
        ctx.workspace,
        "__node_type",
        &node.node_type,
        revision,
        &node.id,
        is_published,
    );
    batch.put_cf(cfs.property_index, node_type_key, TOMBSTONE);

    // Tombstone __name index (if present)
    if !node.name.is_empty() {
        let name_key = keys::property_index_key_versioned(
            ctx.tenant_id,
            ctx.repo_id,
            ctx.branch,
            ctx.workspace,
            "__name",
            &node.name,
            revision,
            &node.id,
            is_published,
        );
        batch.put_cf(cfs.property_index, name_key, TOMBSTONE);
    }

    // Tombstone __archetype index (if present)
    if let Some(ref archetype) = node.archetype {
        if !archetype.is_empty() {
            let archetype_key = keys::property_index_key_versioned(
                ctx.tenant_id,
                ctx.repo_id,
                ctx.branch,
                ctx.workspace,
                "__archetype",
                archetype,
                revision,
                &node.id,
                is_published,
            );
            batch.put_cf(cfs.property_index, archetype_key, TOMBSTONE);
        }
    }

    // Tombstone __created_by index (if present)
    if let Some(ref created_by) = node.created_by {
        if !created_by.is_empty() {
            let created_by_key = keys::property_index_key_versioned(
                ctx.tenant_id,
                ctx.repo_id,
                ctx.branch,
                ctx.workspace,
                "__created_by",
                created_by,
                revision,
                &node.id,
                is_published,
            );
            batch.put_cf(cfs.property_index, created_by_key, TOMBSTONE);
        }
    }

    // Tombstone __updated_by index (if present)
    if let Some(ref updated_by) = node.updated_by {
        if !updated_by.is_empty() {
            let updated_by_key = keys::property_index_key_versioned(
                ctx.tenant_id,
                ctx.repo_id,
                ctx.branch,
                ctx.workspace,
                "__updated_by",
                updated_by,
                revision,
                &node.id,
                is_published,
            );
            batch.put_cf(cfs.property_index, updated_by_key, TOMBSTONE);
        }
    }
}

/// Tombstone reference indexes (REFERENCE_INDEX CF)
///
/// Extracts references from node properties and tombstones both forward and reverse indexes.
pub(super) fn tombstone_reference_indexes(
    batch: &mut WriteBatch,
    ctx: &TombstoneContext,
    cfs: &TombstoneColumnFamilies,
    node: &Node,
    revision: &HLC,
    is_published: bool,
) {
    let refs = extract_references(&node.properties);
    for (property_path, reference) in refs {
        // Tombstone forward index
        let forward_key = keys::reference_forward_key_versioned(
            ctx.tenant_id,
            ctx.repo_id,
            ctx.branch,
            ctx.workspace,
            &node.id,
            &property_path,
            revision,
            is_published,
        );
        batch.put_cf(cfs.reference_index, forward_key, TOMBSTONE);

        // Tombstone reverse index
        let reverse_key = keys::reference_reverse_key_versioned(
            ctx.tenant_id,
            ctx.repo_id,
            ctx.branch,
            ctx.workspace,
            &reference.workspace,
            &reference.path,
            &node.id,
            &property_path,
            revision,
            is_published,
        );
        batch.put_cf(cfs.reference_index, reverse_key, TOMBSTONE);
    }
}

/// Tombstone relation indexes (RELATION_INDEX CF)
///
/// NOTE: We must scan RELATION_INDEX to find actual relations, not use node.relations
/// which is always empty on read! This is critical for proper relation cleanup.
pub(super) fn tombstone_relation_indexes(
    batch: &mut WriteBatch,
    db: &DB,
    ctx: &TombstoneContext,
    cfs: &TombstoneColumnFamilies,
    node: &Node,
    revision: &HLC,
) -> Result<()> {
    // Scan for outgoing relations from this node
    let relation_prefix = keys::relation_forward_prefix(
        ctx.tenant_id,
        ctx.repo_id,
        ctx.branch,
        ctx.workspace,
        &node.id,
    );

    let iter = db.prefix_iterator_cf(cfs.relation_index, &relation_prefix);
    for item in iter {
        let (key, value) = item.map_err(|e| {
            raisin_error::Error::storage(format!("Failed to iterate relations: {}", e))
        })?;

        // Stop when leaving prefix
        if !key.starts_with(&relation_prefix) {
            break;
        }

        // Skip already-tombstoned entries
        if value.as_ref() == TOMBSTONE {
            continue;
        }

        // Parse relation details from key to write reverse tombstone
        if let Some((relation_type, target_workspace, target_id)) =
            parse_relation_from_forward_key(&key, &relation_prefix)
        {
            // Tombstone forward relation
            let fwd_key = keys::relation_forward_key_versioned(
                ctx.tenant_id,
                ctx.repo_id,
                ctx.branch,
                ctx.workspace,
                &node.id,
                &relation_type,
                revision,
                &target_id,
            );
            batch.put_cf(cfs.relation_index, fwd_key, TOMBSTONE);

            // Tombstone reverse relation
            let rev_key = keys::relation_reverse_key_versioned(
                ctx.tenant_id,
                ctx.repo_id,
                ctx.branch,
                &target_workspace,
                &target_id,
                &relation_type,
                revision,
                &node.id,
            );
            batch.put_cf(cfs.relation_index, rev_key, TOMBSTONE);
        }
    }

    Ok(())
}

/// Tombstone compound indexes (COMPOUND_INDEX CF)
///
/// Scans workspace prefix to find all compound index entries for this node.
/// Handles both draft and published compound indexes.
pub(super) fn tombstone_compound_indexes(
    batch: &mut WriteBatch,
    db: &DB,
    ctx: &TombstoneContext,
    cfs: &TombstoneColumnFamilies,
    node: &Node,
) -> Result<()> {
    // Scan compound indexes for both draft and published
    for is_published in [false, true] {
        let prefix = keys::compound_index_workspace_prefix(
            ctx.tenant_id,
            ctx.repo_id,
            ctx.branch,
            ctx.workspace,
            is_published,
        );

        let iter = db.prefix_iterator_cf(cfs.compound_index, &prefix);
        for item in iter {
            let (key, _) = item.map_err(|e| {
                raisin_error::Error::storage(format!("Failed to iterate compound index: {}", e))
            })?;

            // Stop when leaving workspace prefix
            if !key.starts_with(&prefix) {
                break;
            }

            // Check if this key is for our node (node_id is the last component)
            if let Some(key_node_id) = extract_node_id_from_key(&key) {
                if key_node_id == node.id {
                    // Write tombstone for this exact key
                    batch.put_cf(cfs.compound_index, key, TOMBSTONE);
                }
            }
        }
    }

    Ok(())
}

/// Tombstone spatial indexes (SPATIAL_INDEX CF)
///
/// Scans workspace spatial prefix to find all spatial index entries for this node.
pub(super) fn tombstone_spatial_indexes(
    batch: &mut WriteBatch,
    db: &DB,
    ctx: &TombstoneContext,
    cfs: &TombstoneColumnFamilies,
    node: &Node,
) -> Result<()> {
    let prefix =
        keys::spatial_index_workspace_prefix(ctx.tenant_id, ctx.repo_id, ctx.branch, ctx.workspace);

    let iter = db.prefix_iterator_cf(cfs.spatial_index, &prefix);
    for item in iter {
        let (key, _) = item.map_err(|e| {
            raisin_error::Error::storage(format!("Failed to iterate spatial index: {}", e))
        })?;

        // Stop when leaving workspace prefix
        if !key.starts_with(&prefix) {
            break;
        }

        // Check if this key is for our node (node_id is the last component)
        if let Some(key_node_id) = extract_node_id_from_key(&key) {
            if key_node_id == node.id {
                // Write tombstone for this exact key
                batch.put_cf(cfs.spatial_index, key, TOMBSTONE);
            }
        }
    }

    Ok(())
}

/// Tombstone translation data (TRANSLATION_DATA CF)
///
/// Scans for all translation locales for this node and tombstones them.
pub(super) fn tombstone_translation_data(
    batch: &mut WriteBatch,
    db: &DB,
    ctx: &TombstoneContext,
    cfs: &TombstoneColumnFamilies,
    node: &Node,
    revision: &HLC,
) -> Result<()> {
    // Build prefix for translations of this node
    // Key format: {tenant}\0{repo}\0{branch}\0{workspace}\0translations\0{node_id}\0
    let translation_prefix = format!(
        "{}\0{}\0{}\0{}\0translations\0{}\0",
        ctx.tenant_id, ctx.repo_id, ctx.branch, ctx.workspace, node.id
    )
    .into_bytes();

    let iter = db.prefix_iterator_cf(cfs.translation_data, &translation_prefix);
    for item in iter {
        let (key, value) = item.map_err(|e| {
            raisin_error::Error::storage(format!("Failed to iterate translations: {}", e))
        })?;

        // Stop when leaving node's translation prefix
        if !key.starts_with(&translation_prefix) {
            break;
        }

        // Skip already-tombstoned entries
        if value.as_ref() == TOMBSTONE {
            continue;
        }

        // Extract locale from key and write tombstone at new revision
        if let Some(locale) = extract_locale_from_translation_key(&key, &translation_prefix) {
            // Build tombstone key with new revision
            let mut tombstone_key = format!(
                "{}\0{}\0{}\0{}\0translations\0{}\0{}\0",
                ctx.tenant_id, ctx.repo_id, ctx.branch, ctx.workspace, node.id, locale
            )
            .into_bytes();
            tombstone_key.extend_from_slice(&keys::encode_descending_revision(revision));
            batch.put_cf(cfs.translation_data, tombstone_key, TOMBSTONE);
        }
    }

    Ok(())
}
