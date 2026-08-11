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

//! Forking a branch must carry COMPOUND_INDEX, UNIQUE_INDEX and EMBEDDINGS
//! across — and each copy must still FUNCTION on the fork.
//!
//! # Why this file exists
//!
//! The branch is part of every index key prefix, so index entries do not carry
//! over implicitly: a fork gets whatever `copy_branch_indexes` puts there and
//! nothing else. That copy list has now silently lost index types TWICE
//! (`ARCHETYPES`/`ELEMENT_TYPES`, then the whole `SPATIAL_INDEX`), each time
//! producing correct-looking queries that returned nothing.
//!
//! The list is now derived from `branches::cf_registry`, and a unit test forces
//! every CF to be classified. That guard proves a CF is *mentioned*. It cannot
//! prove the copied bytes still answer a query, because the revision locator,
//! the key rewrite and the query path each have to agree — which is what this
//! file tests. `branch_fork_spatial_index_test` does exactly this for
//! `SPATIAL_INDEX`; these are the other three copied index families.
//!
//! # Why `UNIQUE_INDEX` is the one that matters most
//!
//! Its failure mode is not missing rows, it is CORRUPTION. A fork whose unique
//! index did not copy accepts a duplicate of a value that is already taken on
//! the parent, and nothing anywhere errors — the constraint simply stops
//! existing on that branch. A locator unit test cannot catch that: the entry can
//! be copied to a perfectly well-formed key that `check_unique_conflict` then
//! never looks at.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use raisin_ai::config::{EmbedderId, EmbeddingKind};
use raisin_embeddings::embedding_storage::EmbeddingStorage;
use raisin_embeddings::models::EmbeddingData;
use raisin_embeddings::EmbeddingProvider;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::schema::{
    CompoundColumnType, CompoundIndexColumn, CompoundIndexDefinition, PropertyType,
    PropertyValueSchema,
};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::{Node, NodeType};
use raisin_rocksdb::{RocksDBEmbeddingStorage, RocksDBStorage};
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, CompoundColumnValue, CompoundIndexRepository,
    CreateNodeOptions, DeleteNodeOptions, NodeRepository, NodeTypeRepository, Storage,
    StorageScope,
};
use tempfile::TempDir;

const TENANT: &str = "fork-idx";
const REPO: &str = "repo";
const MAIN: &str = "main";
const FORK: &str = "publish";
const WS: &str = "people";

const NODE_TYPE: &str = "test:Member";
/// Compound index over `(team, __created_at)` — the canonical
/// "filter by column, order by time" shape.
const INDEX: &str = "team_created";
/// The property carrying `unique: true`.
const UNIQUE_PROP: &str = "email";

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A NodeType that exercises BOTH index families at once: a compound index over
/// `(team, __created_at)` and a `unique: true` property.
fn member_type() -> NodeType {
    NodeType {
        id: Some(NODE_TYPE.to_string()),
        name: NODE_TYPE.to_string(),
        strict: Some(false),
        allowed_children: vec!["*".to_string()],
        indexable: Some(true),
        created_at: Some(Utc::now()),
        properties: Some(vec![
            PropertyValueSchema {
                name: Some(UNIQUE_PROP.to_string()),
                property_type: PropertyType::String,
                required: None,
                unique: Some(true),
                default: None,
                constraints: None,
                structure: None,
                items: None,
                value: None,
                meta: None,
                is_translatable: None,
                allow_additional_properties: None,
                index: None,
                spatial: None,
                encrypted: None,
            },
            PropertyValueSchema {
                name: Some("team".to_string()),
                property_type: PropertyType::String,
                required: None,
                unique: None,
                default: None,
                constraints: None,
                structure: None,
                items: None,
                value: None,
                meta: None,
                is_translatable: None,
                allow_additional_properties: None,
                index: None,
                spatial: None,
                encrypted: None,
            },
        ]),
        compound_indexes: Some(vec![CompoundIndexDefinition {
            name: INDEX.to_string(),
            columns: vec![
                CompoundIndexColumn {
                    property: "team".to_string(),
                    column_type: CompoundColumnType::String,
                    ascending: None,
                },
                CompoundIndexColumn {
                    property: "__created_at".to_string(),
                    column_type: CompoundColumnType::Timestamp,
                    ascending: None,
                },
            ],
            has_order_column: true,
        }]),
        extends: None,
        mixins: Vec::new(),
        overrides: None,
        description: None,
        icon: None,
        version: Some(1),
        required_nodes: Vec::new(),
        initial_structure: None,
        versionable: Some(true),
        publishable: Some(true),
        auditable: Some(false),
        index_types: None,
        updated_at: None,
        published_at: None,
        published_by: None,
        previous_version: None,
        is_mixin: None,
    }
}

fn member(id: &str, team: &str, email: &str) -> Node {
    let mut properties = HashMap::new();
    properties.insert("team".to_string(), PropertyValue::String(team.to_string()));
    properties.insert(
        UNIQUE_PROP.to_string(),
        PropertyValue::String(email.to_string()),
    );

    Node {
        id: id.to_string(),
        name: id.to_string(),
        path: format!("/{id}"),
        parent: Some("/".to_string()),
        node_type: NODE_TYPE.to_string(),
        properties,
        ..Default::default()
    }
}

/// A 4-dimension embedding, enough to prove the bytes round-tripped.
fn embedding(source_id: &str, vector: Vec<f32>) -> EmbeddingData {
    #[allow(deprecated)]
    EmbeddingData {
        vector,
        embedder_id: EmbedderId::new("test", "tiny", 4),
        embedding_kind: EmbeddingKind::Text,
        source_id: source_id.to_string(),
        chunk_index: 0,
        total_chunks: 1,
        chunk_content: Some(format!("content for {source_id}")),
        generated_at: Utc::now(),
        text_hash: 42,
        model: "tiny".to_string(),
        provider: EmbeddingProvider::Ollama,
    }
}

// ---------------------------------------------------------------------------
// Environment
// ---------------------------------------------------------------------------

struct Env {
    _dir: TempDir,
    storage: Arc<RocksDBStorage>,
}

impl Env {
    async fn new() -> Result<Self> {
        let dir = TempDir::new().expect("temp dir");
        let storage = Arc::new(RocksDBStorage::new(dir.path())?);

        storage
            .branches()
            .create_branch(TENANT, REPO, MAIN, "test", None, None, false, false)
            .await?;

        storage
            .node_types()
            .upsert(
                BranchScope::new(TENANT, REPO, MAIN),
                member_type(),
                CommitMetadata::system("seed member type"),
            )
            .await?;

        Ok(Self { _dir: dir, storage })
    }

    fn scope<'a>(&'a self, branch: &'a str) -> StorageScope<'a> {
        StorageScope::new(TENANT, REPO, branch, WS)
    }

    /// The `fromBranch` fork Studio performs: upstream branch, no explicit
    /// revision.
    async fn fork(&self, from: &str, to: &str) -> Result<()> {
        self.storage
            .branches()
            .create_branch(
                TENANT,
                REPO,
                to,
                "tester",
                None,
                Some(from.to_string()),
                false,
                false,
            )
            .await
            .map(|_| ())
    }

    /// Create a node, asserting it succeeded.
    async fn create(&self, branch: &str, node: Node) -> Result<()> {
        self.storage
            .nodes()
            .create(self.scope(branch), node, relaxed_create())
            .await
    }

    /// Create a node and return the raw result, so a test can assert on a
    /// REJECTION rather than unwrapping it.
    async fn try_create(&self, branch: &str, node: Node) -> Result<()> {
        self.storage
            .nodes()
            .create(self.scope(branch), node, relaxed_create())
            .await
    }

    /// Node ids the COMPOUND INDEX reports for a team — this is the index scan,
    /// not a table scan, so an unforked index shows up as an empty result.
    async fn team_members(&self, branch: &str, team: &str) -> Vec<String> {
        let mut ids: Vec<String> = self
            .storage
            .compound_index()
            .scan_compound_index(
                self.scope(branch),
                INDEX,
                &[CompoundColumnValue::String(team.to_string())],
                false,
                true,
                None,
            )
            .await
            .expect("compound index scan must not error")
            .into_iter()
            .map(|entry| entry.node_id)
            .collect();
        ids.sort();
        ids
    }

    fn embeddings(&self) -> RocksDBEmbeddingStorage {
        RocksDBEmbeddingStorage::new(self.storage.db().clone())
    }

    /// The branch's current HEAD.
    ///
    /// Embeddings in this test are written directly rather than by the async
    /// embedding job, so they need a revision the fork will actually copy:
    /// `copy_branch_indexes` only carries entries at or below the source
    /// branch's HEAD. A bare `HLC::now()` can land ABOVE it and then silently
    /// fails to copy for a reason that has nothing to do with the code
    /// under test.
    async fn head(&self, branch: &str) -> HLC {
        self.storage
            .branches()
            .get_branch(TENANT, REPO, branch)
            .await
            .expect("get_branch")
            .expect("branch exists")
            .head
    }

    async fn store_embedding(&self, branch: &str, node_id: &str, vector: Vec<f32>) {
        let head = self.head(branch).await;
        self.embeddings()
            .store_embedding(
                TENANT,
                REPO,
                branch,
                WS,
                node_id,
                &head,
                &embedding(node_id, vector),
            )
            .expect("store embedding");
    }

    /// Node ids with an embedding on this branch, as the HNSW rebuild reads
    /// them.
    fn embedded_ids(&self, branch: &str) -> Vec<String> {
        let mut ids: Vec<String> = self
            .embeddings()
            .list_embeddings(TENANT, REPO, branch, WS)
            .expect("list embeddings")
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        ids.sort();
        ids
    }

    fn embedding_vector(&self, branch: &str, node_id: &str) -> Option<Vec<f32>> {
        self.embeddings()
            .get_embedding(TENANT, REPO, branch, WS, node_id, None)
            .expect("get embedding")
            .map(|data| data.vector)
    }
}

fn relaxed_create() -> CreateNodeOptions {
    CreateNodeOptions {
        validate_parent_allows_child: false,
        validate_workspace_allows_type: false,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// UNIQUE_INDEX — the data-integrity one
// ---------------------------------------------------------------------------

/// THE constraint, end to end. Claim a value on `main`, fork, try to claim the
/// SAME value on the fork.
///
/// A fork that did not inherit the unique index accepts this write. There is no
/// error, no warning and no missing row — the branch simply has two nodes
/// holding a value declared unique, which is corruption that outlives the fork.
#[tokio::test]
async fn a_fork_inherits_the_unique_index_and_still_rejects_a_duplicate() -> Result<()> {
    let env = Env::new().await?;

    env.create(MAIN, member("ana", "eng", "ana@example.test"))
        .await?;

    // Precondition: the constraint holds on the source branch.
    assert!(
        env.try_create(MAIN, member("ana-dup", "eng", "ana@example.test"))
            .await
            .is_err(),
        "the unique constraint must reject a duplicate on the source branch",
    );

    env.fork(MAIN, FORK).await?;

    let duplicate_on_fork = env
        .try_create(FORK, member("ana-dup", "eng", "ana@example.test"))
        .await;

    assert!(
        duplicate_on_fork.is_err(),
        "the fork must inherit the unique index and reject a value already \
         claimed on the parent — an unforked unique index accepts this write \
         silently, which is CORRUPTION rather than a missing row",
    );

    Ok(())
}

/// The inherited constraint must still name the ORIGINAL owner.
///
/// A copy that landed the entry under a well-formed key but lost the value
/// would read as "taken by ''" — still a rejection, so the test above would
/// pass, while the error message and any conflict-resolution logic that reads
/// the owner would be wrong.
#[tokio::test]
async fn the_inherited_conflict_names_the_node_that_owns_the_value() -> Result<()> {
    let env = Env::new().await?;

    env.create(MAIN, member("ana", "eng", "ana@example.test"))
        .await?;
    env.fork(MAIN, FORK).await?;

    let error = env
        .try_create(FORK, member("ana-dup", "eng", "ana@example.test"))
        .await
        .expect_err("duplicate must be rejected");

    let message = error.to_string();
    assert!(
        message.contains("ana"),
        "the conflict must identify the owning node inherited from the parent, \
         got: {message}",
    );

    Ok(())
}

/// The fork must be a fork, not an alias: a value claimed only on the fork must
/// remain FREE on the parent.
///
/// This is the half a key-rewrite bug would break — entries written under the
/// source branch's prefix would make main reject a value it never issued.
#[tokio::test]
async fn a_value_claimed_only_on_the_fork_stays_free_on_the_parent() -> Result<()> {
    let env = Env::new().await?;

    env.create(MAIN, member("ana", "eng", "ana@example.test"))
        .await?;
    env.fork(MAIN, FORK).await?;

    // A brand new value, claimed only on the fork.
    env.create(FORK, member("bo", "eng", "bo@example.test"))
        .await?;

    env.create(MAIN, member("bo", "eng", "bo@example.test"))
        .await
        .expect(
            "main must not see the fork's unique claim — a leaked entry would \
             make the parent reject a value nobody claimed on it",
        );

    Ok(())
}

/// Releasing a value on the fork must not release it on the parent.
///
/// Deletion writes a TOMBSTONE into the unique index rather than removing the
/// entry, so this exercises the tombstone's branch scoping specifically.
#[tokio::test]
async fn releasing_a_value_on_the_fork_does_not_release_it_on_the_parent() -> Result<()> {
    let env = Env::new().await?;

    env.create(MAIN, member("ana", "eng", "ana@example.test"))
        .await?;
    env.fork(MAIN, FORK).await?;

    env.storage
        .nodes()
        .delete(env.scope(FORK), "ana", DeleteNodeOptions::default())
        .await?;

    // The fork released it, so the fork may re-issue it.
    env.create(FORK, member("ana2", "eng", "ana@example.test"))
        .await
        .expect("the fork tombstoned the value, so it is free there");

    // Main never deleted anything.
    assert!(
        env.try_create(MAIN, member("ana3", "eng", "ana@example.test"))
            .await
            .is_err(),
        "the fork's tombstone must not release the value on the parent",
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// COMPOUND_INDEX
// ---------------------------------------------------------------------------

/// Compound entries carry a VARIABLE number of column segments, so the fork
/// locates their revision from the TAIL. Get that wrong and the entry is either
/// dropped or mis-filtered against the fork's max revision — both of which look
/// like "the query returns nothing" on a branch whose nodes are plainly there.
#[tokio::test]
async fn a_fork_answers_compound_index_scans_with_the_parents_rows() -> Result<()> {
    let env = Env::new().await?;

    env.create(MAIN, member("ana", "eng", "ana@example.test"))
        .await?;
    env.create(MAIN, member("bo", "eng", "bo@example.test"))
        .await?;
    env.create(MAIN, member("cy", "sales", "cy@example.test"))
        .await?;

    // Precondition: the index answers on the source branch.
    assert_eq!(
        env.team_members(MAIN, "eng").await,
        vec!["ana".to_string(), "bo".to_string()],
    );

    env.fork(MAIN, FORK).await?;

    assert_eq!(
        env.team_members(FORK, "eng").await,
        vec!["ana".to_string(), "bo".to_string()],
        "the fork's compound index must contain the parent's entries — an \
         unforked compound index answers every filter+ORDER BY query with zero \
         rows while the nodes themselves fork fine",
    );
    assert_eq!(
        env.team_members(FORK, "sales").await,
        vec!["cy".to_string()],
        "every equality-column value must fork, not just the first",
    );

    Ok(())
}

/// Writes on either side of the fork stay put.
///
/// A copier that got the key rewrite wrong could leave entries under the SOURCE
/// branch's prefix; the test above would still pass and this one would not.
#[tokio::test]
async fn a_fork_and_its_parent_index_compounds_independently() -> Result<()> {
    let env = Env::new().await?;

    env.create(MAIN, member("shared", "eng", "shared@example.test"))
        .await?;
    env.fork(MAIN, FORK).await?;

    env.create(FORK, member("fork-only", "eng", "fork-only@example.test"))
        .await?;
    env.create(MAIN, member("main-only", "eng", "main-only@example.test"))
        .await?;

    assert_eq!(
        env.team_members(FORK, "eng").await,
        vec!["fork-only".to_string(), "shared".to_string()],
        "the fork must see what it inherited plus its own write, and NOT a \
         later write on main",
    );
    assert_eq!(
        env.team_members(MAIN, "eng").await,
        vec!["main-only".to_string(), "shared".to_string()],
        "main must not see the fork's write",
    );

    Ok(())
}

/// Forking a branch with no compound entries must not invent any.
#[tokio::test]
async fn forking_an_empty_compound_index_yields_an_empty_one() -> Result<()> {
    let env = Env::new().await?;
    env.fork(MAIN, FORK).await?;
    assert!(env.team_members(FORK, "eng").await.is_empty());
    Ok(())
}

// ---------------------------------------------------------------------------
// EMBEDDINGS
// ---------------------------------------------------------------------------

/// Embeddings had NEITHER a locator unit test nor an e2e fork test — the
/// weakest of the four copies the fork audit turned up.
///
/// Their v2 key ends with a bare `{~revision}` and no trailing segment, so the
/// tail locator reads the last 16 bytes raw. That is the null-safe read, but
/// nothing proved the copied entry is still retrievable on the fork.
#[tokio::test]
async fn a_fork_inherits_the_parents_embeddings() -> Result<()> {
    let env = Env::new().await?;

    env.create(MAIN, member("ana", "eng", "ana@example.test"))
        .await?;
    env.store_embedding(MAIN, "ana", vec![0.1, 0.2, 0.3, 0.4])
        .await;

    // Precondition: readable on the source branch.
    assert_eq!(env.embedded_ids(MAIN), vec!["ana".to_string()]);

    env.fork(MAIN, FORK).await?;

    assert_eq!(
        env.embedded_ids(FORK),
        vec!["ana".to_string()],
        "the fork must inherit the parent's embeddings — without them vector \
         search on the fork returns nothing and an HNSW rebuild has nothing to \
         rebuild FROM",
    );
    assert_eq!(
        env.embedding_vector(FORK, "ana"),
        Some(vec![0.1, 0.2, 0.3, 0.4]),
        "the copied entry must still deserialize to the same vector — a \
         well-formed key holding a mangled value would pass a listing check",
    );

    Ok(())
}

/// Embeddings written after the fork stay on the branch that wrote them.
#[tokio::test]
async fn embeddings_written_after_the_fork_stay_on_their_branch() -> Result<()> {
    let env = Env::new().await?;

    env.create(MAIN, member("shared", "eng", "shared@example.test"))
        .await?;
    env.store_embedding(MAIN, "shared", vec![1.0, 0.0, 0.0, 0.0])
        .await;
    env.fork(MAIN, FORK).await?;

    env.create(FORK, member("fork-only", "eng", "fork-only@example.test"))
        .await?;
    env.store_embedding(FORK, "fork-only", vec![0.0, 1.0, 0.0, 0.0])
        .await;

    env.create(MAIN, member("main-only", "eng", "main-only@example.test"))
        .await?;
    env.store_embedding(MAIN, "main-only", vec![0.0, 0.0, 1.0, 0.0])
        .await;

    assert_eq!(
        env.embedded_ids(FORK),
        vec!["fork-only".to_string(), "shared".to_string()],
        "the fork keeps what it inherited plus its own, and never a later write \
         on main",
    );
    assert_eq!(
        env.embedded_ids(MAIN),
        vec!["main-only".to_string(), "shared".to_string()],
        "main must not see the fork's embedding",
    );

    Ok(())
}

/// Forking a branch with no embeddings must not invent any.
#[tokio::test]
async fn forking_an_empty_embedding_set_yields_an_empty_one() -> Result<()> {
    let env = Env::new().await?;
    env.fork(MAIN, FORK).await?;
    assert!(env.embedded_ids(FORK).is_empty());
    Ok(())
}

// ---------------------------------------------------------------------------
// GRAPH_PROJECTION — correctly NOT copied
// ---------------------------------------------------------------------------

/// A fork keeps its projection CONFIGS and drops the derived projection.
///
/// The CF registry classified `GRAPH_PROJECTION` as a "KNOWN GAP: this is
/// configuration, not a cache, so a fork loses its projection configs". Both
/// halves of that were wrong, and this test is what pins the correction:
///
/// * A projection **config** is a `raisin:GraphAlgorithmConfig` NODE under
///   `/raisin:access_control/graph-config/` (`graph/config/mod.rs`). Nodes fork,
///   so configs already survive — nothing is lost.
/// * The `GRAPH_PROJECTION` CF holds a `PersistedProjection`: a node list, an
///   edge list, weights and a `stale` flag. That is derived adjacency, not
///   configuration, and `recompute_for_branch` does a **full build** when the
///   load misses — a miss is never an empty answer.
///
/// So skipping it is right, and copying it would only spend fork time (and the
/// size of a whole edge list) on data the next recompute rebuilds anyway.
#[tokio::test]
async fn a_fork_inherits_graph_configs_but_not_the_derived_projection() -> Result<()> {
    use raisin_rocksdb::cf;

    let env = Env::new().await?;

    // The config: an ordinary node, so it rides the NODES copy.
    let mut config = member("pagerank-config", "eng", "cfg@example.test");
    config.path = "/graph-config-pagerank".to_string();
    env.create(MAIN, config).await?;

    // The derived projection, written straight to the CF the way the background
    // compute persists it.
    let projection_key = raisin_rocksdb::keys::graph_projection_key(TENANT, REPO, MAIN, "pagerank");
    let handle = env
        .storage
        .db()
        .cf_handle(cf::GRAPH_PROJECTION)
        .expect("GRAPH_PROJECTION cf");
    env.storage
        .db()
        .put_cf(&handle, &projection_key, b"{\"nodes\":[],\"edges\":[]}")
        .expect("persist projection");

    env.fork(MAIN, FORK).await?;

    // The config forked, because it is a node.
    let forked_config = env
        .storage
        .nodes()
        .get_by_path(env.scope(FORK), "/graph-config-pagerank", None)
        .await?;
    assert!(
        forked_config.is_some(),
        "a projection config is a NODE and must fork — losing it would be the \
         gap the registry described",
    );

    // The derived projection did not, and must not.
    let forked_projection = env
        .storage
        .db()
        .get_cf(
            &handle,
            raisin_rocksdb::keys::graph_projection_key(TENANT, REPO, FORK, "pagerank"),
        )
        .expect("read projection");
    assert!(
        forked_projection.is_none(),
        "the derived projection must NOT be copied — a miss triggers a full \
         rebuild, so copying only costs fork time for data that is regenerated",
    );

    Ok(())
}
