//! Tests for HNSW index transfer.
//!
//! These are round trips through the REAL engine, not fixtures of dummy bytes.
//! The bug this module shipped for its whole life — the `.hnsw.meta` sidecar
//! never crossing the wire, so a transferred index destroyed itself on load and
//! took the receiver's own copy with it — was invisible to a test that wrote
//! `b"test index data"` and asserted the same bytes came out the other end. A
//! transfer test has to end by OPENING the result.

use super::*;
use raisin_hnsw::{HnswIndexingEngine, PartitionId};
use std::sync::Arc;
use tempfile::TempDir;

fn partition() -> PartitionId {
    PartitionId::new("testhashxxxT")
}

/// Build a real, saved, non-empty index under `base`.
fn seed_index(base: &std::path::Path, branch: &str, n: u32) {
    let engine =
        Arc::new(HnswIndexingEngine::new(base.to_path_buf(), 64 * 1024 * 1024, 8).unwrap());
    for i in 0..n {
        engine
            .add_embedding(
                "tenant1",
                "repo1",
                branch,
                &partition(),
                "ws1",
                &format!("node{i}"),
                raisin_hlc::HLC::new(i as u64, 0),
                (0..8).map(|j| (i + j) as f32 * 0.01).collect(),
            )
            .unwrap();
    }
    engine.snapshot_dirty_indexes().unwrap();
}

#[tokio::test]
async fn test_collect_nonexistent_index() {
    let temp_dir = TempDir::new().unwrap();
    let manager = HnswIndexManager::new(temp_dir.path().to_path_buf());

    let metadata = manager
        .collect_index_metadata("tenant1", "repo1", "main", &partition())
        .await
        .unwrap();

    assert!(metadata.is_none());
}

#[tokio::test]
async fn test_collect_existing_index() {
    let temp_dir = TempDir::new().unwrap();
    seed_index(temp_dir.path(), "main", 3);

    let manager = HnswIndexManager::new(temp_dir.path().to_path_buf());
    let metadata = manager
        .collect_index_metadata("tenant1", "repo1", "main", &partition())
        .await
        .unwrap()
        .expect("metadata");

    assert_eq!(metadata.partition, partition());
    // The size is over the BUNDLE. A size over the graph alone would not cover
    // the half that used to go missing.
    let graph = std::fs::metadata(raisin_hnsw::index_path(
        temp_dir.path(),
        "tenant1",
        "repo1",
        "main",
        &partition(),
    ))
    .unwrap()
    .len();
    assert!(
        metadata.size_bytes > graph,
        "transfer size must include the sidecar: bundle {} vs graph {}",
        metadata.size_bytes,
        graph
    );
}

#[tokio::test]
async fn test_list_all_indexes() {
    let temp_dir = TempDir::new().unwrap();
    seed_index(temp_dir.path(), "main", 2);
    seed_index(temp_dir.path(), "develop", 2);

    let manager = HnswIndexManager::new(temp_dir.path().to_path_buf());
    let indexes = manager.list_all_indexes().await.unwrap();

    assert_eq!(indexes.len(), 2);
    assert!(indexes.contains(&(
        "tenant1".to_string(),
        "repo1".to_string(),
        "main".to_string(),
        partition()
    )));
    assert!(indexes.contains(&(
        "tenant1".to_string(),
        "repo1".to_string(),
        "develop".to_string(),
        partition()
    )));
}

/// THE regression test. Before the fix this ended with
/// `Failed to deserialize old index format` on the receiving side, because only
/// `main.hnsw` crossed and the loader read a sidecar-less file as an old bincode
/// index.
#[tokio::test]
async fn a_transferred_index_still_loads_and_still_holds_its_vectors() {
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();
    let staging = TempDir::new().unwrap();

    seed_index(src.path(), "main", 5);

    let manager = HnswIndexManager::new(src.path().to_path_buf());
    let (data, crc32) = manager
        .load_index_data("tenant1", "repo1", "main", &partition())
        .await
        .unwrap()
        .expect("index data");

    let receiver = HnswIndexReceiver::new(dst.path().to_path_buf(), staging.path().to_path_buf());
    let staged = receiver
        .receive_index("tenant1", "repo1", "main", &partition(), data, crc32)
        .await
        .unwrap();
    receiver
        .ingest_index(&staged, "tenant1", "repo1", "main", &partition())
        .await
        .unwrap();

    // Both files landed.
    let target = raisin_hnsw::index_path(dst.path(), "tenant1", "repo1", "main", &partition());
    assert!(target.exists(), "graph file");
    assert!(
        raisin_hnsw::meta_path(&target).exists(),
        "sidecar — this is the file that never used to cross"
    );

    // And the receiving node can actually use it.
    let dst_engine =
        HnswIndexingEngine::new(dst.path().to_path_buf(), 64 * 1024 * 1024, 8).unwrap();
    let stats = dst_engine
        .stats("tenant1", "repo1", "main", &partition())
        .expect("the transferred index must load on the receiving node");
    assert_eq!(stats.count, 5, "transferred index lost its vectors");
    assert_eq!(stats.dimensions, 8);

    let found = dst_engine
        .search(
            "tenant1",
            "repo1",
            "main",
            &partition(),
            &[],
            &(0..8).map(|j| j as f32 * 0.01).collect::<Vec<_>>(),
            3,
        )
        .unwrap();
    assert!(
        !found.is_empty(),
        "the transferred index must answer queries"
    );
}

/// A peer on an older build sends a bare graph file. That must NOT be ingested,
/// and above all must not destroy the healthy index already here — which is what
/// the old `ingest_index` did, by renaming it to `.backup` before moving the
/// unusable payload into its place.
#[tokio::test]
async fn a_bare_graph_payload_is_refused_and_the_local_index_survives() {
    let local = TempDir::new().unwrap();
    let staging = TempDir::new().unwrap();

    seed_index(local.path(), "main", 5);

    let receiver = HnswIndexReceiver::new(local.path().to_path_buf(), staging.path().to_path_buf());

    let bare = std::fs::read(raisin_hnsw::index_path(
        local.path(),
        "tenant1",
        "repo1",
        "main",
        &partition(),
    ))
    .unwrap();
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&bare);
    let crc32 = hasher.finalize();

    let err = receiver
        .receive_index("tenant1", "repo1", "main", &partition(), bare, crc32)
        .await
        .expect_err("an unbundled payload must be refused");
    assert!(
        err.to_string().contains("not a bundle"),
        "the error should name the framing, got: {err}"
    );

    // The local index is untouched.
    let engine = HnswIndexingEngine::new(local.path().to_path_buf(), 64 * 1024 * 1024, 8).unwrap();
    assert_eq!(
        engine
            .stats("tenant1", "repo1", "main", &partition())
            .unwrap()
            .count,
        5,
        "a refused transfer must leave the local index exactly as it was"
    );
}

/// A bundle whose payload is well-framed but not actually an index is caught by
/// the load probe, in staging, before anything reaches the index directory.
#[tokio::test]
async fn a_bundle_that_does_not_load_never_reaches_the_index_directory() {
    let local = TempDir::new().unwrap();
    let staging = TempDir::new().unwrap();

    seed_index(local.path(), "main", 5);
    let receiver = HnswIndexReceiver::new(local.path().to_path_buf(), staging.path().to_path_buf());

    let data = bundle::pack(b"not a usearch index at all", b"{\"dimensions\":8}");
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&data);
    let crc32 = hasher.finalize();

    let err = receiver
        .receive_index("tenant1", "repo1", "main", &partition(), data, crc32)
        .await
        .expect_err("a payload that does not load must be refused");
    assert!(err.to_string().contains("does not load"), "{err}");

    let engine = HnswIndexingEngine::new(local.path().to_path_buf(), 64 * 1024 * 1024, 8).unwrap();
    assert_eq!(
        engine
            .stats("tenant1", "repo1", "main", &partition())
            .unwrap()
            .count,
        5
    );
}

#[tokio::test]
async fn test_checksum_mismatch() {
    let temp_base = TempDir::new().unwrap();
    let temp_staging = TempDir::new().unwrap();

    let receiver = HnswIndexReceiver::new(
        temp_base.path().to_path_buf(),
        temp_staging.path().to_path_buf(),
    );

    let test_data = b"test data".to_vec();
    let wrong_crc32 = 0xDEADBEEF;

    let result = receiver
        .receive_index(
            "tenant1",
            "repo1",
            "main",
            &partition(),
            test_data,
            wrong_crc32,
        )
        .await;

    assert!(result.is_err());
}

/// Refusing to SEND half an index is the other side of the same rule.
#[tokio::test]
async fn an_index_with_no_sidecar_is_not_shipped() {
    let src = TempDir::new().unwrap();
    seed_index(src.path(), "main", 3);

    let index = raisin_hnsw::index_path(src.path(), "tenant1", "repo1", "main", &partition());
    std::fs::remove_file(raisin_hnsw::meta_path(&index)).unwrap();

    let manager = HnswIndexManager::new(src.path().to_path_buf());
    let err = manager
        .load_index_data("tenant1", "repo1", "main", &partition())
        .await
        .expect_err("a sidecar-less index must not be transferred");
    assert!(err.to_string().contains("sidecar"), "{err}");
}
