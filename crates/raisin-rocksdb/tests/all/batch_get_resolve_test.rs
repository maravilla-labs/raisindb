//! Plan Phase 4: RESOLVE reads each frontier level in ONE batched read, and a
//! level costs a bounded number of seeks per distinct target.
//!
//! Seeks are counted on the statement's read snapshot
//! (`raisin_rocksdb::read_snapshot_seeks`): the batched reads run on blocking
//! threads, out of reach of a thread-local `PerfContext`.
//!
//! An id reference costs two seeks — its newest NODES blob and its NODE_PATH
//! entry at or below the revision — so the bound is `2 · distinct + c`, with
//! `c` covering a legacy blob's tie-break lookup. The unbatched resolver
//! (`sql.batched_fetch = false`) must produce the same documents.

use crate::node_path_writer_test::{folder, head, setup, tx_put, BRANCH, REPO, TENANT, WS};
use raisin_core::services::reference_resolver::{ReferenceResolver, ResolveMemo};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::{PropertyValue, RaisinReference};
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{NodeRepository, Storage};
use serde_json::{json, Value};
use std::sync::Arc;

fn reference(id: &str) -> PropertyValue {
    PropertyValue::Reference(RaisinReference {
        id: id.to_string(),
        workspace: WS.to_string(),
        path: String::new(),
    })
}

fn ref_json(id: &str) -> Value {
    json!({ "raisin:ref": id, "raisin:workspace": WS })
}

/// Targets `t0..t{n}`; target `ti` references `t{2i+1}` and `t{2i+2}` (a
/// binary tree over the ids), so depth 2 reaches the next level.
async fn seed_targets(storage: &RocksDBStorage, n: usize) -> Result<HLC> {
    for i in 0..n {
        let mut node = folder(&format!("t{i}"), &format!("/t{i}"));
        for child in [2 * i + 1, 2 * i + 2] {
            if child < n {
                node.properties
                    .insert(format!("c{child}"), reference(&format!("t{child}")));
            }
        }
        tx_put(storage, &node).await?;
    }
    head(storage).await
}

fn resolver(
    storage: &Arc<RocksDBStorage>,
    at: HLC,
    batched: bool,
) -> (
    ReferenceResolver<RocksDBStorage>,
    Option<raisin_storage::ReadSnapshot>,
) {
    let snapshot = storage.nodes().open_read_snapshot();
    let resolver = ReferenceResolver::new(storage.clone(), TENANT, REPO, BRANCH, at)
        .with_memo(Arc::new(ResolveMemo::default()))
        .with_batched_fetch(batched)
        .with_read_snapshot(snapshot.clone());
    (resolver, snapshot)
}

#[tokio::test]
async fn resolve_seeks_bounded_by_two_per_distinct_target() -> Result<()> {
    let (storage, _dir) = setup().await?;
    let storage = Arc::new(storage);
    let at = seed_targets(&storage, 31).await?;

    // One document referencing t0..t6 at depth 2: level 1 = t0..t6, level 2 =
    // their children t7..t14 (t1..t6 are already queued) — 15 distinct.
    let doc = json!({
        "items": (0..7).map(|i| ref_json(&format!("t{i}"))).collect::<Vec<_>>(),
        "again": ref_json("t3"),
    });
    let (batched, snapshot) = resolver(&storage, at, true);
    let out = batched.resolve_json(WS, &doc, 2, None).await?;
    let seeks = raisin_rocksdb::read_snapshot_seeks(snapshot.as_ref().unwrap()).unwrap();
    let distinct = 15;
    assert!(
        seeks <= 2 * distinct + 2,
        "RESOLVE issued {seeks} seeks for {distinct} distinct targets"
    );
    assert!(seeks >= distinct, "every target was read ({seeks} seeks)");

    let (unbatched, snapshot) = resolver(&storage, at, false);
    assert_eq!(unbatched.resolve_json(WS, &doc, 2, None).await?, out);
    assert_eq!(
        raisin_rocksdb::read_snapshot_seeks(snapshot.as_ref().unwrap()),
        Some(0),
        "the unbatched path reads one `get` at a time, not through the view"
    );
    assert_eq!(out["items"][3]["id"], "t3");
    assert_eq!(out["items"][3]["c7"]["id"], "t7", "level 2 inlined");
    Ok(())
}

/// Fifty rows, each with its own target plus three shared ones, resolved as
/// one chunk: every distinct target is read once, in one batch per level.
#[tokio::test]
async fn resolve_chunk_of_rows_reads_each_target_once() -> Result<()> {
    let (storage, _dir) = setup().await?;
    let storage = Arc::new(storage);
    for i in 0..50 {
        tx_put(&storage, &folder(&format!("own{i}"), &format!("/own{i}"))).await?;
    }
    for s in ["header", "footer", "settings"] {
        tx_put(&storage, &folder(s, &format!("/{s}"))).await?;
    }
    let at = head(&storage).await?;

    let docs: Vec<Value> = (0..50)
        .map(|i| {
            json!({
                "own": ref_json(&format!("own{i}")),
                "header": ref_json("header"),
                "footer": ref_json("footer"),
                "settings": ref_json("settings"),
            })
        })
        .collect();
    let doc_refs: Vec<&Value> = docs.iter().collect();

    let memo = Arc::new(ResolveMemo::default());
    let snapshot = storage.nodes().open_read_snapshot();
    let batched = ReferenceResolver::new(storage.clone(), TENANT, REPO, BRANCH, at)
        .with_memo(memo.clone())
        .with_read_snapshot(snapshot.clone());
    let out = batched.resolve_json_many(WS, &doc_refs, 1, None).await?;
    let distinct = 53;
    assert_eq!(memo.stats().reads, distinct, "each target read once");
    let seeks = raisin_rocksdb::read_snapshot_seeks(snapshot.as_ref().unwrap()).unwrap();
    assert!(
        seeks <= 2 * distinct as u64 + 2,
        "{seeks} seeks for {distinct} targets"
    );

    // Row by row, unbatched: the same documents.
    let unbatched =
        ReferenceResolver::new(storage.clone(), TENANT, REPO, BRANCH, at).with_batched_fetch(false);
    for (doc, got) in docs.iter().zip(&out) {
        assert_eq!(&unbatched.resolve_json(WS, doc, 1, None).await?, got);
    }
    assert_eq!(out[7]["own"]["id"], "own7");
    assert_eq!(out[7]["header"]["path"], "/header");
    Ok(())
}
