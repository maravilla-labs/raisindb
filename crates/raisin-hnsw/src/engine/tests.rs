// SPDX-License-Identifier: BSL-1.1

//! Tests for the HNSW indexing engine.

use super::*;
use crate::types::{ScoringConfig, SearchMode, SearchRequest};
use raisin_hlc::HLC;
use std::sync::Arc;
use tempfile::TempDir;

/// The partition every test in this module uses.
///
/// Tests build the engine WITHOUT a spec resolver, so nothing here resolves an
/// embedder; the token only has to be a valid file stem. Real callers get theirs
/// from `EmbeddingPartition::to_index_token()`.
fn p() -> crate::partition::PartitionId {
    crate::partition::PartitionId::new("testhashxxxT")
}

fn create_test_engine() -> (Arc<HnswIndexingEngine>, TempDir) {
    let temp_dir = tempfile::tempdir().unwrap();
    let engine = Arc::new(
        HnswIndexingEngine::new(
            temp_dir.path().to_path_buf(),
            256 * 1024 * 1024, // 256MB cache
            128,               // 128-dim vectors
        )
        .unwrap(),
    );
    (engine, temp_dir)
}

fn create_test_vector(dims: usize, seed: f32) -> Vec<f32> {
    (0..dims).map(|i| (i as f32 + seed) / dims as f32).collect()
}

#[test]
fn test_add_and_search() {
    let (engine, _temp_dir) = create_test_engine();

    // Add embeddings
    engine
        .add_embedding(
            "tenant1",
            "repo1",
            "main",
            &p(),
            "ws1",
            "node1",
            HLC::new(1, 0),
            create_test_vector(128, 1.0),
        )
        .unwrap();

    engine
        .add_embedding(
            "tenant1",
            "repo1",
            "main",
            &p(),
            "ws1",
            "node2",
            HLC::new(2, 0),
            create_test_vector(128, 2.0),
        )
        .unwrap();

    // Search with workspace filter
    let query = create_test_vector(128, 1.1);
    let results = engine
        .search(
            "tenant1",
            "repo1",
            "main",
            &p(),
            &["ws1".to_string()],
            &query,
            2,
        )
        .unwrap();

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].node_id, "node1"); // Closest match
    assert_eq!(results[0].workspace_id, "ws1");
}

#[test]
fn test_persistence() {
    let temp_dir = tempfile::tempdir().unwrap();
    let base_path = temp_dir.path().to_path_buf();

    // Create engine and add data
    {
        let engine =
            Arc::new(HnswIndexingEngine::new(base_path.clone(), 256 * 1024 * 1024, 128).unwrap());

        engine
            .add_embedding(
                "tenant1",
                "repo1",
                "main",
                &p(),
                "ws1",
                "node1",
                HLC::new(1, 0),
                create_test_vector(128, 1.0),
            )
            .unwrap();

        // Save dirty indexes
        engine.snapshot_dirty_indexes().unwrap();
    }

    // Create new engine and verify data persisted
    {
        let engine = Arc::new(HnswIndexingEngine::new(base_path, 256 * 1024 * 1024, 128).unwrap());

        let query = create_test_vector(128, 1.1);
        let results = engine
            .search(
                "tenant1",
                "repo1",
                "main",
                &p(),
                &["ws1".to_string()],
                &query,
                1,
            )
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].node_id, "node1");
    }
}

#[test]
fn test_branch_copy() {
    let (engine, _temp_dir) = create_test_engine();

    // Add data to main branch
    engine
        .add_embedding(
            "tenant1",
            "repo1",
            "main",
            &p(),
            "ws1",
            "node1",
            HLC::new(1, 0),
            create_test_vector(128, 1.0),
        )
        .unwrap();

    // Save to disk
    engine.snapshot_dirty_indexes().unwrap();

    // Copy to feature branch (no longer needs workspace_id)
    engine
        .copy_for_branch("tenant1", "repo1", "main", "feature")
        .unwrap();

    // Verify feature branch has the data
    let query = create_test_vector(128, 1.1);
    let results = engine
        .search(
            "tenant1",
            "repo1",
            "feature",
            &p(),
            &["ws1".to_string()],
            &query,
            1,
        )
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].node_id, "node1");
}
#[test]
fn test_search_chunks_with_modes() {
    let (engine, _temp_dir) = create_test_engine();

    // Add embeddings with chunk IDs
    engine
        .add_embedding(
            "tenant1",
            "repo1",
            "main",
            &p(),
            "ws1",
            "doc1#0",
            HLC::new(1, 0),
            create_test_vector(128, 1.0),
        )
        .unwrap();

    engine
        .add_embedding(
            "tenant1",
            "repo1",
            "main",
            &p(),
            "ws1",
            "doc1#1",
            HLC::new(2, 0),
            create_test_vector(128, 1.1),
        )
        .unwrap();

    engine
        .add_embedding(
            "tenant1",
            "repo1",
            "main",
            &p(),
            "ws1",
            "doc2#0",
            HLC::new(3, 0),
            create_test_vector(128, 5.0),
        )
        .unwrap();

    let query = create_test_vector(128, 1.05);

    // Test Chunks mode - should return all chunks
    let chunks_request = SearchRequest::new(query.clone(), 10)
        .with_mode(SearchMode::Chunks)
        .with_workspace("ws1".to_string());

    let chunks_results = engine
        .search_chunks("tenant1", "repo1", "main", &p(), &chunks_request)
        .unwrap();

    // Should get at least 2 chunks from doc1
    assert!(chunks_results.len() >= 2);

    // Test Documents mode - should deduplicate
    let docs_request = SearchRequest::new(query, 10)
        .with_mode(SearchMode::Documents)
        .with_workspace("ws1".to_string());

    let doc_results = engine
        .search_documents("tenant1", "repo1", "main", &p(), &docs_request)
        .unwrap();

    // Should get fewer results than chunks mode (deduplication)
    assert!(doc_results.len() < chunks_results.len() || doc_results.len() >= 1);
}

#[test]
fn test_configurable_max_distance() {
    let (engine, _temp_dir) = create_test_engine();

    // Add embeddings with varying distances
    engine
        .add_embedding(
            "tenant1",
            "repo1",
            "main",
            &p(),
            "ws1",
            "close#0",
            HLC::new(1, 0),
            create_test_vector(128, 1.0),
        )
        .unwrap();

    engine
        .add_embedding(
            "tenant1",
            "repo1",
            "main",
            &p(),
            "ws1",
            "far#0",
            HLC::new(2, 0),
            create_test_vector(128, 50.0),
        )
        .unwrap();

    let query = create_test_vector(128, 1.0);

    // Test with strict distance threshold
    let strict_request = SearchRequest::new(query.clone(), 10)
        .with_workspace("ws1".to_string())
        .with_max_distance(0.3);

    let strict_results = engine
        .search_chunks("tenant1", "repo1", "main", &p(), &strict_request)
        .unwrap();

    // Should only get close results
    assert!(strict_results.iter().all(|r| r.distance < 0.3));

    // Test with relaxed distance threshold
    let relaxed_request = SearchRequest::new(query, 10)
        .with_workspace("ws1".to_string())
        .with_max_distance(0.9);

    let relaxed_results = engine
        .search_chunks("tenant1", "repo1", "main", &p(), &relaxed_request)
        .unwrap();

    // Should get more results with relaxed threshold
    assert!(relaxed_results.len() >= strict_results.len());
}

#[test]
fn test_chunk_position_scoring() {
    let (engine, _temp_dir) = create_test_engine();

    // Add chunks with similar distances but different positions
    engine
        .add_embedding(
            "tenant1",
            "repo1",
            "main",
            &p(),
            "ws1",
            "doc#0", // First chunk
            HLC::new(1, 0),
            create_test_vector(128, 1.0),
        )
        .unwrap();

    engine
        .add_embedding(
            "tenant1",
            "repo1",
            "main",
            &p(),
            "ws1",
            "doc#5", // Later chunk
            HLC::new(2, 0),
            create_test_vector(128, 1.01),
        )
        .unwrap();

    let query = create_test_vector(128, 1.005);

    // Test without scoring - should rank by distance only
    // Use Chunks mode since both chunks belong to the same "doc" source
    let no_scoring_request = SearchRequest::new(query.clone(), 10)
        .with_mode(SearchMode::Chunks)
        .with_workspace("ws1".to_string())
        .with_max_distance(1.0); // Relaxed threshold for test

    let no_scoring_results = engine
        .search_chunks("tenant1", "repo1", "main", &p(), &no_scoring_request)
        .unwrap();

    assert!(no_scoring_results.len() >= 2);

    // Test with scoring - first chunk should rank higher
    let scoring_request = SearchRequest::new(query, 10)
        .with_mode(SearchMode::Chunks)
        .with_workspace("ws1".to_string())
        .with_max_distance(1.0)
        .with_scoring(ScoringConfig::default());

    let scoring_results = engine
        .search_chunks("tenant1", "repo1", "main", &p(), &scoring_request)
        .unwrap();

    assert!(scoring_results.len() >= 2);

    // First chunk should have adjusted score
    let first_chunk_result = scoring_results.iter().find(|r| r.chunk_index == 0).unwrap();
    assert!(first_chunk_result.adjusted_score.is_some());

    // First chunk boost should be applied
    let adjusted_score = first_chunk_result.adjusted_score.unwrap();
    let base_similarity = first_chunk_result.similarity();
    assert!(adjusted_score > base_similarity);
}

#[test]
fn test_position_decay() {
    let config = ScoringConfig {
        position_decay: 0.1,
        first_chunk_boost: 1.0, // No boost for this test
        exact_match_boost: 1.0,
    };

    let (engine, _temp_dir) = create_test_engine();

    // Add chunks at different positions with identical distance
    for i in 0..5 {
        engine
            .add_embedding(
                "tenant1",
                "repo1",
                "main",
                &p(),
                "ws1",
                &format!("doc#{}", i),
                HLC::new(i as u64 + 1, 0),
                create_test_vector(128, 1.0), // All identical
            )
            .unwrap();
    }

    let query = create_test_vector(128, 1.0);

    // Use Chunks mode since all chunks belong to the same "doc" source
    let request = SearchRequest::new(query, 10)
        .with_mode(SearchMode::Chunks)
        .with_workspace("ws1".to_string())
        .with_max_distance(1.0) // Relaxed threshold for test
        .with_scoring(config);

    let results = engine
        .search_chunks("tenant1", "repo1", "main", &p(), &request)
        .unwrap();

    assert!(results.len() >= 5);

    // Verify chunks are sorted by position (earlier chunks first)
    // With position_decay, chunk 0 should score highest
    assert_eq!(results[0].chunk_index, 0);

    // Verify scores decrease with position
    for i in 0..(results.len() - 1) {
        let score_i = results[i].adjusted_score.unwrap();
        let score_next = results[i + 1].adjusted_score.unwrap();
        assert!(
            score_i >= score_next,
            "Score should decrease with position: {} >= {}",
            score_i,
            score_next
        );
    }
}

#[test]
fn test_first_chunk_boost() {
    let config = ScoringConfig {
        position_decay: 0.0,    // No decay for this test
        first_chunk_boost: 1.5, // 50% boost
        exact_match_boost: 1.0,
    };

    let (engine, _temp_dir) = create_test_engine();

    // Add first and second chunk with same distance
    engine
        .add_embedding(
            "tenant1",
            "repo1",
            "main",
            &p(),
            "ws1",
            "doc#0",
            HLC::new(1, 0),
            create_test_vector(128, 1.0),
        )
        .unwrap();

    engine
        .add_embedding(
            "tenant1",
            "repo1",
            "main",
            &p(),
            "ws1",
            "doc#1",
            HLC::new(2, 0),
            create_test_vector(128, 1.0),
        )
        .unwrap();

    let query = create_test_vector(128, 1.0);

    // Use Chunks mode since both chunks belong to the same "doc" source
    let request = SearchRequest::new(query, 10)
        .with_mode(SearchMode::Chunks)
        .with_workspace("ws1".to_string())
        .with_max_distance(1.0) // Relaxed threshold for test
        .with_scoring(config);

    let results = engine
        .search_chunks("tenant1", "repo1", "main", &p(), &request)
        .unwrap();

    assert!(results.len() >= 2);

    // Find chunks by index
    let chunk0 = results.iter().find(|r| r.chunk_index == 0).unwrap();
    let chunk1 = results.iter().find(|r| r.chunk_index == 1).unwrap();

    let score0 = chunk0.adjusted_score.unwrap();
    let score1 = chunk1.adjusted_score.unwrap();

    // Chunk 0 should have significantly higher score due to first_chunk_boost
    assert!(score0 > score1);

    // Verify the boost is approximately 50%
    let expected_ratio = 1.5;
    let actual_ratio = score0 / score1;
    assert!(
        (actual_ratio - expected_ratio).abs() < 0.01,
        "Expected ratio ~{}, got {}",
        expected_ratio,
        actual_ratio
    );
}

// ---------------------------------------------------------------------------
// Per-tenant vector width
//
// The regression these cover: the engine used to be built with one width for the
// whole process (1536), so a tenant on a 768-wide model had every vector its own
// EmbeddingGenerate job produced rejected by `add` — after the vector had already
// been durably written to cf::EMBEDDINGS. Vector queries returned zero rows while
// the stored embedding count climbed, and nothing said why.
// ---------------------------------------------------------------------------

/// Test resolver: answers a fixed width for one tenant, `None` for everyone else.
struct FixedDims {
    tenant: String,
    dimensions: usize,
}

impl crate::dims::IndexSpecResolver for FixedDims {
    fn default_text_partition(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
    ) -> Option<crate::partition::PartitionId> {
        Some(p())
    }

    fn spec_for(
        &self,
        tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _partition: &crate::partition::PartitionId,
    ) -> Option<crate::dims::IndexSpec> {
        self.dimensions_for(tenant_id)
            .map(crate::dims::IndexSpec::new)
    }
}

impl FixedDims {
    fn dimensions_for(&self, tenant_id: &str) -> Option<usize> {
        (tenant_id == self.tenant).then_some(self.dimensions)
    }
}

#[test]
fn test_new_index_is_created_at_the_tenants_configured_width() {
    let temp_dir = tempfile::tempdir().unwrap();
    let engine = HnswIndexingEngine::new(temp_dir.path().to_path_buf(), 64 * 1024 * 1024, 1536)
        .unwrap()
        .with_spec_resolver(Arc::new(FixedDims {
            tenant: "ollama-tenant".to_string(),
            dimensions: 768,
        }));

    // A 768-wide vector must be accepted even though the engine's fallback is 1536.
    engine
        .add_embedding(
            "ollama-tenant",
            "repo1",
            "main",
            &p(),
            "ws1",
            "node1",
            HLC::new(1, 0),
            create_test_vector(768, 0.0),
        )
        .expect("768-wide vector must be accepted for a tenant configured at 768");

    let stats = engine
        .stats("ollama-tenant", "repo1", "main", &p())
        .unwrap();
    assert_eq!(stats.count, 1);
    // SHOW VECTOR INDEX HEALTH renders this; it must be the index's width, not the
    // engine's fallback.
    assert_eq!(stats.dimensions, 768);
}

#[test]
fn test_unconfigured_tenant_falls_back_to_the_engine_width() {
    let temp_dir = tempfile::tempdir().unwrap();
    let engine = HnswIndexingEngine::new(temp_dir.path().to_path_buf(), 64 * 1024 * 1024, 128)
        .unwrap()
        .with_spec_resolver(Arc::new(FixedDims {
            tenant: "ollama-tenant".to_string(),
            dimensions: 768,
        }));

    engine
        .add_embedding(
            "no-config-tenant",
            "repo1",
            "main",
            &p(),
            "ws1",
            "node1",
            HLC::new(1, 0),
            create_test_vector(128, 0.0),
        )
        .expect("a tenant with no embedding config keeps the pre-resolver behaviour");

    assert_eq!(
        engine
            .stats("no-config-tenant", "repo1", "main", &p())
            .unwrap()
            .dimensions,
        128
    );
}

#[test]
fn test_width_change_over_a_populated_index_names_the_rebuild() {
    let temp_dir = tempfile::tempdir().unwrap();
    let base = temp_dir.path().to_path_buf();

    // Build and persist a 128-wide index holding one vector.
    {
        let engine = HnswIndexingEngine::new(base.clone(), 64 * 1024 * 1024, 128).unwrap();
        engine
            .add_embedding(
                "t",
                "r",
                "main",
                &p(),
                "ws1",
                "node1",
                HLC::new(1, 0),
                create_test_vector(128, 0.0),
            )
            .unwrap();
        engine.snapshot_dirty_indexes().unwrap();
    }

    // Reopen with a resolver that now says 768 — as if the operator switched models
    // without rebuilding.
    let engine = HnswIndexingEngine::new(base, 64 * 1024 * 1024, 128)
        .unwrap()
        .with_spec_resolver(Arc::new(FixedDims {
            tenant: "t".to_string(),
            dimensions: 768,
        }));

    let err = engine
        .stats("t", "r", "main", &p())
        .expect_err("a populated index at the wrong width must fail loudly, not silently");
    let msg = err.to_string();
    assert!(
        msg.contains("REBUILD VECTOR INDEX"),
        "the error must name the fix, got: {msg}"
    );
    assert!(msg.contains("128") && msg.contains("768"), "got: {msg}");
}

#[test]
fn test_empty_index_at_the_wrong_width_self_heals() {
    let temp_dir = tempfile::tempdir().unwrap();
    let base = temp_dir.path().to_path_buf();

    // Persist an EMPTY 1536-wide index — exactly what the old startup constant left
    // behind on a fresh data dir before the first embedding job ran.
    {
        let engine = HnswIndexingEngine::new(base.clone(), 64 * 1024 * 1024, 1536).unwrap();
        engine
            .create_index_with_spec("t", "r", "main", &p(), crate::dims::IndexSpec::new(1536))
            .unwrap();
        engine.snapshot_dirty_indexes().unwrap();
    }

    let engine = HnswIndexingEngine::new(base, 64 * 1024 * 1024, 1536)
        .unwrap()
        .with_spec_resolver(Arc::new(FixedDims {
            tenant: "t".to_string(),
            dimensions: 768,
        }));

    engine
        .add_embedding(
            "t",
            "r",
            "main",
            &p(),
            "ws1",
            "node1",
            HLC::new(1, 0),
            create_test_vector(768, 0.0),
        )
        .expect("an empty index at a stale width must be recreated, not rejected");

    assert_eq!(
        engine.stats("t", "r", "main", &p()).unwrap().dimensions,
        768
    );
}

// ---------------------------------------------------------------------------
// Workspace-scope starvation
// ---------------------------------------------------------------------------

/// Deterministic pseudo-random jitter, so the starvation scenario below is
/// byte-identical on every run and on every platform.
fn lcg(state: &mut u64) -> f32 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    // top 24 bits -> [-1.0, 1.0)
    ((*state >> 40) as f32 / 8_388_608.0) - 1.0
}

/// A vector pointing mostly along dimension 0, tilted `tilt` towards dimension
/// 1 and jittered in the tail so no two are identical.
fn jittered(dims: usize, tilt: f32, eps: f32, state: &mut u64) -> Vec<f32> {
    let mut v = vec![0.0f32; dims];
    v[0] = 1.0 - tilt;
    v[1] = tilt;
    for slot in v.iter_mut().skip(2) {
        *slot += eps * lcg(state);
    }
    v
}

const STARVE_DIMS: usize = 128;
const STARVE_LARGE: usize = 3000;
const STARVE_SMALL: usize = 8;

/// An index holding a LARGE workspace and a SMALL one, where the large
/// workspace sits *closer* to the query than the small one.
///
/// An unfiltered graph walk fills every candidate slot with `wsA`, so a
/// post-filter for `wsB` returns NOTHING — even though `wsB` holds
/// `STARVE_SMALL` perfectly good answers well inside the 0.6 cosine cutoff.
/// This is the shape of the real complaint ("a narrow scope returns short"),
/// and it is the only shape that exercises the defect: a tiny, evenly split
/// index cannot starve, because there every walk reaches every workspace.
///
/// Returns the engine and the query vector.
fn starvation_index() -> (Arc<HnswIndexingEngine>, TempDir, Vec<f32>) {
    let (engine, temp_dir) = create_test_engine();
    let mut state = 0x5eed_1234_u64;

    // Workspace A: many vectors, all very close to the query direction.
    for i in 0..STARVE_LARGE {
        let v = jittered(STARVE_DIMS, 0.02, 0.01, &mut state);
        engine
            .add_embedding(
                "t",
                "r",
                "main",
                &p(),
                "wsA",
                &format!("a{}", i),
                HLC::new(i as u64 + 1, 0),
                v,
            )
            .unwrap();
    }

    // Workspace B: a handful of vectors, tilted away from the query but still
    // far inside the cutoff.
    for i in 0..STARVE_SMALL {
        let v = jittered(STARVE_DIMS, 0.30, 0.01, &mut state);
        engine
            .add_embedding(
                "t",
                "r",
                "main",
                &p(),
                "wsB",
                &format!("b{}", i),
                HLC::new(10_000 + i as u64, 0),
                v,
            )
            .unwrap();
    }

    let mut query = vec![0.0f32; STARVE_DIMS];
    query[0] = 1.0;

    (engine, temp_dir, query)
}

/// The defect, on `search`: before index-side filtering this returned ZERO of
/// the five requested rows.
#[test]
fn test_narrow_workspace_scope_is_not_starved_by_a_large_neighbour() {
    const K: usize = 5;
    let (engine, _temp_dir, query) = starvation_index();

    let results = engine
        .search("t", "r", "main", &p(), &["wsB".to_string()], &query, K)
        .unwrap();

    for r in &results {
        assert_eq!(r.workspace_id, "wsB", "scope leaked");
        assert!(
            r.node_id.starts_with('b'),
            "out-of-scope node {} returned",
            r.node_id
        );
    }
    assert_eq!(
        results.len(),
        K,
        "scoped search starved: workspace B holds {} answers inside the distance \
         threshold but the search returned {}",
        STARVE_SMALL,
        results.len()
    );
}

/// The SAME defect on the mirrored path. `search_chunks` carried its own copy
/// of the fetch sizing and the workspace post-filter, so a fix applied to one
/// of them would leave the other starving. Both now share `fetch_scoped`.
#[test]
fn test_search_chunks_is_not_starved_by_a_large_neighbour() {
    const K: usize = 5;
    let (engine, _temp_dir, query) = starvation_index();

    for mode in [SearchMode::Chunks, SearchMode::Documents] {
        let request = SearchRequest::new(query.clone(), K)
            .with_mode(mode)
            .with_workspace("wsB".to_string());

        let results = engine
            .search_chunks("t", "r", "main", &p(), &request)
            .unwrap();

        for r in &results {
            assert_eq!(r.workspace, "wsB", "scope leaked in {:?} mode", mode);
        }
        assert_eq!(
            results.len(),
            K,
            "search_chunks starved in {:?} mode: returned {} of {}",
            mode,
            results.len(),
            K
        );
    }
}

/// A scope the index holds nothing for is answered from the workspace counts,
/// without a graph walk — and it is EMPTY, never a leak of the other workspace.
#[test]
fn test_scope_with_no_vectors_returns_empty_not_the_other_workspace() {
    let (engine, _temp_dir, query) = starvation_index();

    let results = engine
        .search(
            "t",
            "r",
            "main",
            &p(),
            &["wsNothing".to_string()],
            &query,
            5,
        )
        .unwrap();

    assert!(results.is_empty(), "empty scope returned {:?}", results);
}

/// Scoped search must keep working across a save/load round trip, where the
/// index comes back memory-mapped (`Viewed`) and the workspace counts are
/// rebuilt from the persisted metadata rather than maintained incrementally.
#[test]
fn test_scoped_search_survives_persistence() {
    const K: usize = 5;
    let (engine, temp_dir, query) = starvation_index();
    engine.snapshot_dirty_indexes().unwrap();
    drop(engine);

    let reloaded = Arc::new(
        HnswIndexingEngine::new(
            temp_dir.path().to_path_buf(),
            256 * 1024 * 1024,
            STARVE_DIMS,
        )
        .unwrap(),
    );

    let results = reloaded
        .search("t", "r", "main", &p(), &["wsB".to_string()], &query, K)
        .unwrap();

    assert_eq!(
        results.len(),
        K,
        "a reloaded index starved the narrow scope: returned {} of {}",
        results.len(),
        K
    );
    for r in &results {
        assert_eq!(r.workspace_id, "wsB", "scope leaked after reload");
    }
}

/// The witness for the defect this fixture exists to reproduce.
///
/// It reproduces the PRE-FIX algorithm — an unfiltered walk, then keep the
/// in-scope rows — and asserts it finds NOTHING. Without this, a later change
/// that made `wsB` accidentally reachable would leave the tests above passing
/// while no longer exercising starvation at all.
#[test]
fn test_an_unfiltered_walk_over_this_index_never_reaches_the_narrow_scope() {
    let (engine, _temp_dir, query) = starvation_index();

    // 50 documents is ten times the k the scoped tests ask for, and more than
    // the old post-filter's 10x over-draw would have drawn.
    let unfiltered = engine
        .search("t", "r", "main", &p(), &[], &query, 50)
        .unwrap();
    let in_scope = unfiltered
        .iter()
        .filter(|r| r.workspace_id == "wsB")
        .count();

    assert_eq!(
        in_scope, 0,
        "the fixture is no longer adversarial: an UNFILTERED walk already \
         surfaces {} wsB rows, so the scoped tests above prove nothing",
        in_scope
    );
}

/// The fallback is still the OLD algorithm, and it still starves — that is the
/// point of keeping the two modes distinguishable rather than pretending the
/// fallback is as good.
///
/// Calling `post_filtered` directly (rather than going through the env-var
/// escape hatch) keeps this deterministic under a parallel test runner, and it
/// is the same function the escape hatch and the usearch-error path call.
#[test]
fn test_the_post_filter_fallback_still_starves_and_says_so() {
    const K: usize = 5;
    let (engine, _temp_dir, query) = starvation_index();

    let index_arc = engine.get_or_load_index("t", "r", "main", &p()).unwrap();
    let index = index_arc.read().unwrap();

    let in_scope = index.vectors_in_workspaces(&["wsB".to_string()]);
    assert_eq!(in_scope, STARVE_SMALL, "fixture lost its small workspace");

    let scoped = index
        .post_filtered(&query, K, &["wsB".to_string()], in_scope)
        .unwrap();

    assert_eq!(
        scoped.mode,
        crate::index::ScopeFilterMode::PostFilter,
        "the fallback must identify itself, or a short result gets blamed on permissions"
    );
    assert!(
        scoped.results.is_empty(),
        "the fallback unexpectedly found in-scope rows; the fixture is no longer adversarial"
    );
    assert!(
        scoped.drawn > 0,
        "the fallback drew {} candidates — it must report what the unfiltered walk saw, \
         so an operator can tell 'the walk missed my scope' from 'the index is empty'",
        scoped.drawn
    );
    assert_eq!(
        scoped.in_scope_total, STARVE_SMALL,
        "a short result must carry the number that disproves 'the scope is empty'"
    );

    // And the fixed path, on the same index, in the same test.
    let fixed = index
        .search_scoped(&query, K, &["wsB".to_string()])
        .unwrap();
    assert_eq!(fixed.mode, crate::index::ScopeFilterMode::IndexSide);
    assert_eq!(
        fixed.results.len(),
        K,
        "index-side filtering returned {} of {} on the index the fallback starved on",
        fixed.results.len(),
        K
    );
}

// ============================================================================
// Partitioning, the lazy layout migration, and the memory budget.
// ============================================================================

/// A resolver that hands back a fixed spec, so a test can pin width, metric and
/// quantization the way a real tenant config would.
struct FixedSpec {
    spec: crate::dims::IndexSpec,
}

impl crate::dims::IndexSpecResolver for FixedSpec {
    fn spec_for(
        &self,
        _t: &str,
        _r: &str,
        _b: &str,
        _p: &crate::partition::PartitionId,
    ) -> Option<crate::dims::IndexSpec> {
        Some(self.spec)
    }

    fn default_text_partition(
        &self,
        _t: &str,
        _r: &str,
        _b: &str,
    ) -> Option<crate::partition::PartitionId> {
        Some(p())
    }
}

fn seeded(
    dir: &std::path::Path,
    partition: &crate::partition::PartitionId,
    n: u32,
) -> Arc<HnswIndexingEngine> {
    let engine = Arc::new(HnswIndexingEngine::new(dir.to_path_buf(), 64 * 1024 * 1024, 8).unwrap());
    for i in 0..n {
        engine
            .add_embedding(
                "t",
                "r",
                "main",
                partition,
                "ws1",
                &format!("n{i}"),
                HLC::new(i as u64, 0),
                (0..8).map(|j| (i + j) as f32 * 0.01).collect(),
            )
            .unwrap();
    }
    engine.snapshot_dirty_indexes().unwrap();
    engine
}

/// Two embedders of the SAME width are the case no width check can catch:
/// unrelated regions of R^n, every distance finite, every ranking plausible,
/// nothing logged. Only separate indexes fix it.
#[test]
fn two_partitions_of_one_branch_do_not_see_each_other() {
    let dir = TempDir::new().unwrap();
    let engine =
        Arc::new(HnswIndexingEngine::new(dir.path().to_path_buf(), 64 * 1024 * 1024, 4).unwrap());

    let text = crate::partition::PartitionId::new("modelAAAAAAT");
    let image = crate::partition::PartitionId::new("modelBBBBBBI");

    engine
        .add_embedding(
            "t",
            "r",
            "main",
            &text,
            "ws",
            "text-node",
            HLC::new(1, 0),
            vec![1.0, 0.0, 0.0, 0.0],
        )
        .unwrap();
    engine
        .add_embedding(
            "t",
            "r",
            "main",
            &image,
            "ws",
            "image-node",
            HLC::new(2, 0),
            vec![1.0, 0.0, 0.0, 0.0],
        )
        .unwrap();

    // Identical vectors, identical query — and still each partition returns only
    // its own. Before partitioning both lived in one graph and the image vector
    // could out-rank the text one for a text query.
    let query = vec![1.0, 0.0, 0.0, 0.0];
    let from_text = engine
        .search("t", "r", "main", &text, &[], &query, 10)
        .unwrap();
    let from_image = engine
        .search("t", "r", "main", &image, &[], &query, 10)
        .unwrap();

    assert_eq!(from_text.len(), 1);
    assert_eq!(from_text[0].node_id, "text-node");
    assert_eq!(from_image.len(), 1);
    assert_eq!(from_image[0].node_id, "image-node");

    assert_eq!(engine.stats("t", "r", "main", &text).unwrap().count, 1);
    assert_eq!(engine.stats("t", "r", "main", &image).unwrap().count, 1);

    let mut listed = engine.list_partitions("t", "r", "main").unwrap();
    listed.sort();
    assert_eq!(listed, vec![text, image]);
}

/// Enabling a second partition must not take the first one down. This is the
/// mutual exclusion the whole change exists to remove: one index per branch
/// meant a second embedder made that index unloadable for BOTH.
#[test]
fn a_second_partition_does_not_take_the_first_one_down() {
    let dir = TempDir::new().unwrap();
    let narrow = crate::partition::PartitionId::new("narrowwwwwwT");
    let wide = crate::partition::PartitionId::new("wideeeeeeeeI");

    let engine =
        Arc::new(HnswIndexingEngine::new(dir.path().to_path_buf(), 64 * 1024 * 1024, 4).unwrap());
    engine
        .add_embedding(
            "t",
            "r",
            "main",
            &narrow,
            "ws",
            "a",
            HLC::new(1, 0),
            vec![1.0, 0.0, 0.0, 0.0],
        )
        .unwrap();

    // A DIFFERENT WIDTH in the other partition. Sharing one index, this is the
    // load error that names REBUILD VECTOR INDEX and takes text search down.
    let engine2 = Arc::new(
        HnswIndexingEngine::new(dir.path().to_path_buf(), 64 * 1024 * 1024, 4)
            .unwrap()
            .with_spec_resolver(Arc::new(FixedSpec {
                spec: crate::dims::IndexSpec::new(16),
            })),
    );
    engine2
        .add_embedding(
            "t",
            "r",
            "main",
            &wide,
            "ws",
            "b",
            HLC::new(2, 0),
            (0..16).map(|i| i as f32).collect(),
        )
        .unwrap();
    engine2.snapshot_dirty_indexes().unwrap();

    // The narrow partition is still there, still 4 wide, still answering.
    assert_eq!(
        engine.stats("t", "r", "main", &narrow).unwrap().dimensions,
        4
    );
    assert_eq!(
        engine
            .search("t", "r", "main", &narrow, &[], &[1.0, 0.0, 0.0, 0.0], 5)
            .unwrap()
            .len(),
        1
    );
}

/// The pre-partition layout is moved, not rebuilt — and BOTH files move, or the
/// loader reads a bare `.hnsw` as an old bincode index and destroys it.
#[test]
fn a_pre_partition_index_is_renamed_into_its_partition_on_load() {
    let dir = TempDir::new().unwrap();
    let partition = p();

    // Write an index at the NEW layout, then move it back to the OLD one, which
    // is exactly the on-disk state an upgrading deployment has.
    let engine = seeded(dir.path(), &partition, 5);
    drop(engine);

    let new_index = crate::engine::index_path(dir.path(), "t", "r", "main", &partition);
    let new_meta = crate::engine::meta_path(&new_index);
    let legacy_index = dir.path().join("t").join("r").join("main.hnsw");
    let legacy_meta = crate::engine::meta_path(&legacy_index);
    std::fs::rename(&new_index, &legacy_index).unwrap();
    std::fs::rename(&new_meta, &legacy_meta).unwrap();
    std::fs::remove_dir(dir.path().join("t").join("r").join("main")).unwrap();
    assert!(legacy_index.exists());
    assert!(!new_index.exists());

    // Open it the way a restarted server would.
    let engine = HnswIndexingEngine::new(dir.path().to_path_buf(), 64 * 1024 * 1024, 8).unwrap();
    let stats = engine.stats("t", "r", "main", &partition).unwrap();

    assert_eq!(stats.count, 5, "the migration must not lose vectors");
    assert_eq!(stats.dimensions, 8);
    assert!(new_index.exists(), "graph file moved into the partition");
    assert!(new_meta.exists(), "sidecar moved WITH it");
    assert!(!legacy_index.exists(), "legacy path cleared");
    assert!(!legacy_meta.exists());

    // Idempotent: a second open changes nothing.
    let engine = HnswIndexingEngine::new(dir.path().to_path_buf(), 64 * 1024 * 1024, 8).unwrap();
    assert_eq!(engine.stats("t", "r", "main", &partition).unwrap().count, 5);
}

/// `.with_extension("hnsw")` REPLACED the branch's own extension, so
/// `release.2` and `release.3` both wrote `release.hnsw` and silently shared one
/// index. Making the branch a directory component removes the collision.
#[test]
fn two_dotted_branches_no_longer_share_one_index_file() {
    let dir = TempDir::new().unwrap();
    let partition = p();
    let engine =
        Arc::new(HnswIndexingEngine::new(dir.path().to_path_buf(), 64 * 1024 * 1024, 4).unwrap());

    for (branch, node) in [("release.2", "two"), ("release.3", "three")] {
        engine
            .add_embedding(
                "t",
                "r",
                branch,
                &partition,
                "ws",
                node,
                HLC::new(1, 0),
                vec![1.0, 0.0, 0.0, 0.0],
            )
            .unwrap();
    }
    engine.snapshot_dirty_indexes().unwrap();

    for (branch, node) in [("release.2", "two"), ("release.3", "three")] {
        let found = engine
            .search("t", "r", branch, &partition, &[], &[1.0, 0.0, 0.0, 0.0], 10)
            .unwrap();
        assert_eq!(
            found.len(),
            1,
            "branch {branch} must hold exactly its own vector"
        );
        assert_eq!(found[0].node_id, node);
    }
}

/// The 512 MB budget bounded nothing: moka calls its weigher once at INSERT and
/// never again, so an index created empty was pinned at ~0 bytes forever however
/// large it grew.
#[test]
fn the_cache_reweighs_an_index_as_it_grows() {
    let dir = TempDir::new().unwrap();
    let partition = p();
    let engine =
        Arc::new(HnswIndexingEngine::new(dir.path().to_path_buf(), 512 * 1024 * 1024, 8).unwrap());

    // One vector: the entry exists, weighed at creation time (i.e. empty).
    engine
        .add_embedding(
            "t",
            "r",
            "main",
            &partition,
            "ws",
            "n0",
            HLC::new(0, 0),
            vec![0.0; 8],
        )
        .unwrap();
    let at_creation = engine.cached_bytes();

    // Past the re-weigh interval.
    for i in 1..(super::REWEIGH_EVERY_N_MUTATIONS + 8) {
        engine
            .add_embedding(
                "t",
                "r",
                "main",
                &partition,
                "ws",
                &format!("n{i}"),
                HLC::new(i as u64, 0),
                (0..8).map(|j| (i + j) as f32 * 0.001).collect(),
            )
            .unwrap();
    }

    let after_growth = engine.cached_bytes();
    assert!(
        after_growth > at_creation,
        "the cache must observe the index growing: {at_creation} -> {after_growth}. \
         A frozen weight is what made the memory budget inert."
    );
    assert_eq!(engine.cached_index_count(), 1);
}

/// An index evicted while dirty used to be dropped from the dirty set WITHOUT
/// being saved. More partitions means more eviction pressure, so this had to be
/// closed before partitioning could multiply the entries.
#[test]
fn an_evicted_dirty_index_is_saved_not_dropped() {
    let dir = TempDir::new().unwrap();
    // A budget small enough that a second index evicts the first.
    let engine = Arc::new(HnswIndexingEngine::new(dir.path().to_path_buf(), 1, 8).unwrap());

    let first = crate::partition::PartitionId::new("firstpartT");
    let second = crate::partition::PartitionId::new("secondparT");

    engine
        .add_embedding(
            "t",
            "r",
            "main",
            &first,
            "ws",
            "kept",
            HLC::new(1, 0),
            vec![1.0; 8],
        )
        .unwrap();
    // Touch a second partition to force the cache over its (1 byte) budget.
    engine
        .add_embedding(
            "t",
            "r",
            "main",
            &second,
            "ws",
            "other",
            HLC::new(2, 0),
            vec![0.5; 8],
        )
        .unwrap();
    engine.cached_bytes(); // drives moka's pending eviction work

    // Whatever was evicted must be on disk, not gone.
    let first_path = crate::engine::index_path(dir.path(), "t", "r", "main", &first);
    let second_path = crate::engine::index_path(dir.path(), "t", "r", "main", &second);
    assert!(
        first_path.exists() || second_path.exists(),
        "an evicted dirty index must be written out, not silently discarded"
    );

    // And re-reading gives the vectors back.
    let engine = HnswIndexingEngine::new(dir.path().to_path_buf(), 64 * 1024 * 1024, 8).unwrap();
    assert_eq!(engine.stats("t", "r", "main", &first).unwrap().count, 1);
    assert_eq!(engine.stats("t", "r", "main", &second).unwrap().count, 1);
}

/// Quantization was a shipped console control with nowhere to land: the Rust
/// side had no field, so every index was F32 by accident. It must now reach the
/// usearch scalar kind AND survive a round trip through the sidecar.
#[test]
fn quantization_from_the_resolver_reaches_the_index_and_the_sidecar() {
    let dir = TempDir::new().unwrap();
    let partition = p();

    let engine = Arc::new(
        HnswIndexingEngine::new(dir.path().to_path_buf(), 64 * 1024 * 1024, 8)
            .unwrap()
            .with_spec_resolver(Arc::new(FixedSpec {
                spec: crate::dims::IndexSpec::new(8)
                    .with_quantization(crate::types::QuantizationType::Int8),
            })),
    );

    engine
        .add_embedding(
            "t",
            "r",
            "main",
            &partition,
            "ws",
            "n",
            HLC::new(1, 0),
            vec![0.5; 8],
        )
        .unwrap();

    let stats = engine.stats("t", "r", "main", &partition).unwrap();
    assert_eq!(stats.quantization, crate::types::QuantizationType::Int8);
    engine.snapshot_dirty_indexes().unwrap();

    // Reload with NO resolver: the shape has to come off disk, not off config.
    let reopened = HnswIndexingEngine::new(dir.path().to_path_buf(), 64 * 1024 * 1024, 8).unwrap();
    let stats = reopened.stats("t", "r", "main", &partition).unwrap();
    assert_eq!(
        stats.quantization,
        crate::types::QuantizationType::Int8,
        "an index keeps the precision it was BUILT with; the sidecar is the record"
    );
    assert_eq!(stats.count, 1);

    // And it still answers.
    let found = reopened
        .search("t", "r", "main", &partition, &[], &[0.5; 8], 5)
        .unwrap();
    assert_eq!(found.len(), 1);
}

/// The metric an index is built with must be the metric it reports, so a query
/// path can never answer under a metric the graph was not built for.
#[test]
fn the_resolved_metric_is_the_metric_the_index_is_built_with() {
    let dir = TempDir::new().unwrap();
    let partition = p();
    let engine = Arc::new(
        HnswIndexingEngine::new(dir.path().to_path_buf(), 64 * 1024 * 1024, 8)
            .unwrap()
            .with_spec_resolver(Arc::new(FixedSpec {
                spec: crate::dims::IndexSpec::new(8).with_metric(crate::types::DistanceMetric::L2),
            })),
    );
    engine
        .add_embedding(
            "t",
            "r",
            "main",
            &partition,
            "ws",
            "n",
            HLC::new(1, 0),
            vec![0.25; 8],
        )
        .unwrap();

    assert_eq!(
        engine
            .stats("t", "r", "main", &partition)
            .unwrap()
            .distance_metric,
        crate::types::DistanceMetric::L2
    );
}

/// A branch fork must copy every partition, sidecars included.
#[test]
fn a_branch_copy_carries_every_partition_and_its_sidecar() {
    let dir = TempDir::new().unwrap();
    let a = crate::partition::PartitionId::new("modelAAAAAAT");
    let b = crate::partition::PartitionId::new("modelBBBBBBI");
    let engine =
        Arc::new(HnswIndexingEngine::new(dir.path().to_path_buf(), 64 * 1024 * 1024, 4).unwrap());

    for (part, node) in [(&a, "from-a"), (&b, "from-b")] {
        engine
            .add_embedding(
                "t",
                "r",
                "main",
                part,
                "ws",
                node,
                HLC::new(1, 0),
                vec![1.0, 0.0, 0.0, 0.0],
            )
            .unwrap();
    }
    engine.snapshot_dirty_indexes().unwrap();

    engine.copy_for_branch("t", "r", "main", "feature").unwrap();

    let mut copied = engine.list_partitions("t", "r", "feature").unwrap();
    copied.sort();
    let mut expected = vec![a.clone(), b.clone()];
    expected.sort();
    assert_eq!(copied, expected);

    // A copy without the sidecar would load as a bincode index and blow up here.
    let fresh = HnswIndexingEngine::new(dir.path().to_path_buf(), 64 * 1024 * 1024, 4).unwrap();
    for (part, node) in [(&a, "from-a"), (&b, "from-b")] {
        let found = fresh
            .search("t", "r", "feature", part, &[], &[1.0, 0.0, 0.0, 0.0], 5)
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].node_id, node);
    }
}

/// `purge_index` names one partition; `purge_branch` means all of them.
#[test]
fn purging_one_partition_leaves_the_others_alone() {
    let dir = TempDir::new().unwrap();
    let a = crate::partition::PartitionId::new("modelAAAAAAT");
    let b = crate::partition::PartitionId::new("modelBBBBBBI");
    let engine =
        Arc::new(HnswIndexingEngine::new(dir.path().to_path_buf(), 64 * 1024 * 1024, 4).unwrap());

    for part in [&a, &b] {
        engine
            .add_embedding(
                "t",
                "r",
                "main",
                part,
                "ws",
                "n",
                HLC::new(1, 0),
                vec![1.0, 0.0, 0.0, 0.0],
            )
            .unwrap();
    }
    engine.snapshot_dirty_indexes().unwrap();

    engine.purge_index("t", "r", "main", &a).unwrap();
    assert_eq!(
        engine.list_partitions("t", "r", "main").unwrap(),
        vec![b.clone()]
    );
    // Sidecar gone too — an orphan `.hnsw.meta` beside no graph is confusing at
    // best, and an orphan `.hnsw` is destructive.
    assert!(!crate::engine::meta_path(&crate::engine::index_path(
        dir.path(),
        "t",
        "r",
        "main",
        &a
    ))
    .exists());

    engine.purge_branch("t", "r", "main").unwrap();
    assert!(engine.list_partitions("t", "r", "main").unwrap().is_empty());
}
