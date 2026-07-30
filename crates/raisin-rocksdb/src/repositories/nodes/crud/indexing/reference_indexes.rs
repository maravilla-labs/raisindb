//! Reference index operations (forward and reverse) — SINGLE SOURCE OF TRUTH.
//!
//! Every write path (repository add/update, transaction add/put, replication
//! apply) MUST use `walk_references` + `add_reference_index_entries` /
//! `add_stale_reference_tombstones` so that:
//! - nested references (arrays, objects, element content) are indexed with ONE
//!   property-path format (`tags.0`, `content.hero.image`) — a format mismatch
//!   between writer and tombstoner leaves entries that can never be shadowed;
//! - removing/changing a reference on update tombstones the old entry —
//!   otherwise `REFERENCES()` keeps matching the node forever.

use super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::{PropertyValue, RaisinReference};
use raisin_models::nodes::Node;
use rocksdb::WriteBatch;
use std::collections::HashMap;

/// Tombstone marker shared with the other index families.
const TOMBSTONE: &[u8] = b"T";

/// Walk a property tree and collect every `PropertyValue::Reference` with its
/// dot-format property path (`tags.0`, `content.cta.target`, ...).
///
/// Forward/reverse index keys embed the path this produces, so writers AND
/// tombstoners must both use it for keys to line up.
///
/// The traversal itself lives in [`crate::indexing::walk_properties`], shared with
/// the spatial index's [`crate::indexing::walk_geometries`]. Two hand-maintained
/// copies of the same recursion is how the two path formats drift, and a writer
/// and a tombstoner that disagree about the format leave entries that can never be
/// shadowed.
pub(crate) fn walk_references(
    properties: &HashMap<String, PropertyValue>,
) -> Vec<(String, RaisinReference)> {
    crate::indexing::walk_properties(properties, |value| match value {
        PropertyValue::Reference(r) => Some(r),
        _ => None,
    })
    .into_iter()
    .map(|(path, reference)| (path, reference.clone()))
    .collect()
}

/// Write forward + reverse reference index entries for every reference in
/// `node.properties` at `revision`.
///
/// Forward values are msgpack-encoded `RaisinReference` (the shape
/// `get_node_references` decodes); reverse values carry the source node id.
#[allow(clippy::too_many_arguments)]
pub(crate) fn add_reference_index_entries(
    batch: &mut WriteBatch,
    cf_reference: &rocksdb::ColumnFamily,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node: &Node,
    revision: &HLC,
) -> Result<()> {
    let is_published = node.published_at.is_some();

    for (property_path, reference) in walk_references(&node.properties) {
        let forward_key = keys::reference_forward_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &node.id,
            &property_path,
            revision,
            is_published,
        );
        let ref_value = rmp_serde::to_vec(&reference)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;
        batch.put_cf(cf_reference, forward_key, ref_value);

        // Reverse index keyed by the target's stable node id — survives
        // target moves (path is mutable).
        let reverse_key = keys::reference_reverse_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &reference.workspace,
            &reference.id,
            &node.id,
            &property_path,
            revision,
            is_published,
        );
        batch.put_cf(cf_reference, reverse_key, node.id.as_bytes());
    }

    Ok(())
}

/// Tombstone every reference entry of `old_node` that `new_node` no longer
/// carries (target changed at that path, path removed, or value replaced).
///
/// Written at the NEW revision, for BOTH published variants (the publish flag
/// may have flipped between versions). Must run in the same batch as — and is
/// safe in any order relative to — `add_reference_index_entries`: keys are
/// only tombstoned when the (path, target) pair is absent from the new set.
#[allow(clippy::too_many_arguments)]
pub(crate) fn add_stale_reference_tombstones(
    batch: &mut WriteBatch,
    cf_reference: &rocksdb::ColumnFamily,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    old_node: &Node,
    new_node: &Node,
    revision: &HLC,
) {
    let new_refs: std::collections::HashSet<(String, String, String)> =
        walk_references(&new_node.properties)
            .into_iter()
            .map(|(path, r)| (path, r.workspace, r.id))
            .collect();

    for (property_path, reference) in walk_references(&old_node.properties) {
        if new_refs.contains(&(
            property_path.clone(),
            reference.workspace.clone(),
            reference.id.clone(),
        )) {
            continue;
        }

        for is_published in [false, true] {
            let forward_key = keys::reference_forward_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &old_node.id,
                &property_path,
                revision,
                is_published,
            );
            batch.put_cf(cf_reference, forward_key, TOMBSTONE);

            let reverse_key = keys::reference_reverse_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &reference.workspace,
                &reference.id,
                &old_node.id,
                &property_path,
                revision,
                is_published,
            );
            batch.put_cf(cf_reference, reverse_key, TOMBSTONE);
        }
    }
}

impl NodeRepositoryImpl {
    /// Add reference indexes (forward and reverse) for all references in the
    /// node's property tree (nested arrays/objects/element content included).
    pub(in crate::repositories) fn add_reference_indexes(
        &self,
        batch: &mut WriteBatch,
        node: &Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        revision: &HLC,
    ) -> Result<()> {
        let cf_reference = cf_handle(&self.db, cf::REFERENCE_INDEX)?;
        add_reference_index_entries(
            batch,
            cf_reference,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node,
            revision,
        )
    }

    /// Tombstone stale reference entries on update (see
    /// `add_stale_reference_tombstones`).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::repositories) fn add_stale_reference_tombstones_to_batch(
        &self,
        batch: &mut WriteBatch,
        old_node: &Node,
        new_node: &Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        revision: &HLC,
    ) -> Result<()> {
        let cf_reference = cf_handle(&self.db, cf::REFERENCE_INDEX)?;
        add_stale_reference_tombstones(
            batch,
            cf_reference,
            tenant_id,
            repo_id,
            branch,
            workspace,
            old_node,
            new_node,
            revision,
        );
        Ok(())
    }
}
