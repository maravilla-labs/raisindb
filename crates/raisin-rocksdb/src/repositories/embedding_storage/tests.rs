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
        spec_hash: Some(12345),
        chunk_span: None,
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
        spec_hash: Some(12345),
        chunk_span: None,
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
        spec_hash: Some(12345),
        chunk_span: None,
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
            .store_embedding(
                "t",
                "r",
                "main",
                "ws",
                id,
                &revision,
                &embedding_for(id, vector),
            )
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

/// `list_workspaces` is what lets an administrative operation cover the
/// workspaces that actually hold embeddings instead of naming one by literal.
///
/// Before it existed, `HnswManagement` hardcoded `"staff"` and SQL's
/// `REBUILD VECTOR INDEX` hardcoded `"default"`, while the embedding job writes
/// under whichever workspace the node lives in — so a rebuild was a silent
/// no-op for every other workspace.
#[test]
fn test_list_workspaces_finds_every_workspace_holding_embeddings() {
    let db = create_test_db();
    let storage = RocksDBEmbeddingStorage::new(db);

    let embedding = create_test_embedding();
    let revision = HLC::new(42, 0);

    // Three workspaces on the same branch. "marketing" is deliberately neither
    // of the two literals the old code could name.
    for (workspace, node) in [
        ("default", "node-d"),
        ("staff", "node-s"),
        ("marketing", "node-m"),
    ] {
        storage
            .store_embedding(
                "tenant1", "repo1", "main", workspace, node, &revision, &embedding,
            )
            .unwrap();
    }

    let workspaces = storage.list_workspaces("tenant1", "repo1", "main").unwrap();

    assert_eq!(
        workspaces,
        vec![
            "default".to_string(),
            "marketing".to_string(),
            "staff".to_string()
        ],
        "every workspace holding an embedding must be listed, sorted"
    );
}

/// A workspace with many embeddings is reported once, and enumeration does not
/// leak across branch or tenant.
#[test]
fn test_list_workspaces_dedups_and_respects_scope() {
    let db = create_test_db();
    let storage = RocksDBEmbeddingStorage::new(db);

    let embedding = create_test_embedding();

    // Same workspace, several nodes and several revisions.
    for node in ["n1", "n2", "n3"] {
        for counter in 0..3u64 {
            storage
                .store_embedding(
                    "tenant1",
                    "repo1",
                    "main",
                    "content",
                    node,
                    &HLC::new(100 + counter, counter),
                    &embedding,
                )
                .unwrap();
        }
    }

    // Neighbours that must NOT show up: another branch, another repo, another
    // tenant. Each uses a workspace name unique to it, so a leak is visible.
    storage
        .store_embedding(
            "tenant1",
            "repo1",
            "other-branch",
            "branch-only",
            "n9",
            &HLC::new(1, 0),
            &embedding,
        )
        .unwrap();
    storage
        .store_embedding(
            "tenant1",
            "other-repo",
            "main",
            "repo-only",
            "n9",
            &HLC::new(1, 0),
            &embedding,
        )
        .unwrap();
    storage
        .store_embedding(
            "other-tenant",
            "repo1",
            "main",
            "tenant-only",
            "n9",
            &HLC::new(1, 0),
            &embedding,
        )
        .unwrap();

    let workspaces = storage.list_workspaces("tenant1", "repo1", "main").unwrap();

    assert_eq!(
        workspaces,
        vec!["content".to_string()],
        "nine embeddings in one workspace is one entry, and no neighbour scope leaks in"
    );
}

/// Seed one chunk row exactly as the embedding job writes it: the DOCUMENT's
/// source id, distinguished by the chunk index.
fn store_chunk(
    storage: &RocksDBEmbeddingStorage,
    node_id: &str,
    index: usize,
    total: usize,
    revision: &HLC,
) {
    let data = EmbeddingData {
        chunk_index: index,
        total_chunks: total,
        ..embedding_for(node_id, vec![0.1, 0.2, 0.3])
    };
    storage
        .store_embedding("t", "r", "main", "ws", node_id, revision, &data)
        .unwrap();
}

fn embedder_hash() -> String {
    raisin_ai::config::EmbedderId::new("openai", "test-model", 3).to_key_hash()
}

fn chunk_indexes(storage: &RocksDBEmbeddingStorage, node_id: &str, revision: &HLC) -> Vec<usize> {
    let mut indexes: Vec<usize> = storage
        .list_source_chunks("t", "r", "main", "ws", &embedder_hash(), 'T', node_id)
        .unwrap()
        .into_iter()
        .filter(|(_, rev)| rev == revision)
        .map(|(idx, _)| idx)
        .collect();
    indexes.sort_unstable();
    indexes
}

/// Re-chunking a document into FEWER pieces must not leave the surplus rows of
/// the SAME revision behind, and must not touch the rows of an older one.
///
/// This is the whole distinction the sweep has to get right. The key carries the
/// revision, so an older revision's chunk set is that revision's correct content
/// and is retained by design; a chunk of the revision being written that the new
/// chunking no longer produces is an orphan that will otherwise keep matching
/// queries forever — and be read straight back out of RocksDB by anything that
/// rebuilds from storage.
#[test]
fn the_orphan_sweep_prunes_this_revision_and_spares_history() {
    let db = create_test_db();
    let storage = RocksDBEmbeddingStorage::new(db);
    let hash = embedder_hash();

    let old_revision = HLC::new(100, 0);
    let revision = HLC::new(200, 0);

    // An earlier revision embedded the document in 4 chunks.
    for i in 0..4 {
        store_chunk(&storage, "doc", i, 4, &old_revision);
    }
    // So did an earlier run at THIS revision, before the chunking changed.
    for i in 0..4 {
        store_chunk(&storage, "doc", i, 4, &revision);
    }
    // A neighbouring node id that shares a prefix — deleting it would be data
    // loss.
    store_chunk(&storage, "document", 0, 1, &revision);

    // The new chunking writes 2.
    for i in 0..2 {
        store_chunk(&storage, "doc", i, 2, &revision);
    }

    let pruned = storage
        .prune_chunks_from("t", "r", "main", "ws", &hash, 'T', "doc", 2, &revision)
        .unwrap();

    assert_eq!(
        pruned,
        vec![2, 3],
        "only the surplus chunks of the revision being written are orphans"
    );
    assert_eq!(
        chunk_indexes(&storage, "doc", &revision),
        vec![0, 1],
        "what remains at this revision is exactly the new chunking"
    );
    assert_eq!(
        chunk_indexes(&storage, "doc", &old_revision),
        vec![0, 1, 2, 3],
        "the older revision is that revision's content, not an orphan"
    );
    assert_eq!(
        chunk_indexes(&storage, "document", &revision),
        vec![0],
        "a node id that merely shares a prefix must not be swept"
    );
}

/// `get_chunk` addresses ONE chunk of ONE embedder — neither of which
/// `get_embedding` can express, which is why it answers a chunked document with
/// chunk 0 and a two-model node with whichever row it meets first.
#[test]
fn get_chunk_addresses_one_chunk_of_one_embedder() {
    let db = create_test_db();
    let storage = RocksDBEmbeddingStorage::new(db);
    let revision = HLC::new(100, 0);
    let hash = embedder_hash();

    for i in 0..3 {
        store_chunk(&storage, "doc", i, 3, &revision);
    }

    for i in 0..3 {
        let got = storage
            .get_chunk(
                "t",
                "r",
                "main",
                "ws",
                &hash,
                'T',
                "doc",
                i,
                Some(&revision),
            )
            .unwrap()
            .expect("chunk must be readable at its own index");
        assert_eq!(got.chunk_index, i);
    }
    assert!(
        storage
            .get_chunk(
                "t",
                "r",
                "main",
                "ws",
                &hash,
                'T',
                "doc",
                3,
                Some(&revision)
            )
            .unwrap()
            .is_none(),
        "a chunk index that was never written must read as absent"
    );

    // Another model embedded the same node; asking about ours must not return
    // its vector, and vice versa.
    let other = raisin_ai::config::EmbedderId::new("ollama", "other-model", 3);
    let mut foreign = embedding_for("solo", vec![9.0, 9.0, 9.0]);
    foreign.embedder_id = other.clone();
    storage
        .store_embedding("t", "r", "main", "ws", "solo", &revision, &foreign)
        .unwrap();

    assert!(
        storage
            .get_chunk(
                "t",
                "r",
                "main",
                "ws",
                &hash,
                'T',
                "solo",
                0,
                Some(&revision)
            )
            .unwrap()
            .is_none(),
        "the node is embedded, but not by the model asked about"
    );
    assert!(storage
        .get_chunk(
            "t",
            "r",
            "main",
            "ws",
            &other.to_key_hash(),
            'T',
            "solo",
            0,
            Some(&revision)
        )
        .unwrap()
        .is_some());
}

/// A branch with no embeddings at all enumerates to nothing — not to a
/// hardcoded guess.
#[test]
fn test_list_workspaces_is_empty_when_nothing_is_embedded() {
    let db = create_test_db();
    let storage = RocksDBEmbeddingStorage::new(db);

    let workspaces = storage.list_workspaces("tenant1", "repo1", "main").unwrap();

    assert!(
        workspaces.is_empty(),
        "expected no workspaces, got {workspaces:?}"
    );
}

/// Every scan of this CF must reach EVERY row under its prefix, not just the
/// first — at all three prefix lengths it is scanned at.
///
/// The wanted row is deliberately never the first key under the prefix, so a
/// scan that stops early fails here. That is the failure mode `scan_from`'s
/// explicit seek is defending against (see its doc comment): the day this CF
/// gains a prefix extractor, `prefix_iterator_cf` would cut each of these
/// scans short at a boundary the extractor chose, and all three symptoms are
/// silent —
///
/// * `list_source_chunk_indexes` under-reporting a chunked source as having
///   ONE vector. `VECTOR_OF` reads "unambiguous" out of that count, so a wrong
///   1 is exactly the silent chunk-0 pick the feature exists to refuse;
/// * `get_embedding` answering NULL for every node in a workspace but the one
///   holding the first key;
/// * `list_workspaces` reporting one workspace, so an administrative rebuild
///   skips the rest of the branch.
#[test]
fn a_prefix_scan_reaches_every_row_not_only_the_first() {
    let db = create_test_db();
    let storage = RocksDBEmbeddingStorage::new(db);
    let revision = HLC::new(100, 0);
    let hash = embedder_hash();

    // One chunked source ...
    for i in 0..5 {
        store_chunk(&storage, "chunky", i, 5, &revision);
    }
    // ... and several single-vector sources sharing the workspace prefix, so
    // the wanted row is not the first key under it.
    for node in ["aaa", "bbb", "ccc", "zzz"] {
        store_chunk(&storage, node, 0, 1, &revision);
    }

    assert_eq!(
        storage
            .list_source_chunk_indexes("t", "r", "main", "ws", &hash, 'T', "chunky")
            .unwrap(),
        vec![0, 1, 2, 3, 4],
        "a five-chunk source must report five chunks; reporting 1 is what makes \
         VECTOR_OF believe a chunked document is unambiguous"
    );

    for node in ["aaa", "bbb", "ccc", "zzz", "chunky"] {
        assert!(
            storage
                .get_embedding("t", "r", "main", "ws", node, None)
                .unwrap()
                .is_some(),
            "every node with a stored vector must read one back, not just the \
             one holding the first key in the workspace ({node})"
        );
    }

    // And the same scan one segment higher: several workspaces under a branch.
    for ws in ["alpha", "beta", "gamma"] {
        let data = embedding_for("solo", vec![0.1, 0.2, 0.3]);
        storage
            .store_embedding("t", "r", "main", ws, "solo", &revision, &data)
            .unwrap();
    }
    let mut workspaces = storage.list_workspaces("t", "r", "main").unwrap();
    workspaces.sort();
    assert_eq!(
        workspaces,
        vec![
            "alpha".to_string(),
            "beta".to_string(),
            "gamma".to_string(),
            "ws".to_string()
        ],
        "every workspace holding an embedding must be listed"
    );
}
