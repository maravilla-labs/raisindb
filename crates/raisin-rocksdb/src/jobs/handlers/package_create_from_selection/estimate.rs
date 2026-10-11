//! Estimate (E25) and free-space pre-flight.
//!
//! The estimate is written before any byte, so progress is determinate from
//! the first tick. Items come from the ordered-children index (key-only, no
//! node decoded); bytes are a per-node constant plus the stored size of every
//! `raisin:Asset` in the selection, found through the `__node_type` property
//! index and read one by one — assets are a small fraction of a tree, and
//! their bytes are most of an archive. The whole estimate shares one time
//! budget; when it runs out the estimate is marked `partial` and the build
//! revises it upward as it overtakes it.

use super::mirror::{Estimate, WsCount};
use super::request::SelectionRoot;
use crate::RocksDBStorage;
use raisin_binary::space::{check_room, InsufficientStorage};
use raisin_binary::StorageSpace;
use raisin_error::Result;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::{NodeRepository, PropertyIndexRepository, Storage, StorageScope};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Time budget for the whole estimate (E25: < 5 s).
pub(super) const ESTIMATE_BUDGET: Duration = Duration::from_secs(5);
/// Serialised size assumed per node (YAML of a typical content node).
pub(super) const NODE_BYTES: u64 = 2 * 1024;
/// ZIP overhead and the inventory, as a fraction of the payload.
pub(super) const ARCHIVE_MARGIN_PERCENT: u64 = 15;
/// Spill per node for the inventory sort and the central directory.
pub(super) const SPILL_BYTES_PER_ITEM: u64 = 400;

/// The asset type whose stored sizes the estimate sums.
const ASSET_TYPE: &str = "raisin:Asset";

/// Estimate one build's items and bytes.
pub(super) async fn estimate(
    storage: &Arc<RocksDBStorage>,
    tenant: &str,
    repo: &str,
    branch: &str,
    roots: &[SelectionRoot],
) -> Result<Estimate> {
    let deadline = Instant::now() + ESTIMATE_BUDGET;
    let mut est = Estimate::default();
    for (idx, root) in roots.iter().enumerate() {
        if super::request::covered_by(roots, idx) {
            continue;
        }
        let scope = StorageScope::new(tenant, repo, branch, &root.workspace);
        // Resolve the root's id (the workspace root has none: its children
        // hang off the index's root parent).
        let (root_id, root_itself) = if root.whole_workspace() {
            (crate::constants::ROOT_PARENT_ID.to_string(), 0u64)
        } else {
            match storage.nodes().get_by_path(scope, &root.path, None).await? {
                Some(n) => (n.id, 1u64),
                None => continue,
            }
        };

        // Asset ids of this workspace, for the byte sum.
        let assets: HashSet<String> = storage
            .property_index()
            .find_by_property(
                scope,
                "__node_type",
                &PropertyValue::String(ASSET_TYPE.to_string()),
                false,
                None,
            )
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();

        let mut in_subtree: Vec<String> = Vec::new();
        let (descendants, complete) = if root.recursive {
            let s = storage.clone();
            let (t, r, b, w) = (
                tenant.to_string(),
                repo.to_string(),
                branch.to_string(),
                root.workspace.clone(),
            );
            let rid = root_id.clone();
            let asset_set = assets.clone();
            let (counted, hits) = tokio::task::spawn_blocking(move || {
                let mut hits = Vec::new();
                let counted = s
                    .nodes
                    .count_subtree_keys(&t, &r, &b, &w, &rid, deadline, |id| {
                        if asset_set.contains(id) {
                            hits.push(id.to_string());
                        }
                    });
                counted.map(|c| (c, hits))
            })
            .await
            .map_err(|e| raisin_error::Error::storage(format!("estimate walk failed: {e}")))??;
            in_subtree = hits;
            counted
        } else {
            (0, true)
        };
        if root_itself == 1 && assets.contains(&root_id) {
            in_subtree.push(root_id.clone());
        }

        let mut asset_bytes = 0u64;
        for id in &in_subtree {
            if Instant::now() >= deadline {
                est.partial = true;
                break;
            }
            if let Ok(Some(node)) = storage.nodes().get(scope, id, None).await {
                asset_bytes += stored_size(&node);
            }
        }

        let items = descendants + root_itself;
        let bytes = items * NODE_BYTES + asset_bytes;
        let entry = est
            .per_ws
            .entry(root.workspace.clone())
            .or_insert(WsCount::default());
        entry.items += items;
        entry.bytes += bytes;
        est.items += items;
        est.bytes += bytes;
        if !complete {
            est.partial = true;
        }
        if Instant::now() >= deadline {
            est.partial = true;
            break;
        }
    }
    Ok(est)
}

/// The stored size of a node's embedded file, 0 if it has none.
pub(super) fn stored_size(node: &raisin_models::nodes::Node) -> u64 {
    for key in ["file", "resource"] {
        match node.properties.get(key) {
            Some(PropertyValue::Resource(r)) => {
                return r.size.unwrap_or(0).max(0) as u64;
            }
            Some(PropertyValue::Object(o)) => {
                if let Some(PropertyValue::Integer(n)) = o.get("size") {
                    return (*n).max(0) as u64;
                }
            }
            _ => {}
        }
    }
    0
}

/// Bytes the archive is expected to occupy in the blob store.
pub(super) fn archive_need(est: &Estimate) -> u64 {
    est.bytes + est.bytes / 100 * ARCHIVE_MARGIN_PERCENT
}

/// Bytes the local spill (inventory runs, central directory) needs.
pub(super) fn spill_need(est: &Estimate) -> u64 {
    est.items * SPILL_BYTES_PER_ITEM
}

/// The safety floor, unless overridden for a scratch environment.
///
/// `RAISIN_PACKAGE_BUILD_FLOOR_BYTES` replaces `max(5 GiB, 5 %)` with a fixed
/// number of bytes. It exists for test servers on small or nearly full
/// volumes; production leaves it unset.
pub(super) fn floor_override() -> Option<u64> {
    std::env::var("RAISIN_PACKAGE_BUILD_FLOOR_BYTES")
        .ok()
        .and_then(|v| v.trim().parse().ok())
}

/// [`check_room`] with the optional floor override applied.
pub(super) fn room_for(
    space: Option<StorageSpace>,
    bytes: u64,
    floor: Option<u64>,
) -> std::result::Result<(), InsufficientStorage> {
    match (space, floor) {
        (Some(space), Some(floor)) => {
            let need = bytes.saturating_add(floor);
            if space.available >= need {
                Ok(())
            } else {
                Err(InsufficientStorage {
                    need,
                    have: space.available,
                })
            }
        }
        _ => check_room(space, bytes),
    }
}

/// Free space of the volume holding `dir` (the local spill).
pub(super) fn local_space(dir: &std::path::Path) -> Option<StorageSpace> {
    raisin_binary::space::volume_space(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn the_preflight_counts_margin_and_floor() {
        let est = Estimate {
            items: 1000,
            bytes: 10 * GIB,
            ..Default::default()
        };
        assert_eq!(archive_need(&est), 10 * GIB + (10 * GIB / 100) * 15);
        assert_eq!(spill_need(&est), 400_000);

        let space = Some(StorageSpace {
            available: 20 * GIB,
            total: 100 * GIB,
        });
        // 11.5 GiB + 5 GiB floor fits in 20 GiB…
        assert!(room_for(space, archive_need(&est), None).is_ok());
        // …a 10 GiB floor does not.
        let err = room_for(space, archive_need(&est), Some(10 * GIB)).unwrap_err();
        assert_eq!(err.have, 20 * GIB);
        // Unknown space (object storage) is the quota's business.
        assert!(room_for(None, u64::MAX, Some(1)).is_ok());
    }

    #[test]
    fn stored_size_reads_resource_and_object_envelopes() {
        use raisin_models::nodes::properties::value::Resource;
        let mut node = raisin_models::nodes::Node::default();
        assert_eq!(stored_size(&node), 0);
        node.properties.insert(
            "file".into(),
            PropertyValue::Resource(Resource {
                uuid: "u".into(),
                name: None,
                size: Some(1234),
                mime_type: None,
                url: None,
                metadata: None,
                is_loaded: None,
                is_external: None,
                created_at: chrono::Utc::now().into(),
                updated_at: chrono::Utc::now().into(),
            }),
        );
        assert_eq!(stored_size(&node), 1234);
    }
}
