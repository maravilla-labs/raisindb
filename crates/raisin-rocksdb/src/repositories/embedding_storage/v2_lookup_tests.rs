//! `get_embedding`'s v2 fallback: newest row at or before the revision, found
//! by seeks, with the same answer the whole-workspace scan gave.

use super::*;
use crate::cf;
use chrono::Utc;
use raisin_ai::config::{EmbedderId, EmbeddingKind};
use raisin_embeddings::{EmbeddingData, EmbeddingProvider, EmbeddingStorage};
use raisin_hlc::HLC;
use rocksdb::DB;
use std::sync::Arc;

fn storage() -> (RocksDBEmbeddingStorage, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut opts = rocksdb::Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);
    let db = DB::open_cf(&opts, dir.path(), vec![cf::EMBEDDINGS, cf::EMBEDDING_JOBS]).unwrap();
    (RocksDBEmbeddingStorage::new(Arc::new(db)), dir)
}

fn row(source_id: &str, model: &str, kind: EmbeddingKind, chunk: usize, v: f32) -> EmbeddingData {
    #[allow(deprecated)]
    EmbeddingData {
        vector: vec![v, v, v],
        embedder_id: EmbedderId::new("openai", model, 3),
        embedding_kind: kind,
        source_id: source_id.to_string(),
        chunk_index: chunk,
        total_chunks: 2,
        chunk_content: None,
        generated_at: Utc::now(),
        text_hash: 1,
        spec_hash: Some(1),
        chunk_span: None,
        model: model.to_string(),
        provider: EmbeddingProvider::OpenAI,
    }
}

fn put(storage: &RocksDBEmbeddingStorage, data: &EmbeddingData, revision: u64) {
    storage
        .store_embedding(
            "t",
            "r",
            "main",
            "ws",
            &data.source_id,
            &HLC::new(revision, 0),
            data,
        )
        .unwrap();
}

fn get(storage: &RocksDBEmbeddingStorage, node: &str, revision: Option<u64>) -> Option<f32> {
    let revision = revision.map(|r| HLC::new(r, 0));
    storage
        .get_embedding("t", "r", "main", "ws", node, revision.as_ref())
        .unwrap()
        .map(|data| data.vector[0])
}

/// A node read at a revision LATER than the one its vector was computed at
/// must still find the vector. The fallback used to demand an exact revision
/// match, which is the NULL `SELECT embedding` column.
#[test]
fn a_read_at_a_later_revision_finds_the_newest_vector_at_or_before_it() {
    let (storage, _dir) = storage();
    for (revision, v) in [(10, 1.0), (20, 2.0), (30, 3.0)] {
        put(
            &storage,
            &row("n", "m", EmbeddingKind::Text, 0, v),
            revision,
        );
    }
    // Distractors on both sides of the source in key order.
    put(&storage, &row("a", "m", EmbeddingKind::Text, 0, 9.0), 25);
    put(&storage, &row("z", "m", EmbeddingKind::Text, 0, 9.0), 25);

    assert_eq!(get(&storage, "n", Some(5)), None);
    assert_eq!(get(&storage, "n", Some(10)), Some(1.0));
    assert_eq!(get(&storage, "n", Some(25)), Some(2.0));
    assert_eq!(get(&storage, "n", Some(30)), Some(3.0));
    assert_eq!(get(&storage, "n", Some(1_000)), Some(3.0));
    assert_eq!(get(&storage, "n", None), Some(3.0));
    assert_eq!(get(&storage, "missing", None), None);
    assert_eq!(get(&storage, "missing", Some(25)), None);
}

/// Across partitions the answer is still the first row in KEY order —
/// partition `(embedder_hash, kind)` first — as the full scan answered it.
#[test]
fn the_first_partition_in_key_order_answers() {
    let (storage, _dir) = storage();
    let models = ["model-a", "model-b", "model-c"];
    let mut partitions: Vec<(String, char, f32)> = Vec::new();
    for (i, model) in models.iter().enumerate() {
        for kind in [EmbeddingKind::Text, EmbeddingKind::Image] {
            let v = (i * 10) as f32
                + if kind == EmbeddingKind::Text {
                    1.0
                } else {
                    2.0
                };
            put(&storage, &row("n", model, kind, 0, v), 10);
            put(&storage, &row("other", model, kind, 0, 99.0), 10);
            partitions.push((
                EmbedderId::new("openai", *model, 3).to_key_hash(),
                kind.to_key_char(),
                v,
            ));
        }
    }
    partitions.sort_by(|a, b| (a.0.as_str(), a.1).cmp(&(b.0.as_str(), b.1)));
    let expected = partitions[0].2;

    assert_eq!(get(&storage, "n", None), Some(expected));
    assert_eq!(get(&storage, "n", Some(10)), Some(expected));
    assert_eq!(get(&storage, "n", Some(9)), None);
}

/// Within a source the first CHUNK that has a version at or before the
/// revision answers, at its newest such version.
#[test]
fn a_chunk_whose_versions_are_all_too_new_is_skipped() {
    let (storage, _dir) = storage();
    put(&storage, &row("n", "m", EmbeddingKind::Text, 0, 1.0), 30);
    put(&storage, &row("n", "m", EmbeddingKind::Text, 1, 2.0), 10);
    put(&storage, &row("n", "m", EmbeddingKind::Text, 1, 3.0), 15);

    assert_eq!(get(&storage, "n", Some(20)), Some(3.0));
    assert_eq!(get(&storage, "n", Some(12)), Some(2.0));
    assert_eq!(get(&storage, "n", Some(30)), Some(1.0));
    assert_eq!(get(&storage, "n", None), Some(1.0));
}
