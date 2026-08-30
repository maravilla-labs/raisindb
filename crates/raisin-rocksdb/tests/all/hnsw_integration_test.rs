// SPDX-License-Identifier: BSL-1.1

//! Integration tests for HNSW vector indexing through the engine layer.
//!
//! These tests exercise the full HNSW stack: adding embeddings, searching,
//! persistence (including mmap view path), multi-tenant isolation, branch
//! operations, purge/rebuild, and concurrent access.
//!
//! Note: The engine applies a MAX_DISTANCE=0.6 cosine distance filter, so test
//! vectors must be sufficiently similar (not orthogonal) to survive filtering.

use raisin_hlc::HLC;
use raisin_hnsw::HnswIndexingEngine;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

/// The partition these tests operate on.
///
/// A vector index is per embedding space now, so every engine call names one.
/// Real callers get theirs from `EmbeddingPartition::to_index_token()`; a test
/// with no configured embedder only needs a valid file stem.
fn test_partition() -> raisin_hnsw::PartitionId {
    raisin_hnsw::PartitionId::new("testhashxxxT")
}

/// Test context with an HNSW engine and temp directory.
struct TestCtx {
    engine: Arc<HnswIndexingEngine>,
    base_path: PathBuf,
    _temp_dir: TempDir,
}

impl TestCtx {
    /// Create a new test context with 8-dimensional vectors.
    fn new() -> Self {
        let temp_dir = tempfile::tempdir().unwrap();
        let base_path = temp_dir.path().to_path_buf();
        let engine = Arc::new(
            HnswIndexingEngine::new(
                base_path.clone(),
                64 * 1024 * 1024, // 64MB cache
                8,
            )
            .unwrap(),
        );
        Self {
            engine,
            base_path,
            _temp_dir: temp_dir,
        }
    }

    /// Create a new engine pointing at the same directory (simulates restart).
    fn new_engine(&self) -> Arc<HnswIndexingEngine> {
        Arc::new(HnswIndexingEngine::new(self.base_path.clone(), 64 * 1024 * 1024, 8).unwrap())
    }
}

/// Create normalized test vectors that are close in cosine distance.
/// Uses a base direction with small perturbations so all vectors are within
/// the engine's MAX_DISTANCE=0.6 threshold of the query.
fn make_vector(seed: f32) -> Vec<f32> {
    let raw: Vec<f32> = (0..8)
        .map(|i| 1.0 + seed * 0.1 * (i as f32 + 1.0))
        .collect();
    let mag = raw.iter().map(|x| x * x).sum::<f32>().sqrt();
    raw.iter().map(|x| x / mag).collect()
}

/// Create a query vector close to a given seed.
fn make_query(seed: f32) -> Vec<f32> {
    make_vector(seed)
}

// ---------------------------------------------------------------------------
// 1. Basic add + search round-trip
// ---------------------------------------------------------------------------

#[test]
fn test_hnsw_add_search_roundtrip() {
    let ctx = TestCtx::new();

    ctx.engine
        .add_embedding(
            "t1",
            "r1",
            "main",
            &test_partition(),
            "ws1",
            "n1",
            HLC::new(1, 0),
            make_vector(1.0),
        )
        .unwrap();
    ctx.engine
        .add_embedding(
            "t1",
            "r1",
            "main",
            &test_partition(),
            "ws1",
            "n2",
            HLC::new(2, 0),
            make_vector(2.0),
        )
        .unwrap();
    ctx.engine
        .add_embedding(
            "t1",
            "r1",
            "main",
            &test_partition(),
            "ws1",
            "n3",
            HLC::new(3, 0),
            make_vector(3.0),
        )
        .unwrap();

    // Query close to n1
    let results = ctx
        .engine
        .search(
            "t1",
            "r1",
            "main",
            &test_partition(),
            &["ws1".to_string()],
            &make_query(1.0),
            3,
        )
        .unwrap();

    assert_eq!(results.len(), 3);
    assert_eq!(results[0].node_id, "n1", "nearest neighbor should be n1");
}

// ---------------------------------------------------------------------------
// 2. Persistence across engine restarts (exercises mmap view path)
// ---------------------------------------------------------------------------

#[test]
fn test_hnsw_persistence_across_engines() {
    let ctx = TestCtx::new();

    ctx.engine
        .add_embedding(
            "t1",
            "r1",
            "main",
            &test_partition(),
            "ws1",
            "n1",
            HLC::new(1, 0),
            make_vector(1.0),
        )
        .unwrap();
    ctx.engine
        .add_embedding(
            "t1",
            "r1",
            "main",
            &test_partition(),
            "ws1",
            "n2",
            HLC::new(2, 0),
            make_vector(2.0),
        )
        .unwrap();

    ctx.engine.snapshot_dirty_indexes().unwrap();

    // New engine loads from disk via mmap view
    let engine2 = ctx.new_engine();
    let results = engine2
        .search(
            "t1",
            "r1",
            "main",
            &test_partition(),
            &["ws1".to_string()],
            &make_query(1.0),
            2,
        )
        .unwrap();

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].node_id, "n1");
}

// ---------------------------------------------------------------------------
// 3. mmap view → mutate → snapshot → view again
// ---------------------------------------------------------------------------

#[test]
fn test_hnsw_mmap_view_then_mutate() {
    let ctx = TestCtx::new();

    // Phase 1: add + snapshot
    ctx.engine
        .add_embedding(
            "t1",
            "r1",
            "main",
            &test_partition(),
            "ws1",
            "n1",
            HLC::new(1, 0),
            make_vector(1.0),
        )
        .unwrap();
    ctx.engine.snapshot_dirty_indexes().unwrap();

    // Phase 2: new engine (mmap view), then mutate (triggers promotion)
    let engine2 = ctx.new_engine();
    engine2
        .add_embedding(
            "t1",
            "r1",
            "main",
            &test_partition(),
            "ws1",
            "n2",
            HLC::new(2, 0),
            make_vector(2.0),
        )
        .unwrap();

    // Both vectors should be searchable
    let results = engine2
        .search(
            "t1",
            "r1",
            "main",
            &test_partition(),
            &["ws1".to_string()],
            &make_query(1.5),
            10,
        )
        .unwrap();
    assert_eq!(results.len(), 2);

    // Phase 3: snapshot again, then verify with a third engine
    engine2.snapshot_dirty_indexes().unwrap();

    let engine3 = ctx.new_engine();
    let results = engine3
        .search(
            "t1",
            "r1",
            "main",
            &test_partition(),
            &["ws1".to_string()],
            &make_query(1.5),
            10,
        )
        .unwrap();
    assert_eq!(results.len(), 2);
}

// ---------------------------------------------------------------------------
// 4. Multi-tenant isolation
// ---------------------------------------------------------------------------

#[test]
fn test_hnsw_multi_tenant_isolation() {
    let ctx = TestCtx::new();

    ctx.engine
        .add_embedding(
            "tenant-a",
            "r1",
            "main",
            &test_partition(),
            "ws1",
            "na1",
            HLC::new(1, 0),
            make_vector(1.0),
        )
        .unwrap();
    ctx.engine
        .add_embedding(
            "tenant-b",
            "r1",
            "main",
            &test_partition(),
            "ws1",
            "nb1",
            HLC::new(2, 0),
            make_vector(2.0),
        )
        .unwrap();

    // tenant-a should only see its own vectors
    let results_a = ctx
        .engine
        .search(
            "tenant-a",
            "r1",
            "main",
            &test_partition(),
            &["ws1".to_string()],
            &make_query(1.0),
            10,
        )
        .unwrap();
    assert_eq!(results_a.len(), 1);
    assert_eq!(results_a[0].node_id, "na1");

    // tenant-b should only see its own vectors
    let results_b = ctx
        .engine
        .search(
            "tenant-b",
            "r1",
            "main",
            &test_partition(),
            &["ws1".to_string()],
            &make_query(2.0),
            10,
        )
        .unwrap();
    assert_eq!(results_b.len(), 1);
    assert_eq!(results_b[0].node_id, "nb1");
}

// ---------------------------------------------------------------------------
// 5. Branch copy and isolation
// ---------------------------------------------------------------------------

#[test]
fn test_hnsw_branch_copy_and_search() {
    let ctx = TestCtx::new();

    ctx.engine
        .add_embedding(
            "t1",
            "r1",
            "main",
            &test_partition(),
            "ws1",
            "n1",
            HLC::new(1, 0),
            make_vector(1.0),
        )
        .unwrap();
    ctx.engine.snapshot_dirty_indexes().unwrap();

    // Copy main → feature
    ctx.engine
        .copy_for_branch("t1", "r1", "main", "feature")
        .unwrap();

    // Feature branch should have the same data
    let results = ctx
        .engine
        .search(
            "t1",
            "r1",
            "feature",
            &test_partition(),
            &["ws1".to_string()],
            &make_query(1.0),
            10,
        )
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].node_id, "n1");

    // Add to feature only
    ctx.engine
        .add_embedding(
            "t1",
            "r1",
            "feature",
            &test_partition(),
            "ws1",
            "n2",
            HLC::new(2, 0),
            make_vector(1.5),
        )
        .unwrap();

    // Main should NOT see n2
    let main_results = ctx
        .engine
        .search(
            "t1",
            "r1",
            "main",
            &test_partition(),
            &["ws1".to_string()],
            &make_query(1.5),
            10,
        )
        .unwrap();
    assert!(
        main_results.iter().all(|r| r.node_id != "n2"),
        "main branch should not contain feature-only embedding"
    );

    // Feature should see both
    let feature_results = ctx
        .engine
        .search(
            "t1",
            "r1",
            "feature",
            &test_partition(),
            &["ws1".to_string()],
            &make_query(1.2),
            10,
        )
        .unwrap();
    assert_eq!(feature_results.len(), 2);
}

// ---------------------------------------------------------------------------
// 6. Purge and rebuild
// ---------------------------------------------------------------------------

#[test]
fn test_hnsw_purge_and_rebuild() {
    let ctx = TestCtx::new();

    ctx.engine
        .add_embedding(
            "t1",
            "r1",
            "main",
            &test_partition(),
            "ws1",
            "n1",
            HLC::new(1, 0),
            make_vector(1.0),
        )
        .unwrap();
    ctx.engine
        .add_embedding(
            "t1",
            "r1",
            "main",
            &test_partition(),
            "ws1",
            "n2",
            HLC::new(2, 0),
            make_vector(2.0),
        )
        .unwrap();
    ctx.engine.snapshot_dirty_indexes().unwrap();

    // Purge
    ctx.engine
        .purge_index("t1", "r1", "main", &test_partition())
        .unwrap();

    let results = ctx
        .engine
        .search(
            "t1",
            "r1",
            "main",
            &test_partition(),
            &["ws1".to_string()],
            &make_query(1.0),
            10,
        )
        .unwrap();
    assert!(results.is_empty(), "purged index should return no results");

    // Rebuild by re-adding
    ctx.engine
        .add_embedding(
            "t1",
            "r1",
            "main",
            &test_partition(),
            "ws1",
            "n3",
            HLC::new(3, 0),
            make_vector(3.0),
        )
        .unwrap();

    let results = ctx
        .engine
        .search(
            "t1",
            "r1",
            "main",
            &test_partition(),
            &["ws1".to_string()],
            &make_query(3.0),
            10,
        )
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].node_id, "n3");
}

// ---------------------------------------------------------------------------
// 7. Remove embedding
// ---------------------------------------------------------------------------

#[test]
fn test_hnsw_remove_embedding() {
    let ctx = TestCtx::new();

    ctx.engine
        .add_embedding(
            "t1",
            "r1",
            "main",
            &test_partition(),
            "ws1",
            "n1",
            HLC::new(1, 0),
            make_vector(1.0),
        )
        .unwrap();
    ctx.engine
        .add_embedding(
            "t1",
            "r1",
            "main",
            &test_partition(),
            "ws1",
            "n2",
            HLC::new(2, 0),
            make_vector(1.5),
        )
        .unwrap();
    ctx.engine
        .add_embedding(
            "t1",
            "r1",
            "main",
            &test_partition(),
            "ws1",
            "n3",
            HLC::new(3, 0),
            make_vector(2.0),
        )
        .unwrap();

    // Remove n2
    ctx.engine
        .remove_embedding("t1", "r1", "main", &test_partition(), "n2")
        .unwrap();

    let results = ctx
        .engine
        .search(
            "t1",
            "r1",
            "main",
            &test_partition(),
            &["ws1".to_string()],
            &make_query(1.5),
            10,
        )
        .unwrap();
    assert_eq!(results.len(), 2);
    assert!(
        results.iter().all(|r| r.node_id != "n2"),
        "removed embedding should not appear in search results"
    );
}

// ---------------------------------------------------------------------------
// 8. Concurrent read/write
// ---------------------------------------------------------------------------

#[test]
fn test_hnsw_concurrent_read_write() {
    let ctx = TestCtx::new();

    // Seed with data (all similar vectors so they survive distance filtering)
    for i in 0..10 {
        ctx.engine
            .add_embedding(
                "t1",
                "r1",
                "main",
                &test_partition(),
                "ws1",
                &format!("seed-{}", i),
                HLC::new(i as u64, 0),
                make_vector(1.0 + i as f32 * 0.1),
            )
            .unwrap();
    }

    let engine = ctx.engine.clone();
    let mut handles = Vec::new();

    // Spawn reader threads
    for _ in 0..4 {
        let e = engine.clone();
        handles.push(std::thread::spawn(move || {
            for _ in 0..20 {
                let _ = e.search(
                    "t1",
                    "r1",
                    "main",
                    &test_partition(),
                    &["ws1".to_string()],
                    &make_query(1.5),
                    5,
                );
            }
        }));
    }

    // Spawn writer threads
    for t in 0..4 {
        let e = engine.clone();
        handles.push(std::thread::spawn(move || {
            for i in 0..5 {
                let id = format!("writer-{}-{}", t, i);
                let _ = e.add_embedding(
                    "t1",
                    "r1",
                    "main",
                    &test_partition(),
                    "ws1",
                    &id,
                    HLC::new(100 + t as u64 * 10 + i as u64, 0),
                    make_vector(1.0 + t as f32 * 0.1 + i as f32 * 0.01),
                );
            }
        }));
    }

    // All threads should complete without panics
    for h in handles {
        h.join().expect("thread panicked during concurrent access");
    }

    // Verify we can still search
    let results = ctx
        .engine
        .search(
            "t1",
            "r1",
            "main",
            &test_partition(),
            &["ws1".to_string()],
            &make_query(1.5),
            50,
        )
        .unwrap();
    // 10 seed + up to 20 from writers
    assert!(results.len() >= 10, "should have at least seed data");
}

// ---------------------------------------------------------------------------
// 12. Chunked documents: search must return FETCHABLE source ids, one per doc
// ---------------------------------------------------------------------------

/// A chunked document is filed in the index under `{node_id}#{chunk_index}`
/// (`jobs/handlers/embedding/handler.rs`). That id exists nowhere else: it
/// matches no full-text hit, and `storage.nodes().get(scope, "abc#3")` is
/// `None`.
///
/// Regression, measured on a live server before the fix: a 37-chunk document
/// took 9 of the 10 vector slots for its own query, every one of them
/// unresolvable, and the document itself never appeared as a row —
/// `HYBRID_SEARCH` reported `fulltext_rank: 1, vector_rank: null` for it while
/// an unchunked document carrying the same text fused normally. The vector half
/// was dead for exactly the documents RAG exists to retrieve.
#[test]
fn test_chunked_document_search_returns_fetchable_source_ids() {
    let ctx = TestCtx::new();

    // One document split into four chunks, plus two unchunked documents.
    for chunk in 0..4u64 {
        ctx.engine
            .add_embedding(
                "t1",
                "r1",
                "main",
                &test_partition(),
                "ws1",
                &format!("long-doc#{}", chunk),
                HLC::new(10 + chunk, 0),
                // Chunk 2 is the closest to the query; the rest trail it.
                make_vector(1.0 + (chunk as f32 - 2.0).abs() * 0.05),
            )
            .unwrap();
    }
    ctx.engine
        .add_embedding(
            "t1",
            "r1",
            "main",
            &test_partition(),
            "ws1",
            "short-doc",
            HLC::new(20, 0),
            make_vector(1.3),
        )
        .unwrap();
    ctx.engine
        .add_embedding(
            "t1",
            "r1",
            "main",
            &test_partition(),
            "ws1",
            "other-doc",
            HLC::new(21, 0),
            make_vector(1.6),
        )
        .unwrap();

    let results = ctx
        .engine
        .search(
            "t1",
            "r1",
            "main",
            &test_partition(),
            &["ws1".to_string()],
            &make_query(1.0),
            10,
        )
        .unwrap();

    // No result may carry a chunk suffix in `node_id`: that field is what every
    // caller passes to `storage.nodes().get(...)`.
    for r in &results {
        assert!(
            !r.node_id.contains('#'),
            "node_id must be fetchable, got {:?}",
            r.node_id
        );
    }

    // One row per source document — the four chunks collapse to one.
    let ids: Vec<&str> = results.iter().map(|r| r.node_id.as_str()).collect();
    assert_eq!(
        ids.iter().filter(|id| **id == "long-doc").count(),
        1,
        "a chunked document must occupy exactly one result slot, got {:?}",
        ids
    );
    assert_eq!(
        results.len(),
        3,
        "three source documents were indexed, got {:?}",
        ids
    );

    // The chunk that actually won is still identifiable.
    let long = results.iter().find(|r| r.node_id == "long-doc").unwrap();
    assert_eq!(long.chunk_id, "long-doc#2", "best chunk should be #2");
    assert_eq!(long.chunk_index, 2);
    assert!(long.is_chunk());

    let short = results.iter().find(|r| r.node_id == "short-doc").unwrap();
    assert_eq!(short.chunk_id, "short-doc");
    assert_eq!(short.chunk_index, 0);
    assert!(!short.is_chunk());
}

/// `k` must mean "k documents", not "k index entries".
///
/// Without the collapse one chunked document fills the whole result set and
/// every other document is crowded out — a `LIMIT 3` that returns three slices
/// of the same page.
#[test]
fn test_chunk_collapse_does_not_crowd_out_other_documents() {
    let ctx = TestCtx::new();

    // A document with more chunks than the requested limit, all of them nearer
    // the query than the other documents.
    for chunk in 0..8u64 {
        ctx.engine
            .add_embedding(
                "t1",
                "r1",
                "main",
                &test_partition(),
                "ws1",
                &format!("verbose#{}", chunk),
                HLC::new(10 + chunk, 0),
                make_vector(1.0 + chunk as f32 * 0.001),
            )
            .unwrap();
    }
    for (i, name) in ["alpha", "beta"].iter().enumerate() {
        ctx.engine
            .add_embedding(
                "t1",
                "r1",
                "main",
                &test_partition(),
                "ws1",
                name,
                HLC::new(30 + i as u64, 0),
                make_vector(1.1 + i as f32 * 0.05),
            )
            .unwrap();
    }

    let results = ctx
        .engine
        .search(
            "t1",
            "r1",
            "main",
            &test_partition(),
            &["ws1".to_string()],
            &make_query(1.0),
            3,
        )
        .unwrap();

    let ids: Vec<&str> = results.iter().map(|r| r.node_id.as_str()).collect();
    assert_eq!(
        ids.len(),
        3,
        "LIMIT 3 should still yield 3 documents, got {:?}",
        ids
    );
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        3,
        "each slot must be a DIFFERENT document, got {:?}",
        ids
    );
    assert!(ids.contains(&"alpha"), "alpha was crowded out: {:?}", ids);
    assert!(ids.contains(&"beta"), "beta was crowded out: {:?}", ids);
}
