//! Reference-index retarget-on-move tests.
//!
//! These open a bare `RocksDBStorage` and drive the reference index directly —
//! no workspace root node or node-type bootstrap — so they exercise the real
//! versioned RocksDB keys used by the async `RetargetReferences` job.

use raisin_error::Result;
use raisin_models::nodes::properties::{PropertyValue, RaisinReference};
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::scope::StorageScope;
use raisin_storage::{ReferenceIndexRepository, Storage};
use std::collections::HashMap;

const TENANT: &str = "t";
const REPO: &str = "r";
const BRANCH: &str = "main";
const WS: &str = "default";

fn open() -> Result<(RocksDBStorage, tempfile::TempDir)> {
    let dir = tempfile::tempdir().map_err(|e| raisin_error::Error::Backend(e.to_string()))?;
    let storage = RocksDBStorage::new(dir.path())?;
    Ok((storage, dir))
}

fn reference(id: &str, ws: &str, path: &str) -> RaisinReference {
    RaisinReference {
        id: id.to_string(),
        workspace: ws.to_string(),
        path: path.to_string(),
    }
}

/// Retarget on the real versioned keys: index at rev1, retarget at rev2 > rev1,
/// and confirm the read returns ONLY the new path (latest-wins, no stale leak),
/// with the reverse index (keyed by the stable target id) unchanged.
#[tokio::test]
async fn retarget_latest_wins() -> Result<()> {
    let (storage, _dir) = open()?;
    let ref_index = storage.reference_index();
    let scope = StorageScope::new(TENANT, REPO, BRANCH, WS);

    let mut props = HashMap::new();
    props.insert(
        "hero".to_string(),
        PropertyValue::Reference(reference("target1", WS, "/assets/hero.png")),
    );
    ref_index
        .index_references(scope, "node1", &props, &raisin_hlc::HLC::new(1, 0), false)
        .await?;

    let refs = ref_index.get_node_references(scope, "node1", false).await?;
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].0, "hero");
    assert_eq!(refs[0].1.path, "/assets/hero.png");

    // Target moved -> retarget at a strictly newer revision.
    ref_index
        .retarget_forward_path(
            scope,
            "node1",
            "hero",
            &reference("target1", WS, "/assets/moved/hero.png"),
            &raisin_hlc::HLC::new(2, 0),
            false,
        )
        .await?;

    // Latest-wins: exactly one live entry, the new path; no stale rev1 leak.
    let refs = ref_index.get_node_references(scope, "node1", false).await?;
    assert_eq!(refs.len(), 1, "one live entry per property path");
    assert_eq!(refs[0].1.path, "/assets/moved/hero.png");
    assert_eq!(refs[0].1.id, "target1");

    // Reverse index is keyed by the stable target id — unaffected by the move.
    let referrers = ref_index
        .find_referencing_nodes(scope, WS, "target1", false)
        .await?;
    assert_eq!(referrers, vec![("node1".to_string(), "hero".to_string())]);

    Ok(())
}

/// A referrer in a DIFFERENT workspace than the target is still found and
/// retargeted — this is why the job scans all workspaces.
#[tokio::test]
async fn retarget_cross_workspace_referrer() -> Result<()> {
    let (storage, _dir) = open()?;
    let ref_index = storage.reference_index();
    let target_ws = "assets";
    let referrer_scope = StorageScope::new(TENANT, REPO, BRANCH, "content");

    let mut props = HashMap::new();
    props.insert(
        "image".to_string(),
        PropertyValue::Reference(reference("asset1", target_ws, "/a/old.png")),
    );
    ref_index
        .index_references(
            referrer_scope,
            "page1",
            &props,
            &raisin_hlc::HLC::new(1, 0),
            false,
        )
        .await?;

    // Find referrers of the target (in the "assets" ws) from the referrer's ws.
    let referrers = ref_index
        .find_referencing_nodes(referrer_scope, target_ws, "asset1", false)
        .await?;
    assert_eq!(referrers, vec![("page1".to_string(), "image".to_string())]);

    ref_index
        .retarget_forward_path(
            referrer_scope,
            "page1",
            "image",
            &reference("asset1", target_ws, "/a/new.png"),
            &raisin_hlc::HLC::new(2, 0),
            false,
        )
        .await?;

    let refs = ref_index
        .get_node_references(referrer_scope, "page1", false)
        .await?;
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].1.path, "/a/new.png");
    assert_eq!(refs[0].1.workspace, target_ws);

    Ok(())
}
