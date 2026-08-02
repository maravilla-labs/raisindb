//! Tests for embedding storage and job store implementations.

use super::*;
use crate::cf;
use chrono::Utc;
use raisin_embeddings::{
    EmbeddingData, EmbeddingJob, EmbeddingJobStore, EmbeddingProvider, EmbeddingStorage,
};
use raisin_hlc::HLC;
use rocksdb::DB;
use std::sync::Arc;

fn create_test_db() -> Arc<DB> {
    let path = tempfile::tempdir().unwrap();
    let mut opts = rocksdb::Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);

    let cfs = vec![cf::EMBEDDINGS, cf::EMBEDDING_JOBS];
    Arc::new(DB::open_cf(&opts, path.path(), cfs).unwrap())
}

fn create_test_embedding() -> EmbeddingData {
    let embedder_id = raisin_ai::config::EmbedderId::new("openai", "test-model", 3);

    #[allow(deprecated)]
    EmbeddingData {
        vector: vec![0.1, 0.2, 0.3],
        embedder_id,
        embedding_kind: raisin_ai::config::EmbeddingKind::Text,
        source_id: "node1".to_string(),
        chunk_index: 0,
        total_chunks: 1,
        chunk_content: Some("test content".to_string()),
        generated_at: Utc::now(),
        text_hash: 12345,
        model: "test-model".to_string(),
        provider: EmbeddingProvider::OpenAI,
    }
}

#[test]
fn test_store_and_get_embedding() {
    let db = create_test_db();
    let storage = RocksDBEmbeddingStorage::new(db);

    let embedding = create_test_embedding();
    let revision = HLC::new(42, 0);

    // Store embedding
    storage
        .store_embedding(
            "tenant1", "repo1", "main", "ws1", "node1", &revision, &embedding,
        )
        .unwrap();

    // Get specific revision
    let retrieved = storage
        .get_embedding("tenant1", "repo1", "main", "ws1", "node1", Some(&revision))
        .unwrap()
        .unwrap();

    assert_eq!(retrieved.vector, embedding.vector);
    assert_eq!(retrieved.model, embedding.model);

    // Get latest revision (should be revision 42)
    let latest = storage
        .get_embedding("tenant1", "repo1", "main", "ws1", "node1", None)
        .unwrap()
        .unwrap();

    assert_eq!(latest.vector, embedding.vector);
}

#[test]
fn test_revision_ordering() {
    let db = create_test_db();
    let storage = RocksDBEmbeddingStorage::new(db);

    let embedder_id = raisin_ai::config::EmbedderId::new("openai", "test-model", 3);

    #[allow(deprecated)]
    let mut embedding1 = EmbeddingData {
        vector: vec![1.0, 1.0, 1.0],
        embedder_id: embedder_id.clone(),
        embedding_kind: raisin_ai::config::EmbeddingKind::Text,
        source_id: "node1".to_string(),
        chunk_index: 0,
        total_chunks: 1,
        chunk_content: Some("test content 1".to_string()),
        generated_at: Utc::now(),
        text_hash: 12345,
        model: "test-model".to_string(),
        provider: EmbeddingProvider::OpenAI,
    };

    #[allow(deprecated)]
    let mut embedding2 = EmbeddingData {
        vector: vec![2.0, 2.0, 2.0],
        embedder_id: embedder_id.clone(),
        embedding_kind: raisin_ai::config::EmbeddingKind::Text,
        source_id: "node1".to_string(),
        chunk_index: 0,
        total_chunks: 1,
        chunk_content: Some("test content 2".to_string()),
        generated_at: Utc::now(),
        text_hash: 12345,
        model: "test-model".to_string(),
        provider: EmbeddingProvider::OpenAI,
    };

    let revision10 = HLC::new(10, 0);
    let revision20 = HLC::new(20, 0);

    // Store older revision first
    storage
        .store_embedding(
            "tenant1",
            "repo1",
            "main",
            "ws1",
            "node1",
            &revision10,
            &embedding1,
        )
        .unwrap();

    // Store newer revision
    storage
        .store_embedding(
            "tenant1",
            "repo1",
            "main",
            "ws1",
            "node1",
            &revision20,
            &embedding2,
        )
        .unwrap();

    // Get latest should return revision 20
    let latest = storage
        .get_embedding("tenant1", "repo1", "main", "ws1", "node1", None)
        .unwrap()
        .unwrap();

    assert_eq!(latest.vector, vec![2.0, 2.0, 2.0]);

    // Get specific revision
    let rev10 = storage
        .get_embedding(
            "tenant1",
            "repo1",
            "main",
            "ws1",
            "node1",
            Some(&revision10),
        )
        .unwrap()
        .unwrap();

    assert_eq!(rev10.vector, vec![1.0, 1.0, 1.0]);
}

#[test]
fn test_delete_embedding() {
    let db = create_test_db();
    let storage = RocksDBEmbeddingStorage::new(db);

    let embedding = create_test_embedding();
    let revision10 = HLC::new(10, 0);
    let revision20 = HLC::new(20, 0);

    // Store multiple revisions
    storage
        .store_embedding(
            "tenant1",
            "repo1",
            "main",
            "ws1",
            "node1",
            &revision10,
            &embedding,
        )
        .unwrap();
    storage
        .store_embedding(
            "tenant1",
            "repo1",
            "main",
            "ws1",
            "node1",
            &revision20,
            &embedding,
        )
        .unwrap();

    // Delete specific revision
    storage
        .delete_embedding(
            "tenant1",
            "repo1",
            "main",
            "ws1",
            "node1",
            Some(&revision10),
        )
        .unwrap();

    // Revision 10 should be gone
    let rev10 = storage
        .get_embedding(
            "tenant1",
            "repo1",
            "main",
            "ws1",
            "node1",
            Some(&revision10),
        )
        .unwrap();
    assert!(rev10.is_none());

    // Revision 20 should still exist
    let rev20 = storage
        .get_embedding(
            "tenant1",
            "repo1",
            "main",
            "ws1",
            "node1",
            Some(&revision20),
        )
        .unwrap();
    assert!(rev20.is_some());

    // Delete all revisions
    storage
        .delete_embedding("tenant1", "repo1", "main", "ws1", "node1", None)
        .unwrap();

    // All should be gone
    let latest = storage
        .get_embedding("tenant1", "repo1", "main", "ws1", "node1", None)
        .unwrap();
    assert!(latest.is_none());
}

#[test]
fn test_job_lifecycle() {
    let db = create_test_db();
    let job_store = RocksDBEmbeddingJobStore::new(db);

    let job = EmbeddingJob::add_node(
        "tenant1".to_string(),
        "repo1".to_string(),
        "main".to_string(),
        "ws1".to_string(),
        "node1".to_string(),
        HLC::new(42, 0),
    );

    // Enqueue
    job_store.enqueue(&job).unwrap();

    // Count pending
    assert_eq!(job_store.count_pending().unwrap(), 1);

    // Dequeue
    let jobs = job_store.dequeue(10).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].job_id, job.job_id);

    // Should be 0 pending now
    assert_eq!(job_store.count_pending().unwrap(), 0);

    // Complete
    job_store.complete(&[job.job_id.clone()]).unwrap();

    // Get should return None
    let retrieved = job_store.get(&job.job_id).unwrap();
    assert!(retrieved.is_none());
}

/// An embedding for `source_id`, so a test can seed several distinct nodes.
fn embedding_for(source_id: &str, vector: Vec<f32>) -> EmbeddingData {
    EmbeddingData {
        source_id: source_id.to_string(),
        vector,
        ..create_test_embedding()
    }
}

/// THE contract every caller depends on: what `list_embeddings` returns must be
/// feedable straight back into `get_embedding`.
///
/// `list_embeddings` used to read the source id out of key part 4 with its own
/// hand-rolled split. That is the node_id slot in the LEGACY key layout, but in
/// the v2 layout `store_embedding` actually writes, part 4 is the EMBEDDER HASH.
/// So it returned a list of embedder hashes, and this round trip returned `None`
/// for every entry.
///
/// That is not a cosmetic bug: `management/vector.rs` rebuilds the HNSW index by
/// walking exactly this list and calling `get_embedding` on each id, so the
/// rebuild re-added NOTHING and left an empty index behind — silently, since a
/// miss is indistinguishable from "no embedding for this node".
#[test]
fn listed_ids_can_be_fetched_back() {
    let db = create_test_db();
    let storage = RocksDBEmbeddingStorage::new(db);
    let revision = HLC::now();

    for (id, vector) in [
        ("node-a", vec![0.1, 0.2, 0.3]),
        ("node-b", vec![0.4, 0.5, 0.6]),
    ] {
        storage
            .store_embedding("t", "r", "main", "ws", id, &revision, &embedding_for(id, vector))
            .unwrap();
    }

    let listed = storage.list_embeddings("t", "r", "main", "ws").unwrap();

    let mut ids: Vec<String> = listed.iter().map(|(id, _)| id.clone()).collect();
    ids.sort();
    assert_eq!(
        ids,
        vec!["node-a".to_string(), "node-b".to_string()],
        "list_embeddings must return NODE IDS — a list of embedder hashes would \
         also be non-empty, which is why the rebuild failed silently",
    );

    for (node_id, revision) in &listed {
        let fetched = storage
            .get_embedding("t", "r", "main", "ws", node_id, Some(revision))
            .unwrap();
        assert!(
            fetched.is_some(),
            "'{node_id}' came out of list_embeddings but get_embedding cannot \
             find it — this is exactly the round trip the HNSW rebuild performs",
        );
    }
}

/// One row per node, not one per embedder.
///
/// The old dedup compared each key with the PREVIOUS one, which only works if
/// every entry for a node is adjacent. Two embedders sort into separate key
/// ranges, so the same node reappears far apart — and because the id being
/// compared was the embedder hash, a whole workspace collapsed to one row per
/// embedder instead.
#[test]
fn a_node_embedded_by_two_embedders_is_listed_once() {
    let db = create_test_db();
    let storage = RocksDBEmbeddingStorage::new(db);
    let revision = HLC::now();

    let mut first = embedding_for("node-a", vec![0.1, 0.2, 0.3]);
    first.embedder_id = raisin_ai::config::EmbedderId::new("openai", "model-one", 3);
    let mut second = embedding_for("node-a", vec![0.4, 0.5, 0.6]);
    second.embedder_id = raisin_ai::config::EmbedderId::new("ollama", "model-two", 3);

    storage
        .store_embedding("t", "r", "main", "ws", "node-a", &revision, &first)
        .unwrap();
    storage
        .store_embedding("t", "r", "main", "ws", "node-a", &revision, &second)
        .unwrap();

    let listed = storage.list_embeddings("t", "r", "main", "ws").unwrap();
    assert_eq!(
        listed.len(),
        1,
        "one node embedded twice must appear once, got {listed:?}",
    );
    assert_eq!(listed[0].0, "node-a");
}
