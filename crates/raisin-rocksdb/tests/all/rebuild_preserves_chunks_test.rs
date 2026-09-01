// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! `REBUILD VECTOR INDEX` must not eat a chunked document.
//!
//! # The bug this pins
//!
//! The rebuild iterated [`EmbeddingStorage::list_embeddings`], which returns ONE
//! row per source node — deliberately, that being the right answer to "which
//! nodes have embeddings". It then read each one with `get_embedding`, which
//! takes a bare node id and therefore always answers with chunk 0. So a
//! twenty-three-chunk document was re-indexed as a single vector and the other
//! twenty-two were gone: recall collapsed, no error was raised, and `VERIFY`
//! afterwards compared a per-NODE storage count against a per-VECTOR index
//! count and pronounced the wreckage "consistent".
//!
//! Before the fix a rebuild over the corpus below indexed 2 vectors and
//! reported success. It must index 5.
//!
//! No network: the vectors are fabricated and written straight into
//! `cf::EMBEDDINGS`, because what is under test is which ROWS the rebuild
//! visits and under which IDS it files them — not the embedder.

use std::sync::Arc;

use raisin_ai::config::{EmbedderId, EmbeddingKind};
use raisin_embeddings::config::{EmbeddingProvider, TenantEmbeddingConfig};
use raisin_embeddings::{EmbeddingData, EmbeddingStorage, TenantEmbeddingConfigStore};
use raisin_hlc::HLC;
use raisin_hnsw::HnswIndexingEngine;
use raisin_rocksdb::{HnswManagement, RocksDBEmbeddingStorage, RocksDBStorage};
use raisin_storage::{BranchRepository, Storage};
use tempfile::TempDir;

const TENANT: &str = "rebuild-chunks";
const REPO: &str = "repo";
const BRANCH: &str = "main";
const WS: &str = "default";
const DIMS: usize = 8;

/// A short, chunked document and a long, unchunked one — 4 + 1 = 5 vectors
/// across 2 nodes, so a per-node count and a per-vector count cannot be
/// confused for one another by accident.
const CHUNKED_NODE: &str = "nodeChunked1";
const SINGLE_NODE: &str = "nodeSingle01";
const CHUNKS: usize = 4;

fn vector(seed: usize) -> Vec<f32> {
    let raw: Vec<f32> = (0..DIMS)
        .map(|i| 1.0 + (seed as f32) * 0.05 * (i as f32 + 1.0))
        .collect();
    let mag = raw.iter().map(|x| x * x).sum::<f32>().sqrt();
    raw.iter().map(|x| x / mag).collect()
}

fn data(source_id: &str, chunk_index: usize, total_chunks: usize, seed: usize) -> EmbeddingData {
    EmbeddingData {
        vector: vector(seed),
        embedder_id: EmbedderId::new("ollama", "test-model", DIMS),
        embedding_kind: EmbeddingKind::Text,
        source_id: source_id.to_string(),
        chunk_index,
        total_chunks,
        chunk_content: None,
        generated_at: chrono::Utc::now(),
        text_hash: seed as u64,
        spec_hash: Some(1),
        chunk_span: None,
        model: "test-model".to_string(),
        provider: EmbeddingProvider::Ollama,
    }
}

#[tokio::test]
async fn rebuild_reindexes_every_chunk_not_one_per_node() {
    let dir = TempDir::new().expect("temp dir");
    let storage = Arc::new(RocksDBStorage::new(dir.path()).expect("storage"));
    let hnsw = Arc::new(
        HnswIndexingEngine::new(dir.path().join("hnsw"), 64 * 1024 * 1024, DIMS)
            .expect("hnsw engine")
            .with_spec_resolver(Arc::new(raisin_rocksdb::TenantEmbeddingSpecResolver::new(
                storage.db().clone(),
            ))),
    );

    storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test", None, None, false, false)
        .await
        .expect("branch");

    let mut config = TenantEmbeddingConfig::new(TENANT.to_string());
    config.enabled = true;
    config.provider = EmbeddingProvider::Ollama;
    config.model = "test-model".to_string();
    config.dimensions = DIMS;
    config.base_url = Some("http://127.0.0.1:11434".to_string());
    storage
        .tenant_embedding_config_repository()
        .set_config(&config)
        .expect("store embedding config");

    let embeddings = RocksDBEmbeddingStorage::new(storage.db().clone());
    let embedder_hash = EmbedderId::new("ollama", "test-model", DIMS).to_key_hash();
    let revision = HLC::new(1_000, 0);

    // The rows a real embedding run would have written: one per chunk, under
    // the SAME `cf::EMBEDDINGS` source id, distinguished by the chunk segment.
    for chunk in 0..CHUNKS {
        embeddings
            .store_embedding(
                TENANT,
                REPO,
                BRANCH,
                WS,
                CHUNKED_NODE,
                &revision,
                &data(CHUNKED_NODE, chunk, CHUNKS, chunk + 1),
            )
            .expect("store chunk");
    }
    embeddings
        .store_embedding(
            TENANT,
            REPO,
            BRANCH,
            WS,
            SINGLE_NODE,
            &revision,
            &data(SINGLE_NODE, 0, 1, 99),
        )
        .expect("store single");

    // The two questions storage can answer, and why they differ. Conflating
    // them is the whole bug.
    let nodes = embeddings
        .list_embeddings(TENANT, REPO, BRANCH, WS)
        .expect("list_embeddings");
    assert_eq!(
        nodes.len(),
        2,
        "list_embeddings answers 'which NODES have embeddings' — one row per source"
    );
    let entries = embeddings
        .list_index_entries(TENANT, REPO, BRANCH, WS)
        .expect("list_index_entries");
    assert_eq!(
        entries.len(),
        CHUNKS + 1,
        "list_index_entries answers 'what does the INDEX hold' — one row per chunk"
    );
    assert!(
        entries
            .iter()
            .all(|e| e.embedder_hash == embedder_hash && e.kind == 'T'),
        "every entry must carry the address get_source_chunk needs"
    );

    // Rebuild from scratch.
    let management = HnswManagement::from_stores(
        hnsw.clone(),
        Arc::new(embeddings.clone()),
        Arc::new(storage.tenant_embedding_config_repository()),
    );
    let stats = management
        .rebuild_index(TENANT, REPO, BRANCH, None)
        .await
        .expect("rebuild");

    assert_eq!(
        stats.items_processed,
        CHUNKS + 1,
        "the rebuild must index every CHUNK; {} would mean one vector per node",
        nodes.len()
    );
    assert_eq!(stats.errors, 0, "no row should have been skipped");

    let partition = hnsw
        .default_text_partition(TENANT, REPO, BRANCH)
        .expect("partition resolves from the tenant config");
    let indexed = hnsw
        .stats(TENANT, REPO, BRANCH, &partition)
        .expect("index stats")
        .count;
    assert_eq!(indexed, CHUNKS + 1, "index holds one vector per chunk");

    // And under the ids the WRITER would have used, or the search's
    // `parse_index_id` cannot map a hit back to a node.
    for chunk in 0..CHUNKS {
        let id = raisin_hnsw::chunk_source_id(CHUNKED_NODE, None, chunk, CHUNKS);
        assert!(
            hnsw.contains_embedding(TENANT, REPO, BRANCH, &partition, &id)
                .expect("contains"),
            "chunk {chunk} must be indexed as '{id}'"
        );
    }
    assert!(
        hnsw.contains_embedding(TENANT, REPO, BRANCH, &partition, SINGLE_NODE)
            .expect("contains"),
        "an unchunked document keeps its bare node id"
    );
}

/// The id a rebuild derives from a stored row must be the id the writer wrote.
#[test]
fn stored_index_id_agrees_with_the_writer() {
    for (spec, total) in [
        (None, 1usize),
        (None, 4),
        (Some("doc"), 1),
        (Some("doc"), 4),
    ] {
        for chunk in 0..total {
            let source_id = raisin_hnsw::namespaced_source_id("node1", spec);
            assert_eq!(
                raisin_hnsw::index_id_for_stored(&source_id, chunk, total),
                raisin_hnsw::chunk_source_id("node1", spec, chunk, total),
                "spec={spec:?} chunk={chunk} total={total}"
            );
        }
    }
}
