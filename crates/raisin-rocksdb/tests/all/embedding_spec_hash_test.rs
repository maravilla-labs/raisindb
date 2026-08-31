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

//! An embedding is stale when ANY of its inputs changed — and only then.
//!
//! # What this covers that a unit test cannot
//!
//! The pieces are unit-tested separately: `EmbeddingSpec::hash` moves for every
//! input, the orphan sweep prunes this revision and spares history. What no unit
//! test can show is that the JOB HANDLER wires them together — that a re-run
//! with the same inputs performs no provider call and no write, that changing
//! ONLY the chunking configuration re-embeds, and that the surplus chunk rows
//! are gone afterwards rather than lingering as permanently-matching debris.
//!
//! Before the spec hash, the stored identity was `hash_text(chunk_content)`:
//! per-chunk text and nothing else. A chunking change was noticed only
//! incidentally, by the chunk texts happening to move — and when the new
//! configuration produced FEWER chunks, the surplus ones were never rewritten,
//! never deleted, and came back out of RocksDB on every HNSW rebuild.
//!
//! # Requires a local Ollama
//!
//! `#[ignore]`d: it embeds for real against `http://127.0.0.1:11434` with
//! `nomic-embed-text`. Faking the provider would leave the interesting half
//! untested — the point is that the second run does not CALL it.
//!
//! ```text
//! ollama pull nomic-embed-text
//! cargo test -p raisin-rocksdb --test all embedding_spec_hash_test -- --ignored --nocapture
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use raisin_ai::config::{ChunkingConfig, EmbedderId, EmbeddingKind, OverlapConfig};
use raisin_embeddings::config::{EmbeddingProvider, TenantEmbeddingConfig};
use raisin_embeddings::models::EmbeddingData;
use raisin_embeddings::TenantEmbeddingConfigStore;
use raisin_hlc::HLC;
use raisin_hnsw::HnswIndexingEngine;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_rocksdb::{RocksDBEmbeddingStorage, RocksDBStorage};
use raisin_storage::jobs::{JobContext, JobId, JobInfo, JobStatus, JobType};
use raisin_storage::{BranchRepository, CreateNodeOptions, NodeRepository, Storage, StorageScope};
use tempfile::TempDir;

const TENANT: &str = "spec-hash";
const REPO: &str = "repo";
const BRANCH: &str = "main";
const WS: &str = "default";
const NODE: &str = "doc-1";
const MASTER_KEY: [u8; 32] = [7u8; 32];

/// Long enough that a small chunk size splits it into several pieces and a large
/// one does not.
const BODY: &str = "Lime mortar breathes where cement does not, which is why a solid wall \
bedded in lime sheds the water it takes on instead of trapping it behind an impermeable skin. \
A repointing job that uses the wrong mix will look sound for a decade and then push the face \
off the stone. The gauge of the sand matters as much as the binder: a well graded sharp sand \
packs without shrinking, while a fine soft sand needs more lime and cracks as it cures. Hot \
mixed mortar, slaked on site against the aggregate, sets faster and sticks better than a \
bagged hydrate, but it has to be worked while it is still warm. On an exposed elevation the \
joints should be finished slightly back from the arris so rain runs off the stone rather than \
sitting on a proud ribbon of mortar. A lime wash over the finished work weathers evenly and \
can be renewed without stripping anything back to bare stone.";

struct Env {
    _dir: TempDir,
    storage: Arc<RocksDBStorage>,
    hnsw: Arc<HnswIndexingEngine>,
    revision: HLC,
}

impl Env {
    async fn new() -> Env {
        let dir = TempDir::new().expect("temp dir");
        let storage = Arc::new(RocksDBStorage::new(dir.path()).expect("storage"));
        // The cache size is in BYTES (moka weighs each index by its estimated
        // memory), so a small number here evicts the live index between calls
        // and silently loses the adds. 64 MiB is what the other HNSW tests use.
        // Built with the REAL spec resolver, exactly as `startup/indexing.rs`
        // does. That is what makes the index partition this test looks in the
        // same one the job handler writes to — resolved from the tenant's
        // embedding config rather than guessed at.
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

        let mut properties = HashMap::new();
        properties.insert("body".to_string(), PropertyValue::String(BODY.to_string()));
        let node = Node {
            id: NODE.to_string(),
            name: "mortar".to_string(),
            path: "/mortar".to_string(),
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

        Env {
            _dir: dir,
            storage,
            hnsw,
            revision,
        }
    }

    /// Point the tenant at local Ollama, with the given chunking.
    fn configure(&self, chunking: Option<ChunkingConfig>) {
        let mut config = TenantEmbeddingConfig::new(TENANT.to_string());
        config.enabled = true;
        config.provider = EmbeddingProvider::Ollama;
        config.model = "nomic-embed-text".to_string();
        config.dimensions = 768;
        config.include_name = true;
        config.include_path = false;
        config.base_url = Some("http://127.0.0.1:11434".to_string());
        config.chunking = chunking;

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
    /// job handler's partition comes from. Hard-coding a token here would let
    /// the writer and the assertion drift apart, and the failure mode is an
    /// empty index that looks exactly like "the job did nothing".
    fn partition(&self) -> raisin_hnsw::PartitionId {
        self.hnsw
            .default_text_partition(TENANT, REPO, BRANCH)
            .expect("the tenant's embedding config must resolve to a partition")
    }

    /// Re-write the node with IDENTICAL properties.
    ///
    /// A new revision, unchanged embeddable content — which is exactly what a
    /// virtual-mount sync produces every 15-20 seconds, and what
    /// `is_already_current` (pinned to `context.revision`) can never match.
    async fn touch(&mut self) {
        let node = self
            .storage
            .nodes()
            .get(StorageScope::new(TENANT, REPO, BRANCH, WS), NODE, None)
            .await
            .expect("get node")
            .expect("node exists");

        self.storage
            .nodes()
            .update(
                StorageScope::new(TENANT, REPO, BRANCH, WS),
                node,
                Default::default(),
            )
            .await
            .expect("update node");

        self.revision = self
            .storage
            .branches()
            .get_branch(TENANT, REPO, BRANCH)
            .await
            .expect("get branch")
            .expect("branch exists")
            .head;
    }

    /// Run the embedding job exactly as the worker pool would.
    async fn run_job(&self) {
        let handler = raisin_rocksdb::EmbeddingJobHandler::new(
            Arc::clone(&self.storage),
            Arc::clone(&self.hnsw),
            MASTER_KEY,
        );
        let job = JobInfo {
            id: JobId::new(),
            job_type: JobType::EmbeddingGenerate {
                node_id: NODE.to_string(),
            },
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
        };
        let context = JobContext {
            tenant_id: TENANT.to_string(),
            repo_id: REPO.to_string(),
            branch: BRANCH.to_string(),
            workspace_id: WS.to_string(),
            revision: self.revision,
            metadata: HashMap::new(),
        };
        handler
            .handle_generate(&job, &context)
            .await
            .expect("embedding job must succeed (is ollama running?)");
    }

    /// Every chunk row of this node AT the revision the jobs run against, as
    /// `(chunk_index, spec_hash, generated_at)`.
    ///
    /// `generated_at` is stamped on every write, so comparing two of these
    /// answers "did the second run write anything?" — which no count can.
    fn rows(&self) -> Vec<(usize, Option<u64>, chrono::DateTime<chrono::Utc>)> {
        let storage = self.embeddings();
        let hash = self.embedder_hash();
        let kind = EmbeddingKind::Text.to_key_char();

        let mut indexes: Vec<usize> = storage
            .list_source_chunks(TENANT, REPO, BRANCH, WS, &hash, kind, NODE)
            .expect("chunk scan")
            .into_iter()
            .filter(|(_, revision)| *revision == self.revision)
            .map(|(idx, _)| idx)
            .collect();
        indexes.sort_unstable();

        indexes
            .into_iter()
            .filter_map(|idx| {
                storage
                    .get_chunk(
                        TENANT,
                        REPO,
                        BRANCH,
                        WS,
                        &hash,
                        kind,
                        NODE,
                        idx,
                        Some(&self.revision),
                    )
                    .expect("get chunk")
                    .map(|data: EmbeddingData| (idx, data.spec_hash, data.generated_at))
            })
            .collect()
    }

    fn indexes(&self) -> Vec<usize> {
        self.rows().into_iter().map(|(idx, _, _)| idx).collect()
    }

    /// Which of this node's HNSW ids the index actually holds.
    fn indexed_ids(&self, upper_bound: usize) -> Vec<String> {
        let mut held = Vec::new();
        if self
            .hnsw
            .contains_embedding(TENANT, REPO, BRANCH, &self.partition(), NODE)
            .expect("contains")
        {
            held.push(NODE.to_string());
        }
        for i in 0..upper_bound {
            let id = raisin_hnsw::chunk_index_id(NODE, None, i);
            if self
                .hnsw
                .contains_embedding(TENANT, REPO, BRANCH, &self.partition(), &id)
                .expect("contains")
            {
                held.push(id);
            }
        }
        held
    }
}

/// The whole lifecycle, in one run because each step depends on the last.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a local Ollama on 127.0.0.1:11434 with nomic-embed-text"]
async fn a_chunking_change_re_embeds_and_leaves_no_orphans_while_a_re_run_writes_nothing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("info,raisin_rocksdb=debug,raisin_embeddings=debug")
        .try_init();
    let env = Env::new().await;

    // ---------------------------------------------------------------
    // 1. Small chunks: the document splits into several pieces.
    // ---------------------------------------------------------------
    env.configure(Some(ChunkingConfig {
        chunk_size: 40,
        overlap: OverlapConfig::Tokens(8),
        ..ChunkingConfig::default()
    }));
    env.run_job().await;

    let first = env.rows();
    let many = first.len();
    eprintln!("after small-chunk run: chunk indexes {:?}", env.indexes());
    assert!(
        many > 2,
        "a 40-token chunking of this body must produce several chunks, got {many}"
    );
    assert!(
        first.iter().all(|(_, spec, _)| spec.is_some()),
        "every row must carry the spec hash it was produced under"
    );
    let first_spec = first[0].1;
    assert!(
        first.iter().all(|(_, spec, _)| *spec == first_spec),
        "one run writes one spec hash across all of its chunks"
    );
    assert_eq!(
        env.indexed_ids(many + 8),
        raisin_hnsw::chunk_id_set(NODE, None, many),
        "the index holds exactly this run's chunk ids"
    );

    // ---------------------------------------------------------------
    // 2. STEADY STATE: nothing changed, so the re-run must write nothing.
    //    `generated_at` is stamped per write, so an unchanged timestamp is
    //    proof that the row was not rewritten — and, since the vector is
    //    written with it, that the provider was never called.
    // ---------------------------------------------------------------
    env.run_job().await;
    let second = env.rows();
    assert_eq!(
        first, second,
        "a re-run with identical inputs must not rewrite a single row — this is \
         what stops a periodic pass minting work forever"
    );

    // ---------------------------------------------------------------
    // 3. Change ONLY the chunking configuration, to one that yields FEWER
    //    chunks. The text, the node, the embedder and the revision are all
    //    untouched: a text hash cannot see this, a spec hash must.
    // ---------------------------------------------------------------
    env.configure(Some(ChunkingConfig {
        chunk_size: 4096,
        overlap: OverlapConfig::Tokens(0),
        ..ChunkingConfig::default()
    }));
    env.run_job().await;

    let third = env.rows();
    eprintln!("after large-chunk run: chunk indexes {:?}", env.indexes());
    assert!(
        third.len() < many,
        "the larger chunk size must produce fewer rows: {} -> {}",
        many,
        third.len()
    );
    assert!(
        third.iter().all(|(_, spec, _)| *spec != first_spec),
        "the chunking configuration is part of the spec, so every surviving row \
         must have been re-embedded under a new one"
    );

    // No orphan survives, in EITHER store.
    let expected: Vec<usize> = (0..third.len()).collect();
    assert_eq!(
        env.indexes(),
        expected,
        "the surplus chunk rows of the previous chunking must be DELETED, not \
         left to keep matching queries and to be read back by every rebuild"
    );
    assert_eq!(
        env.indexed_ids(many + 8),
        raisin_hnsw::chunk_id_set(NODE, None, third.len()),
        "and the superseded ids must be out of the HNSW index too"
    );

    // ---------------------------------------------------------------
    // 4. Steady state again at the new configuration.
    // ---------------------------------------------------------------
    env.run_job().await;
    assert_eq!(
        third,
        env.rows(),
        "the new configuration is now the steady state and must also be a no-op"
    );
}

/// A revision bump with unchanged content must not touch the HNSW index.
///
/// # What this covers that the steady-state case above does not
///
/// `is_already_current` is pinned to `context.revision`, so it matches only a
/// re-run at the SAME revision. A rewrite of the node — a publish, a save that
/// touched nothing embeddable, or a virtual-mount sync — mints a new revision
/// and always misses it. The job then correctly skips the provider call via
/// `carry_forward_vectors`, but it used to go on to run the full per-chunk
/// index loop anyway: one `remove` plus one `add` of the same id, per revision.
///
/// That was not merely wasteful. Every `HnswIndex::add` minted a fresh usearch
/// key, and on usearch < 2.26 an erased key left a tombstone that the rehash
/// could never reclaim, because the rehash trigger counted only LIVE entries —
/// which, for a node being updated in place, never moves. Once every slot in
/// the key table was populated, `remove`'s probe had no empty slot to terminate
/// on and spun forever, pegging a core and starving every reader of the index
/// behind the write guard. A mount syncing every ~15-20s reached that in about
/// 85 minutes.
///
/// So this asserts the property that keeps the churn at zero: an unchanged
/// re-embed writes the revision's ROWS and leaves the index alone.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a local Ollama on 127.0.0.1:11434 with nomic-embed-text"]
async fn a_revision_bump_with_unchanged_content_leaves_the_index_alone() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("info,raisin_rocksdb=debug")
        .try_init();
    let mut env = Env::new().await;

    env.configure(Some(ChunkingConfig {
        chunk_size: 40,
        overlap: OverlapConfig::Tokens(8),
        ..ChunkingConfig::default()
    }));
    env.run_job().await;

    let chunks = env.rows().len();
    assert!(chunks > 2, "expected a multi-chunk body, got {chunks}");
    let expected_ids = raisin_hnsw::chunk_id_set(NODE, None, chunks);
    assert_eq!(
        env.indexed_ids(chunks + 8),
        expected_ids,
        "the first run indexes every chunk"
    );

    // A new revision, identical content — the vmount case.
    for _ in 0..3 {
        env.touch().await;
        env.run_job().await;
    }

    // The ROWS are written at each new revision: they are revision-keyed and a
    // later REBUILD reads them, so skipping them would lose history.
    assert_eq!(
        env.rows().len(),
        chunks,
        "each revision gets its own complete set of chunk rows"
    );

    // The INDEX is untouched, and still complete. Both halves matter: an index
    // that lost ids would be worse than the churn it replaced.
    assert_eq!(
        env.indexed_ids(chunks + 8),
        expected_ids,
        "every chunk is still resident after three unchanged re-embeds"
    );
}
