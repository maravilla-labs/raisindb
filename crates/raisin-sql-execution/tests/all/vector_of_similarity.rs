//! Similarity by reference: `KNN(VECTOR_OF(...))`, self-exclusion, chunk
//! collapse, and a BOUND vector reaching the same rows.
//!
//! The four things asserted here are the four ways this feature is silently
//! wrong if it is only half built:
//!
//! 1. **The stored vector is what gets searched with.** Not a re-encode. The
//!    fixture never wires an embedding provider, exactly like the pgwire and
//!    WebSocket engines, so if `VECTOR_OF` reached for one the query would fail
//!    outright rather than quietly re-embedding.
//! 2. **The reference is excluded from its own results.** Its vector is at
//!    distance 0 from itself, so without the drop it is rank 1 in every
//!    similar-to-this query and `LIMIT k` spends a slot restating the question.
//! 3. **A chunked source is an ERROR, never chunk 0.** An image has one vector
//!    and the question has an answer; a document has several and they are
//!    different vectors. Picking the first silently would answer a question
//!    nobody asked, invisibly.
//! 4. **A bound vector is a vector.** `$1` never reaches the argument parser --
//!    parameters are substituted into the SQL text first, and the HTTP/pgwire
//!    substituter renders a bound array as the quoted string `'[0.1,0.2]'`.
//!    That string used to be sent to the embedding provider as a search phrase.
//!    The test binds the reference node's OWN stored vector and requires the
//!    same rows in the same order as `VECTOR_OF` produced -- one assertion that
//!    covers both the recognition and the equivalence.

use futures::StreamExt;
use raisin_embeddings::models::EmbeddingData;
use raisin_embeddings::EmbeddingStorage;
use raisin_embeddings::{EmbedderId, EmbeddingKind};
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{BranchRepository, CreateNodeOptions, NodeRepository, Storage, StorageScope};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "t_vector_of";
const REPO: &str = "r_vector_of";
const BRANCH: &str = "main";
const WS: &str = "assets";
const ASSET_TYPE: &str = "raisin:Asset";

/// Small enough to write by hand, wide enough that the ordering below is not an
/// accident of a 2-D toy.
const DIMS: usize = 4;

/// The embedder this fixture writes vectors under.
///
/// The partition token is DERIVED from it rather than written out, because that
/// derivation is the invariant under test: the writer keys `cf::EMBEDDINGS` on
/// `embedder_id.to_key_hash()` + `embedding_kind.to_key_char()`, and
/// `VECTOR_OF` addresses the row through `PartitionId::embedder_hash()` +
/// `kind_char()`. A hardcoded token here would let those two drift and the test
/// would still pass.
fn embedder() -> EmbedderId {
    EmbedderId::new("fixture", "fixture-model", DIMS)
}

fn partition() -> raisin_hnsw::PartitionId {
    raisin_hnsw::PartitionId::new(format!(
        "{}{}",
        embedder().to_key_hash(),
        EmbeddingKind::Text.to_key_char()
    ))
}

/// A resolver whose answer is fixed, standing in for the real
/// `TenantEmbeddingSpecResolver`. Without one, `default_text_partition` is
/// `None` and the vector leg has no index to search.
struct FixedPartition;

impl raisin_hnsw::IndexSpecResolver for FixedPartition {
    fn spec_for(
        &self,
        _t: &str,
        _r: &str,
        _b: &str,
        _partition: &raisin_hnsw::PartitionId,
    ) -> Option<raisin_hnsw::IndexSpec> {
        Some(raisin_hnsw::IndexSpec::new(DIMS))
    }

    fn default_text_partition(
        &self,
        _t: &str,
        _r: &str,
        _b: &str,
    ) -> Option<raisin_hnsw::PartitionId> {
        Some(partition())
    }
}

fn scope() -> StorageScope<'static> {
    StorageScope::new(TENANT, REPO, BRANCH, WS)
}

fn catalog() -> Arc<StaticCatalog> {
    Arc::new(StaticCatalog::default_nodes_schema())
}

/// Four vectors on the unit circle of the first two axes. `anchor` is nearest to
/// `near`, then `mid`, then `far` -- an ordering that can be read off the
/// numbers rather than trusted from the ranking.
fn vectors() -> Vec<(&'static str, Vec<f32>)> {
    vec![
        ("anchor", vec![1.0, 0.0, 0.0, 0.0]),
        ("near", vec![0.97, 0.24, 0.0, 0.0]),
        ("mid", vec![0.71, 0.71, 0.0, 0.0]),
        ("far", vec![0.0, 1.0, 0.0, 0.0]),
    ]
}

struct Fixture {
    storage: Arc<raisin_rocksdb::RocksDBStorage>,
    hnsw: Arc<raisin_hnsw::HnswIndexingEngine>,
    embeddings: Arc<dyn EmbeddingStorage>,
    _db: TempDir,
    _vectors: TempDir,
}

impl Fixture {
    /// Catalog, HNSW engine, embedding store -- and deliberately **no**
    /// embedding provider. A `VECTOR_OF` query that tried to encode anything
    /// would fail here, which is what makes "it read the stored vector" an
    /// assertion rather than a hope.
    fn query_engine(&self) -> QueryEngine<raisin_rocksdb::RocksDBStorage> {
        QueryEngine::new(
            self.storage.clone(),
            TENANT.to_string(),
            REPO.to_string(),
            BRANCH.to_string(),
        )
        .with_catalog(catalog())
        .with_hnsw_engine(self.hnsw.clone())
        .with_embedding_storage(self.embeddings.clone())
    }

    /// Node ids in emitted order.
    async fn ids(&self, sql: &str) -> Result<Vec<String>, String> {
        let engine = self.query_engine();
        let mut stream = engine.execute(sql).await.map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        while let Some(row) = stream.next().await {
            let row = row.map_err(|e| e.to_string())?;
            let value = row
                .get_by_unqualified("node_id")
                .expect("every search row carries node_id");
            match value {
                PropertyValue::String(s) => out.push(s.clone()),
                other => panic!("node_id came back as {other:?}"),
            }
        }
        Ok(out)
    }
}

/// Store one vector in BOTH places, exactly as the embedding job does: the HNSW
/// graph is what gets searched, `cf::EMBEDDINGS` is what `VECTOR_OF` reads back.
/// Writing only one of them is how a test can pass while the feature does not
/// work.
///
/// The `cf::EMBEDDINGS` write goes through the REAL `store_embedding`, so the
/// key this test reads from is the key production writes -- not one this file
/// derived for itself.
#[allow(clippy::too_many_arguments)]
fn store_vector(
    f: &Fixture,
    source_id: &str,
    chunk_idx: usize,
    total_chunks: usize,
    hnsw_id: &str,
    vector: Vec<f32>,
    revision: HLC,
) {
    f.hnsw
        .add_embedding(
            TENANT,
            REPO,
            BRANCH,
            &partition(),
            WS,
            hnsw_id,
            revision,
            raisin_hnsw::normalize_vector(&vector),
        )
        .expect("index vector");

    #[allow(deprecated)]
    let data = EmbeddingData {
        vector,
        embedder_id: embedder(),
        embedding_kind: EmbeddingKind::Text,
        source_id: source_id.to_string(),
        chunk_index: chunk_idx,
        total_chunks,
        chunk_content: None,
        generated_at: chrono::Utc::now(),
        text_hash: 0,
        spec_hash: Some(1),
        chunk_span: None,
        model: "fixture-model".to_string(),
        provider: raisin_embeddings::EmbeddingProvider::OpenAI,
    };

    f.embeddings
        .store_embedding(TENANT, REPO, BRANCH, WS, source_id, &revision, &data)
        .expect("store embedding");
}

async fn fixture() -> Fixture {
    let db = TempDir::new().expect("db temp dir");
    let vectors_dir = TempDir::new().expect("vector temp dir");

    let storage = Arc::new(raisin_rocksdb::RocksDBStorage::new(db.path()).expect("storage"));
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await;

    for (id, _) in vectors() {
        let mut props = HashMap::new();
        props.insert(
            "description".to_string(),
            PropertyValue::String(format!("asset {id}")),
        );
        let node = Node {
            id: id.to_string(),
            path: format!("/{id}"),
            name: id.to_string(),
            parent: Some("/".to_string()),
            node_type: ASSET_TYPE.to_string(),
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
            .unwrap_or_else(|e| panic!("create {id}: {e}"));
    }

    // A second node standing in for a CHUNKED document: one node, two stored
    // vectors. This is the case VECTOR_OF must refuse rather than collapse.
    let mut props = HashMap::new();
    props.insert(
        "body".to_string(),
        PropertyValue::String("a long manual".to_string()),
    );
    storage
        .nodes()
        .create(
            scope(),
            Node {
                id: "manual".to_string(),
                path: "/manual".to_string(),
                name: "manual".to_string(),
                parent: Some("/".to_string()),
                node_type: ASSET_TYPE.to_string(),
                properties: props,
                ..Default::default()
            },
            CreateNodeOptions {
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                ..Default::default()
            },
        )
        .await
        .expect("create manual");

    let hnsw = Arc::new(
        raisin_hnsw::HnswIndexingEngine::new(
            vectors_dir.path().to_path_buf(),
            16 * 1024 * 1024,
            DIMS,
        )
        .expect("hnsw engine")
        .with_spec_resolver(Arc::new(FixedPartition)),
    );

    let embeddings: Arc<dyn EmbeddingStorage> = Arc::new(
        raisin_rocksdb::RocksDBEmbeddingStorage::new(storage.db().clone()),
    );

    let f = Fixture {
        storage,
        hnsw,
        embeddings,
        _db: db,
        _vectors: vectors_dir,
    };

    let revision = HLC::new(1, 1);
    for (id, v) in vectors() {
        store_vector(&f, id, 0, 1, id, v, revision);
    }
    // Two chunks under one source id, indexed under the `{node}#{n}` HNSW
    // vocabulary the chunked writer uses.
    store_vector(
        &f,
        "manual",
        0,
        2,
        "manual#0",
        vec![0.5, 0.5, 0.5, 0.5],
        revision,
    );
    store_vector(
        &f,
        "manual",
        1,
        2,
        "manual#1",
        vec![0.0, 0.0, 1.0, 0.0],
        revision,
    );

    f
}

/// The headline behaviour, and the two things that make it usable.
#[tokio::test(flavor = "multi_thread")]
async fn vector_of_finds_neighbours_excludes_self_and_matches_a_bound_vector() {
    let f = fixture().await;

    // -- 1. VECTOR_OF ranks by the stored vector, nearest first ------------
    let by_reference = f
        .ids(&format!(
            "SELECT node_id FROM KNN(VECTOR_OF('{WS}:/anchor'), 5, \
             workspaces => 'ALL READABLE', max_distance => 2.0)"
        ))
        .await
        .expect("VECTOR_OF query must run with no embedding provider wired");

    assert!(
        !by_reference.contains(&"anchor".to_string()),
        "SELF-EXCLUSION: the reference node must not be its own nearest \
         neighbour, and its distance to itself is 0 so it would otherwise be \
         rank 1 in every such query. Got {by_reference:?}"
    );
    assert_eq!(
        by_reference.first().map(String::as_str),
        Some("near"),
        "the nearest stored neighbour of 'anchor' is 'near'; got {by_reference:?}"
    );
    assert!(
        by_reference.contains(&"mid".to_string()) && by_reference.contains(&"far".to_string()),
        "excluding the reference must not exclude anything else; got {by_reference:?}"
    );

    // -- 2. The SAME vector, bound as a parameter --------------------------
    //
    // This is what an image encoder outside the database does: it hands over
    // floats. `$1` is substituted into the SQL text before parsing, so what the
    // argument parser actually sees is the quoted string below -- which used to
    // be sent to the embedding provider as a search phrase.
    let anchor_vector = vectors()
        .into_iter()
        .find(|(id, _)| *id == "anchor")
        .map(|(_, v)| v)
        .expect("anchor vector");
    let rendered = format!(
        "[{}]",
        anchor_vector
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(",")
    );

    let by_parameter = f
        .ids(&format!(
            "SELECT node_id FROM KNN('{rendered}', 5, workspaces => 'ALL READABLE', \
             max_distance => 2.0)"
        ))
        .await
        .expect("a bound vector must not need an embedding provider either");

    // The bound query does NOT name a reference node, so nothing is excluded and
    // the anchor comes back at rank 1 -- which is itself the proof that the two
    // paths used the same vector.
    assert_eq!(
        by_parameter.first().map(String::as_str),
        Some("anchor"),
        "a bound vector identical to anchor's must rank anchor first; got \
         {by_parameter:?}"
    );
    let without_self: Vec<String> = by_parameter
        .iter()
        .filter(|id| *id != "anchor")
        .cloned()
        .collect();
    assert_eq!(
        without_self, by_reference,
        "the bound vector and VECTOR_OF for the same source must return the \
         same rows in the same order once the reference itself is discounted"
    );
}

/// The `'[...]'` recognition must not swallow prose.
///
/// Asserted as an EQUIVALENCE between a bracketed phrase and the same phrase
/// without its brackets, rather than against a fixed error string, because
/// whether a text query can be embedded at all depends on process-wide state
/// this test does not own: `configure_query_embedder` installs a `OnceLock`,
/// and `hybrid_search_query_embedder` in this same binary installs one. A test
/// that asserted "a text query fails for want of an embedder" passed alone and
/// failed in the suite — order-dependent, and a false signal either way.
///
/// The equivalence holds under both worlds. If the bracketed form were read as
/// a vector it would take a completely different path: no embedder call, and a
/// 3-element query vector against a 4-dimensional index, i.e. success where the
/// plain form failed or a dimension error where it succeeded.
#[tokio::test(flavor = "multi_thread")]
async fn a_bracketed_phrase_is_still_prose() {
    let f = fixture().await;
    let sql = |q: &str| {
        format!(
            "SELECT node_id FROM KNN('{q}', 5, workspaces => 'ALL READABLE', max_distance => 2.0)"
        )
    };

    let plain = f.ids(&sql("draft release notes")).await;
    let bracketed = f.ids(&sql("[draft] release notes")).await;

    assert_eq!(
        plain.is_ok(),
        bracketed.is_ok(),
        "a bracketed phrase must take the SAME path as the same phrase without \
         brackets. plain={plain:?} bracketed={bracketed:?}"
    );
    if let (Err(plain), Err(bracketed)) = (&plain, &bracketed) {
        assert_eq!(
            plain, bracketed,
            "both must fail for the same reason; a divergence here means the \
             bracketed form was parsed as a vector"
        );
    }

    // And the strict half: a real bracketed VECTOR of the wrong width must be
    // recognised AS a vector, which is what makes the misread loud rather than
    // silent. Three floats against a four-dimensional index.
    let err = f
        .ids("SELECT node_id FROM KNN('[0.1, 0.2, 0.3]', 5, workspaces => 'ALL READABLE')")
        .await
        .expect_err("a 3-float vector cannot search a 4-dimensional index");
    assert!(
        !err.contains("no such node"),
        "it must have been treated as a vector, not a reference; got: {err}"
    );
}

/// Chunk collapse, refused. This is the assertion that stops the ambiguous case
/// from being resolved by accident: `manual` has two stored vectors and the
/// query names neither.
#[tokio::test(flavor = "multi_thread")]
async fn a_chunked_source_is_an_error_and_never_silently_chunk_zero() {
    let f = fixture().await;

    let err = f
        .ids(&format!(
            "SELECT node_id FROM KNN(VECTOR_OF('{WS}:/manual'), 5, workspaces => 'ALL READABLE')"
        ))
        .await
        .expect_err(
            "a source with several stored vectors is ambiguous and must fail, \
             not resolve to chunk 0",
        );
    assert!(
        err.contains("2 stored chunks"),
        "the error must name how many chunks there are; got: {err}"
    );
    assert!(
        err.contains("VECTOR_OF"),
        "the error must name the argument that resolves it; got: {err}"
    );

    // Naming the chunk resolves it -- the ambiguity is the caller's to settle,
    // and once settled the query runs.
    let named = f
        .ids(&format!(
            "SELECT node_id FROM KNN(VECTOR_OF('{WS}:/manual', 1), 5, \
             workspaces => 'ALL READABLE', max_distance => 2.0)"
        ))
        .await
        .expect("naming the chunk must make the query runnable");
    assert!(
        !named.contains(&"manual".to_string()),
        "self-exclusion applies to a chunk-addressed reference too; got {named:?}"
    );
}

/// A reference that cannot be resolved fails loudly and by name, rather than
/// producing an empty vector leg that looks like an empty corpus.
#[tokio::test(flavor = "multi_thread")]
async fn an_unresolvable_reference_says_so() {
    let f = fixture().await;

    let err = f
        .ids(&format!(
            "SELECT node_id FROM KNN(VECTOR_OF('{WS}:/nope'), 5, workspaces => 'ALL READABLE')"
        ))
        .await
        .expect_err("a missing reference node must be an error");
    assert!(err.contains("no such node"), "got: {err}");

    // The workspace prefix is required, exactly as it is for REFERENCES(...).
    let err = f
        .ids(&format!(
            "SELECT node_id FROM KNN(VECTOR_OF('/anchor'), 5, workspaces => 'ALL READABLE')"
        ))
        .await
        .expect_err("a reference with no workspace must be an error");
    assert!(err.contains("workspace"), "got: {err}");
}
