//! `HYBRID_SEARCH` must actually embed the query — on every SQL surface, not
//! just the one that remembered to wire a provider.
//!
//! `QueryEngine::with_embedding_provider` had exactly ONE call site in the whole
//! workspace outside its own definition: the HTTP `/api/sql` handler. pgwire
//! simple query, pgwire extended query, the WebSocket `sql_query` request and
//! the three engine constructions behind `raisin.sql()` all wired the fulltext
//! and HNSW engines and stopped there. `HYBRID_SEARCH` then hit:
//!
//! ```ignore
//! if let (Some(ref hnsw), Some(ref provider)) = (&hnsw_engine, &embedding_provider) {
//! ```
//!
//! ...found `provider` empty, and fell through with NO else arm. The vector
//! half never ran. Rows still came back — fused from the fulltext leg alone,
//! with `vector_rank` and `vector_distance` NULL on every one of them — and
//! nothing was logged. That is indistinguishable from a working hybrid query
//! whose vector leg happened to match nothing, so an agent built on
//! `raisin.sql('SELECT * FROM HYBRID_SEARCH(...)')` got keyword search and
//! believed it got hybrid.
//!
//! Both halves of the fix are asserted here, and note what the engines under
//! test do NOT have: **no `with_embedding_provider` call anywhere in this
//! file.** These engines are built the way pgwire and WebSocket build theirs.
//!
//! 1. An HNSW engine present with no embedder resolvable is now a loud error
//!    rather than a silent downgrade to full-text.
//! 2. With a process-wide embedder installed (what server startup does), an
//!    engine that never wired a provider embeds the query anyway.
//!
//! The two run in ONE `#[test]` on purpose: the installed embedder is a
//! process-wide `OnceLock`, so "before install" and "after install" cannot be
//! two test functions that the harness may run in either order or in parallel.

use futures::StreamExt;
use raisin_embeddings::provider::EmbeddingProvider;
use raisin_indexer::{BatchIndexContext, TantivyIndexingEngine};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::fulltext::NodeIndexPlan;
use raisin_storage::{BranchRepository, CreateNodeOptions, NodeRepository, Storage, StorageScope};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "t_hybrid_embed";
const REPO: &str = "r_hybrid_embed";
const BRANCH: &str = "main";
const WS: &str = "library";
const DOC_TYPE: &str = "raisin:Document";
const TERM: &str = "zebracorn";

/// Vector width used by both the fake provider and the HNSW index, so the two
/// agree and a failure here can only be about wiring.
const DIMS: usize = 8;

/// Counts the queries it was asked to embed.
///
/// The count is the actual assertion: it proves the query text reached an
/// embedder from an engine that never had one wired, which is precisely what
/// four of the five SQL surfaces could not do.
struct CountingProvider {
    calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl EmbeddingProvider for CountingProvider {
    async fn generate_embedding(&self, _text: &str) -> raisin_error::Result<Vec<f32>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(vec![0.1; DIMS])
    }

    fn dimensions(&self) -> usize {
        DIMS
    }
}

/// Stands in for `TenantQueryEmbedderResolver`, which needs a real tenant
/// embedding config in RocksDB. What is under test is the plumbing between the
/// installed embedder and the executor, not the config read.
struct InstalledEmbedder {
    calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl raisin_embeddings::TenantQueryEmbedder for InstalledEmbedder {
    async fn embedder_for(
        &self,
        _tenant_id: &str,
    ) -> raisin_error::Result<Option<Arc<dyn EmbeddingProvider>>> {
        Ok(Some(Arc::new(CountingProvider {
            calls: self.calls.clone(),
        })))
    }
}

fn scope() -> StorageScope<'static> {
    StorageScope::new(TENANT, REPO, BRANCH, WS)
}

fn catalog() -> Arc<StaticCatalog> {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    Arc::new(catalog)
}

/// A spec resolver that names one text partition, standing in for the tenant
/// embedding config row that `TenantEmbeddingSpecResolver` reads in production.
struct FixedPartition;

impl raisin_hnsw::IndexSpecResolver for FixedPartition {
    fn spec_for(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        _partition: &raisin_hnsw::PartitionId,
    ) -> Option<raisin_hnsw::IndexSpec> {
        Some(raisin_hnsw::IndexSpec::new(DIMS))
    }

    fn default_text_partition(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
    ) -> Option<raisin_hnsw::PartitionId> {
        // Any valid token ending in 'T'; the vectors this fixture indexes go to
        // whichever partition the writer used, and the assertions here are about
        // the embedder being CALLED, not about recall.
        Some(raisin_hnsw::PartitionId::new("fixtureaaaaT"))
    }
}

struct Fixture {
    storage: Arc<raisin_rocksdb::RocksDBStorage>,
    reader: Arc<TantivyIndexingEngine>,
    hnsw: Arc<raisin_hnsw::HnswIndexingEngine>,
    _db: TempDir,
    _index: TempDir,
    _vectors: TempDir,
}

impl Fixture {
    /// An engine wired the way pgwire / WebSocket / `raisin.sql()` wire theirs:
    /// catalog, fulltext engine, HNSW engine — and deliberately NO
    /// `with_embedding_provider`.
    fn query_engine(&self) -> QueryEngine<raisin_rocksdb::RocksDBStorage> {
        QueryEngine::new(
            self.storage.clone(),
            TENANT.to_string(),
            REPO.to_string(),
            BRANCH.to_string(),
        )
        .with_catalog(catalog())
        .with_indexing_engine(self.reader.clone())
        .with_hnsw_engine(self.hnsw.clone())
    }

    async fn run(&self, sql: &str) -> Result<usize, String> {
        let engine = self.query_engine();
        let mut stream = engine.execute(sql).await.map_err(|e| e.to_string())?;
        let mut n = 0;
        while let Some(row) = stream.next().await {
            row.map_err(|e| e.to_string())?;
            n += 1;
        }
        Ok(n)
    }
}

async fn fixture() -> Fixture {
    let db = TempDir::new().expect("db temp dir");
    let index = TempDir::new().expect("index temp dir");
    let vectors = TempDir::new().expect("vector temp dir");

    let storage = Arc::new(raisin_rocksdb::RocksDBStorage::new(db.path()).expect("storage"));
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await;

    let index_path = index.path().to_path_buf();
    let open = || {
        Arc::new(
            TantivyIndexingEngine::new(index_path.clone(), 64 * 1024 * 1024)
                .expect("tantivy engine"),
        )
    };
    let writer = open();

    // One document, indexed for full text. The fulltext leg alone is enough to
    // produce a fused result set, which is exactly how the bug stayed invisible:
    // rows came back either way.
    let mut props = HashMap::new();
    props.insert(
        "content".to_string(),
        PropertyValue::String(format!("the {TERM} report")),
    );
    let node = Node {
        id: "alpha".to_string(),
        path: "/docs/alpha".to_string(),
        name: "alpha".to_string(),
        parent: Some("/".to_string()),
        node_type: DOC_TYPE.to_string(),
        properties: props,
        ..Default::default()
    };
    storage
        .nodes()
        .create(
            scope(),
            node,
            CreateNodeOptions {
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                ..Default::default()
            },
        )
        .await
        .expect("create alpha");

    let stored = storage
        .nodes()
        .get(scope(), "alpha", None)
        .await
        .expect("read back alpha")
        .expect("alpha missing");

    let context = BatchIndexContext {
        tenant_id: TENANT.to_string(),
        repo_id: REPO.to_string(),
        branch: BRANCH.to_string(),
        workspace_id: WS.to_string(),
        default_language: "en".to_string(),
        supported_languages: vec!["en".to_string()],
    };
    writer
        .do_batch_index(
            &context,
            vec![(
                stored,
                NodeIndexPlan {
                    node_type: DOC_TYPE.to_string(),
                    legacy_index_all_strings: true,
                    ..Default::default()
                },
            )],
            vec![],
        )
        .expect("batch index");

    let hnsw = Arc::new(
        raisin_hnsw::HnswIndexingEngine::new(vectors.path().to_path_buf(), 16 * 1024 * 1024, DIMS)
            .expect("hnsw engine")
            // Without a spec resolver `default_text_partition` returns None, so
            // the vector leg has no index to search and fails before it can
            // demonstrate anything about the EMBEDDER, which is what this file
            // is about. Server startup wires the real
            // `TenantEmbeddingSpecResolver` here; this is the same wiring with a
            // fixed answer.
            .with_spec_resolver(Arc::new(FixedPartition)),
    );

    Fixture {
        storage,
        // Reopened after the writer committed, so the first lookup reads from
        // disk rather than racing a background reader reload.
        reader: open(),
        hnsw,
        _db: db,
        _index: index,
        _vectors: vectors,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn hybrid_search_embeds_the_query_on_an_engine_that_wired_no_provider() {
    let f = fixture().await;
    let sql = format!("SELECT * FROM HYBRID_SEARCH('{TERM}', 10, workspaces => 'ALL READABLE')");

    // ---------------------------------------------------------------------
    // 1. HNSW engine present, no embedder resolvable anywhere.
    //
    // This is the exact production shape of the bug: pgwire, WebSocket and
    // `raisin.sql()` all reached here. It used to return rows. It must now
    // refuse, because returning full-text results under the name HYBRID_SEARCH
    // is the failure — the caller cannot tell, and neither could the log.
    // ---------------------------------------------------------------------
    let before = f.run(&sql).await;
    let err = before.expect_err(
        "HYBRID_SEARCH with an HNSW engine and no embedder must fail loudly, not silently \
         return full-text-only rows dressed as hybrid",
    );
    assert!(
        err.contains("embedding"),
        "the error must name the missing embedding configuration so an operator can act on \
         it; got: {err}"
    );

    // ---------------------------------------------------------------------
    // 2. Install a process-wide embedder, which is what server startup does.
    //    The SAME engine construction — still no `with_embedding_provider` —
    //    must now embed the query.
    // ---------------------------------------------------------------------
    let calls = Arc::new(AtomicUsize::new(0));
    assert!(
        raisin_embeddings::configure_query_embedder(Arc::new(InstalledEmbedder {
            calls: calls.clone(),
        })),
        "no other test in this binary may install a query embedder"
    );

    let rows = f
        .run(&sql)
        .await
        .expect("with an embedder installed, HYBRID_SEARCH must run");

    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the query text must reach the embedder exactly once, from an engine that never had a \
         provider wired into it — that is the whole defect"
    );
    assert_eq!(
        rows, 1,
        "the seeded document must still come back through fusion"
    );

    // ---------------------------------------------------------------------
    // 3. `kind =>` selects the embedding space. It lives in this test because
    //    it needs the one thing this fixture has and the other search test
    //    files do not: a real HNSW engine with a resolvable partition.
    // ---------------------------------------------------------------------
    let with_kind = |kind: &str| {
        format!(
            "SELECT * FROM HYBRID_SEARCH('{TERM}', 10, workspaces => 'ALL READABLE', \
             kind => '{kind}')"
        )
    };

    // `text` is the default, so it must be byte-for-byte the default's result.
    assert_eq!(
        f.run(&with_kind("text")).await,
        Ok(rows),
        "kind => 'text' is the default and must return exactly what omitting it returns"
    );

    // `all` with only a text partition configured is the text leg alone -- NOT
    // an error. Making 'all' fail where 'text' succeeds would make the broader
    // selector the stricter one.
    assert_eq!(
        f.run(&with_kind("all")).await,
        Ok(rows),
        "kind => 'all' must fall back to the spaces that do exist, not fail"
    );

    // `image` with no image partition is a LOUD failure, not zero rows. An
    // empty leg reported as a search is indistinguishable from a corpus that
    // matched nothing -- the production shape of the dead-vector-leg bug this
    // whole file exists to prevent.
    let err = f
        .run(&with_kind("image"))
        .await
        .expect_err("kind => 'image' with no image partition must fail loudly");
    assert!(
        err.contains("image"),
        "the error must name the missing image partition; got: {err}"
    );

    let err = f
        .run(&with_kind("pictures"))
        .await
        .expect_err("an unknown kind must be refused");
    assert!(
        err.contains("'text'") && err.contains("'image'") && err.contains("'all'"),
        "the error must name the three valid words; got: {err}"
    );
}
