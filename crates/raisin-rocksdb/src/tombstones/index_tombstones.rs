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
            &reference.id,
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

        // Parse relation details to write the reverse tombstone. The forward
        // VALUE is a serialized RelationRef carrying the target's workspace —
        // the key alone doesn't contain it (the old key-parse helper returned
        // an empty workspace, so reverse tombstones never matched the real
        // reverse keys and incoming edges survived deletion).
        let parsed = crate::repositories::deserialize_relation_ref(&value)
            .ok()
            .map(|r| (r.relation_type, r.workspace, r.target))
            .or_else(|| parse_relation_from_forward_key(&key, &relation_prefix));
        if let Some((relation_type, target_workspace, target_id)) = parsed {
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

    // ALSO clear the PACKED adjacency list (new format). Reads prefer the
    // packed list over the legacy per-edge keys, so tombstoning only the
    // legacy keys above left deleted nodes with dangling graph edges
    // (NEIGHBORS / GRAPH_TABLE kept returning them). Write an empty list at
    // the delete revision; older revisions stay intact for MVCC reads.
    let empty = crate::repositories::serialize_compact_relations(&[])?;
    let adj_key = keys::node_adjacency_key_versioned(
        ctx.tenant_id,
        ctx.repo_id,
        ctx.branch,
        ctx.workspace,
        &node.id,
        revision,
    );
    batch.put_cf(cfs.relation_index, adj_key, empty);

    // Incoming edges: rewrite each source node's packed list without edges
    // targeting the deleted node. (Repository deletes normally refuse nodes
    // with incoming relations, but cascade/transaction deletes can remove
    // whole trees with intra-tree edges, and the safety check can be bypassed.)
    let reverse_prefix = keys::relation_reverse_prefix(
        ctx.tenant_id,
        ctx.repo_id,
        ctx.branch,
        ctx.workspace,
        &node.id,
    );
    let mut seen_sources = std::collections::HashSet::new();
    let iter = db.prefix_iterator_cf(cfs.relation_index, &reverse_prefix);
    for item in iter {
        let (key, value) = item.map_err(|e| {
            raisin_error::Error::storage(format!("Failed to iterate reverse relations: {}", e))
        })?;
        if !key.starts_with(&reverse_prefix) {
            break;
        }
        if value.as_ref() == TOMBSTONE {
            continue;
        }
        // Key format: {tenant}\0{repo}\0{branch}\0{src_workspace}\0rel_rev\0{target}\0{type}\0{~rev}\0{source_id}
        let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();
        if parts.len() >= 9 {
            let source_workspace = String::from_utf8_lossy(parts[3]).to_string();
            let source_node_id = String::from_utf8_lossy(parts[8]).to_string();
            if !seen_sources.insert((source_workspace.clone(), source_node_id.clone())) {
                continue;
            }
            if let Some(mut packed) = crate::repositories::read_packed_relations_at(
                db,
                revision,
                ctx.tenant_id,
                ctx.repo_id,
                ctx.branch,
                &source_workspace,
                &source_node_id,
            )? {
                let before = packed.len();
                packed.retain(|r| !(r.target_id == node.id && r.target_workspace == ctx.workspace));
                if packed.len() != before {
                    let bytes = crate::repositories::serialize_compact_relations(&packed)?;
                    let src_key = keys::node_adjacency_key_versioned(
                        ctx.tenant_id,
                        ctx.repo_id,
                        ctx.branch,
                        &source_workspace,
                        &source_node_id,
                        revision,
                    );
                    batch.put_cf(cfs.relation_index, src_key, bytes);
                }
            }
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
