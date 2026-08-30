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

//! One node, SEVERAL named embeddings — and the ids stay fetchable.
//!
//! # What this covers that a unit test cannot
//!
//! `raisin_hnsw::chunk_id` unit-tests the grammar, and
//! `raisin_models::nodes::extraction` unit-tests the artifact. What neither can
//! show is that the JOB HANDLER wires them together: that a node carrying an
//! extraction artifact ends up with TWO embeddings under two distinct
//! `cf::EMBEDDINGS` source ids, that both index entries resolve back to the ONE
//! node id a caller can fetch, that a node whose extraction was `unsupported`
//! gets no document vector at all, and that deleting the node retires both
//! namespaces rather than leaving the document vector behind to win ANN slots
//! forever.
//!
//! # Requires a local Ollama
//!
//! `#[ignore]`d: it embeds for real against `http://127.0.0.1:11434` with
//! `nomic-embed-text`. A faked provider would leave the interesting half
//! untested — the point is WHICH texts reach it and under which ids the results
//! are filed.
//!
//! ```text
//! ollama pull nomic-embed-text
//! cargo test -p raisin-rocksdb --test all named_embedding_specs_test -- --ignored --nocapture
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use raisin_ai::config::{EmbedderId, EmbeddingKind};
use raisin_embeddings::config::{EmbeddingProvider, TenantEmbeddingConfig};
use raisin_embeddings::TenantEmbeddingConfigStore;
use raisin_hlc::HLC;
use raisin_hnsw::HnswIndexingEngine;
use raisin_models::nodes::extraction::ExtractionArtifact;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_rocksdb::EXTRACTED_TEXT_SPEC;
use raisin_rocksdb::{RocksDBEmbeddingStorage, RocksDBStorage};
use raisin_storage::jobs::{JobContext, JobId, JobInfo, JobStatus, JobType};
use raisin_storage::{BranchRepository, CreateNodeOptions, NodeRepository, Storage, StorageScope};
use tempfile::TempDir;

const TENANT: &str = "named-specs";
const REPO: &str = "repo";
const BRANCH: &str = "main";
const WS: &str = "default";
const MASTER_KEY: [u8; 32] = [7u8; 32];

/// The document body — what an extractor pulled out of the binary. It has
/// nothing in common with the node's own name, which is the whole reason the
/// two are embedded separately instead of concatenated.
const EXTRACTED: &str = "Section 12, lockout tagout. Before servicing a conveyor, isolate the \
drive motor at the local disconnect and apply a personal lock. Verify zero energy with a \
calibrated meter before removing any guard. A second person must witness the isolation on any \
circuit above six hundred volts.";

struct Env {
    _dir: TempDir,
    storage: Arc<RocksDBStorage>,
    hnsw: Arc<HnswIndexingEngine>,
    revision: HLC,
}

impl Env {
    async fn new(node_id: &str, artifact: ExtractionArtifact) -> Env {
        let dir = TempDir::new().expect("temp dir");
        let storage = Arc::new(RocksDBStorage::new(dir.path()).expect("storage"));
        // Bytes, not entries: too small a cache evicts the live index between
        // calls and silently loses the adds.
        // Built with the REAL spec resolver, exactly as `startup/indexing.rs`
        // does, so the partition this test looks in is the one the job handler
        // writes to.
        let hnsw = Arc::new(
            HnswIndexingEngine::new(dir.path().join("hnsw"), 64 * 1024 * 1024, 768)
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

        // The node as PUBLISH would deliver it: the artifact is already in
        // `properties`, because that is all a SQL copy carries. No extractor
        // runs here, and none needs to.
        let mut properties = HashMap::new();
        properties.insert(
            "title".to_string(),
            PropertyValue::String("Conveyor line C".to_string()),
        );
        artifact.apply(&mut properties, true);

        let node = Node {
            id: node_id.to_string(),
            name: "safety.pdf".to_string(),
            path: "/safety.pdf".to_string(),
            parent: Some("/".to_string()),
            node_type: "test:Doc".to_string(),
            properties,
            ..Default::default()
        };
        storage
            .nodes()
            .create(
                StorageScope::new(TENANT, REPO, BRANCH, WS),
                node,
                CreateNodeOptions {
                    validate_parent_allows_child: false,
                    validate_workspace_allows_type: false,
                    ..Default::default()
                },
            )
            .await
            .expect("create node");

        let revision = storage
            .branches()
            .get_branch(TENANT, REPO, BRANCH)
            .await
            .expect("get branch")
            .expect("branch exists")
            .head;

        let env = Env {
            _dir: dir,
            storage,
            hnsw,
            revision,
        };
        env.configure();
        env
    }

    fn configure(&self) {
        let mut config = TenantEmbeddingConfig::new(TENANT.to_string());
        config.enabled = true;
        config.provider = EmbeddingProvider::Ollama;
        config.model = "nomic-embed-text".to_string();
        config.dimensions = 768;
        config.include_name = true;
        config.include_path = false;
        config.base_url = Some("http://127.0.0.1:11434".to_string());
        config.chunking = None;

        self.storage
            .tenant_embedding_config_repository()
            .set_config(&config)
            .expect("store embedding config");
    }

    fn embeddings(&self) -> RocksDBEmbeddingStorage {
        RocksDBEmbeddingStorage::new(self.storage.db().clone())
    }

    fn embedder_hash(&self) -> String {
        EmbedderId::new("ollama", "nomic-embed-text", 768).to_key_hash()
    }

    /// Which index partition this tenant's text vectors live in.
    ///
    /// Resolved through the engine's `IndexSpecResolver` — the SAME read the
    /// job handler's partition comes from, so the writer and the assertion
    /// cannot drift apart into an empty index that looks like a job that did
    /// nothing.
    fn partition(&self) -> raisin_hnsw::PartitionId {
        self.hnsw
            .default_text_partition(TENANT, REPO, BRANCH)
            .expect("the tenant's embedding config must resolve to a partition")
    }

    fn context(&self) -> JobContext {
        JobContext {
            tenant_id: TENANT.to_string(),
            repo_id: REPO.to_string(),
            branch: BRANCH.to_string(),
            workspace_id: WS.to_string(),
            revision: self.revision,
            metadata: HashMap::new(),
        }
    }

    fn job(&self, node_id: &str, job_type: JobType) -> JobInfo {
        let _ = node_id;
        JobInfo {
            id: JobId::new(),
            job_type,
            status: JobStatus::Running,
            tenant: TENANT.to_string(),
            started_at: chrono::Utc::now(),
            completed_at: None,
            progress: None,
            error: None,
            result: None,
            retry_count: 0,
            max_retries: 3,
            last_heartbeat: None,
            timeout_seconds: 300,
            next_retry_at: None,
            executing_since: None,
        }
    }

    async fn generate(&self, node_id: &str) {
        let handler = raisin_rocksdb::EmbeddingJobHandler::new(
            Arc::clone(&self.storage),
            Arc::clone(&self.hnsw),
            MASTER_KEY,
        );
        let job = self.job(
            node_id,
            JobType::EmbeddingGenerate {
                node_id: node_id.to_string(),
            },
        );
        handler
            .handle_generate(&job, &self.context())
            .await
            .expect("embedding job must succeed (is ollama running?)");
    }

    async fn delete(&self, node_id: &str) {
        let handler = raisin_rocksdb::EmbeddingJobHandler::new(
            Arc::clone(&self.storage),
            Arc::clone(&self.hnsw),
            MASTER_KEY,
        );
        let job = self.job(
            node_id,
            JobType::EmbeddingDelete {
                node_id: node_id.to_string(),
            },
        );
        handler
            .handle_delete(&job, &self.context())
            .await
            .expect("embedding delete job");
    }

    /// Is there a stored row for this spec's chunk 0 at the job's revision?
    fn has_row(&self, node_id: &str, spec: Option<&str>) -> bool {
        self.embeddings()
            .get_chunk(
                TENANT,
                REPO,
                BRANCH,
                WS,
                &self.embedder_hash(),
                EmbeddingKind::Text.to_key_char(),
                &raisin_hnsw::namespaced_source_id(node_id, spec),
                0,
                Some(&self.revision),
            )
            .expect("chunk read")
            .is_some()
    }

    fn indexed(&self, index_id: &str) -> bool {
        self.hnsw
            .contains_embedding(TENANT, REPO, BRANCH, &self.partition(), index_id)
            .expect("contains")
    }
}

/// A node carrying an extraction artifact ends up with TWO embeddings, in two
/// key namespaces, both of which name the ONE node a caller can fetch.
#[tokio::test]
#[ignore = "needs a local Ollama on 127.0.0.1:11434 with nomic-embed-text"]
async fn an_extraction_artifact_gives_the_node_a_second_named_embedding() {
    const NODE: &str = "doc-two-specs";
    let env = Env::new(
        NODE,
        ExtractionArtifact::extracted("v1:sha|k|1".into(), "core-pdf", EXTRACTED.to_string()),
    )
    .await;

    env.generate(NODE).await;

    // --- cf::EMBEDDINGS: two distinct source ids ---------------------------
    assert!(
        env.has_row(NODE, None),
        "the node's own fields must still be embedded under the BARE node id — \
         that is what every existing row and reader contains"
    );
    assert!(
        env.has_row(NODE, Some(EXTRACTED_TEXT_SPEC)),
        "the extracted document body must be embedded under its own namespace"
    );

    // --- HNSW: two ids, both parsing back to a fetchable node --------------
    let default_id = raisin_hnsw::chunk_source_id(NODE, None, 0, 1);
    let doc_id = raisin_hnsw::chunk_source_id(NODE, Some(EXTRACTED_TEXT_SPEC), 0, 1);
    assert_eq!(default_id, NODE, "default spec keeps the legacy bare id");
    assert_eq!(doc_id, format!("{NODE}#{EXTRACTED_TEXT_SPEC}#0"));

    assert!(env.indexed(&default_id), "default vector not in the index");
    assert!(env.indexed(&doc_id), "document vector not in the index");

    for id in [&default_id, &doc_id] {
        let parsed = raisin_hnsw::parse_index_id(id);
        assert_eq!(
            parsed.node_id, NODE,
            "a SearchResult built from {id} would carry an unfetchable node_id"
        );
        // ...and that id really does resolve to a node.
        assert!(
            env.storage
                .nodes()
                .get(
                    StorageScope::new(TENANT, REPO, BRANCH, WS),
                    &parsed.node_id,
                    None
                )
                .await
                .expect("node read")
                .is_some(),
            "{id} did not resolve to a fetchable node"
        );
    }

    // --- a delete must retire BOTH namespaces ------------------------------
    //
    // Miss one and the rows survive, VERIFY reports a permanent mismatch, and a
    // REBUILD re-inserts the deleted node's document vector so it goes on
    // winning ANN slots that belong to live content.
    env.delete(NODE).await;

    assert!(!env.has_row(NODE, None), "default row survived the delete");
    assert!(
        !env.has_row(NODE, Some(EXTRACTED_TEXT_SPEC)),
        "document row survived the delete — this is the residue that degrades recall"
    );
    assert!(!env.indexed(&default_id), "default vector survived");
    assert!(!env.indexed(&doc_id), "document vector survived");
}

/// An `unsupported` extraction produces NO document vector.
///
/// The node still gets its own embedding, and the durable `unsupported` record
/// is what a backfill finds later. What must NOT happen is a `doc` vector of
/// the empty string: it would be a near-neighbour of every query, and it would
/// erase the difference between "we could not read this" and "we read it and it
/// said nothing".
#[tokio::test]
#[ignore = "needs a local Ollama on 127.0.0.1:11434 with nomic-embed-text"]
async fn an_unsupported_extraction_produces_no_document_vector() {
    const NODE: &str = "doc-unsupported";
    let env = Env::new(
        NODE,
        ExtractionArtifact::unsupported("v1:sha|k|1".into(), "no extractor for msword"),
    )
    .await;

    env.generate(NODE).await;

    assert!(
        env.has_row(NODE, None),
        "the node's own fields are still embedded"
    );
    assert!(
        !env.has_row(NODE, Some(EXTRACTED_TEXT_SPEC)),
        "an unsupported binary must not be filed as an embedded document"
    );
    assert!(!env.indexed(&raisin_hnsw::chunk_source_id(
        NODE,
        Some(EXTRACTED_TEXT_SPEC),
        0,
        1
    )));
}
