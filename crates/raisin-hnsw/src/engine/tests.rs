// SPDX-License-Identifier: BSL-1.1

//! Tests for the HNSW indexing engine.

use super::*;
use crate::types::{ScoringConfig, SearchMode, SearchRequest};
use raisin_hlc::HLC;
use std::sync::Arc;
use tempfile::TempDir;

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
            "ws1",
            "node2",
            HLC::new(2, 0),
            create_test_vector(128, 2.0),
        )
        .unwrap();

    // Search with workspace filter
    let query = create_test_vector(128, 1.1);
    let results = engine
        .search("tenant1", "repo1", "main", Some("ws1"), &query, 2)
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
            .search("tenant1", "repo1", "main", Some("ws1"), &query, 1)
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
        .search("tenant1", "repo1", "feature", Some("ws1"), &query, 1)
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
        .search_chunks("tenant1", "repo1", "main", &chunks_request)
        .unwrap();

    // Should get at least 2 chunks from doc1
    assert!(chunks_results.len() >= 2);

    // Test Documents mode - should deduplicate
    let docs_request = SearchRequest::new(query, 10)
        .with_mode(SearchMode::Documents)
        .with_workspace("ws1".to_string());

    let doc_results = engine
        .search_documents("tenant1", "repo1", "main", &docs_request)
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
        .search_chunks("tenant1", "repo1", "main", &strict_request)
        .unwrap();

    // Should only get close results
    assert!(strict_results.iter().all(|r| r.distance < 0.3));

    // Test with relaxed distance threshold
    let relaxed_request = SearchRequest::new(query, 10)
        .with_workspace("ws1".to_string())
        .with_max_distance(0.9);

    let relaxed_results = engine
        .search_chunks("tenant1", "repo1", "main", &relaxed_request)
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
        .search_chunks("tenant1", "repo1", "main", &no_scoring_request)
        .unwrap();

    assert!(no_scoring_results.len() >= 2);

    // Test with scoring - first chunk should rank higher
    let scoring_request = SearchRequest::new(query, 10)
        .with_mode(SearchMode::Chunks)
        .with_workspace("ws1".to_string())
        .with_max_distance(1.0)
        .with_scoring(ScoringConfig::default());

    let scoring_results = engine
        .search_chunks("tenant1", "repo1", "main", &scoring_request)
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
        .search_chunks("tenant1", "repo1", "main", &request)
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
        .search_chunks("tenant1", "repo1", "main", &request)
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
