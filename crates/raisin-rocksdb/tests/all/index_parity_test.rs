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

//! Every write path must produce IDENTICAL spatial index bytes.
//!
//! # What this guards
//!
//! The node-indexing logic existed in four hand-maintained copies plus a dead one,
//! with nothing structurally forcing them to agree. The receipt is already in the
//! tree: "the transaction (SQL DML) write path historically did NOT maintain
//! compound indexes", so `raisin.sql.execute` / pgwire / HTTP SQL were invisible to
//! compound-index scans. The spatial gap was the same bug in a new costume — the
//! replication apply path wrote property, reference and relation indexes and no
//! spatial index at all, so a geometry written on node1 was spatially queryable
//! only on node1.
//!
//! The fix routed every path through one writer (`crate::indexing::spatial`). This
//! test is what stops a fifth path from being added in isolation: it writes ONE
//! fixed node through each live path into its own temp database and asserts the
//! resulting `SPATIAL_INDEX` key/value sets are byte-identical.
//!
//! # Which paths are covered
//!
//! * **repository** — `storage.nodes().create(...)`, reaching
//!   `add_node_indexes_to_batch_with_parent_id`. This is the path that had NO
//!   spatial indexing at all.
//! * **the shared writer** — `crate::indexing::write_node_spatial_indexes`, which is
//!   exactly what the replication apply path and the reindex job both call. A
//!   difference here means one of them would produce entries the others cannot
//!   tombstone.
//!
//! The transaction path (SQL DML and `NodeService`) is covered end-to-end by
//! `spatial_index_test.rs` and by the server-level suites; asserting it here as well
//! would require standing up a full transaction context for no additional signal,
//! since it calls the same `write_node_spatial_indexes`.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::{GeoJson, PropertyValue};
use raisin_models::nodes::Node;
use raisin_models::workspace::Workspace;
use raisin_rocksdb::{fractional_index, RocksDBConfig, RocksDBStorage};
use raisin_storage::{
    BranchRepository, CreateNodeOptions, NodeRepository, RegistryRepository,
    RepositoryManagementRepository, Storage, StorageScope,
};
use tempfile::TempDir;

const TENANT: &str = "parity-test";
const REPO: &str = "repo";
const BRANCH: &str = "main";
const WS: &str = "places";
const NODE_ID: &str = "parity-node";

/// A node carrying TWO geometry properties of different types plus a companion
/// scalar, so a writer that handles only the first property it finds, or only
/// points, is visibly different from one that does the job.
fn fixture_node() -> Node {
    let mut properties = HashMap::new();
    properties.insert(
        "location".to_string(),
        PropertyValue::Geometry(GeoJson::point(8.5402, 47.3782)),
    );
    properties.insert(
        "footprint".to_string(),
        PropertyValue::Geometry(GeoJson::Polygon {
            coordinates: vec![vec![
                [8.5400, 47.3780].into(),
                [8.5404, 47.3780].into(),
                [8.5404, 47.3784].into(),
                [8.5400, 47.3784].into(),
                [8.5400, 47.3780].into(),
            ]],
            srid: None,
        }),
    );
    properties.insert("floor".to_string(), PropertyValue::String("L2".to_string()));

    Node {
        id: NODE_ID.to_string(),
        name: "parity".to_string(),
        path: "/parity".to_string(),
        node_type: "test:Place".to_string(),
        archetype: None,
        properties,
        children: Vec::new(),
        order_key: fractional_index::first(),
        has_children: Some(false),
        parent: Some("/".to_string()),
        version: 1,
        // Fixed timestamps: a differing `created_at` would perturb unrelated indexes
        // and could mask a spatial difference behind noise.
        created_at: Some(
            chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        ),
        updated_at: None,
        published_at: None,
        published_by: None,
        updated_by: Some("tester".to_string()),
        created_by: Some("tester".to_string()),
        translations: None,
        tenant_id: Some(TENANT.to_string()),
        workspace: Some(WS.to_string()),
        owner_id: None,
        relations: Vec::new(),
    }
}

/// The revision the shared writer stamps, so its keys are reproducible.
fn fixed_revision() -> HLC {
    HLC::new(1_767_225_600_000, 7)
}

async fn fresh_storage() -> Result<(TempDir, Arc<RocksDBStorage>)> {
    let temp_dir = TempDir::new().unwrap();
    let mut config = RocksDBConfig::default();
    config.path = temp_dir.path().to_path_buf();
    let storage = Arc::new(RocksDBStorage::with_config(config)?);

    storage
        .registry()
        .register_tenant(TENANT, HashMap::new())
        .await?;
    storage
        .repository_management()
        .create_repository(
            TENANT,
            REPO,
            RepositoryConfig {
                default_language: "en".to_string(),
                supported_languages: vec!["en".to_string()],
                locale_fallback_chains: HashMap::new(),
                default_branch: BRANCH.to_string(),
                description: None,
                tags: HashMap::new(),
            },
        )
        .await?;
    storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "system", None, None, false, false)
        .await?;

    let workspace_service = WorkspaceService::new(storage.clone());
    let mut workspace = Workspace::new(WS.to_string());
    workspace.config.default_branch = BRANCH.to_string();
    workspace_service.put(TENANT, REPO, workspace).await?;

    Ok((temp_dir, storage))
}

/// Every live SPATIAL_INDEX entry, keyed by (property, geohash) so two databases
/// compare deterministically. The revision is deliberately excluded from the
/// comparison key: the paths under test stamp their own, and what must match is the
/// CELL SET and the VALUE RECORD.
fn spatial_entries(storage: &RocksDBStorage) -> BTreeMap<(String, String), Vec<u8>> {
    let db = storage.db();
    let cf = db
        .cf_handle("spatial_index")
        .expect("spatial_index CF must exist");

    let mut out = BTreeMap::new();
    for item in db.iterator_cf(cf, rocksdb::IteratorMode::Start) {
        let (key, value) = item.expect("iteration must succeed");
        if value.as_ref() == b"T" {
            continue; // tombstone
        }
        let parsed = raisin_rocksdb::keys::parse_spatial_index_key(&key)
            .expect("every spatial key must parse through the shared parser");
        out.insert(
            (parsed.property_name.to_string(), parsed.geohash.to_string()),
            value.to_vec(),
        );
    }
    out
}

/// Write via the repository path (`storage.nodes().create`).
async fn write_via_repository() -> Result<(TempDir, BTreeMap<(String, String), Vec<u8>>)> {
    let (dir, storage) = fresh_storage().await?;
    storage
        .nodes()
        .create(
            StorageScope::new(TENANT, REPO, BRANCH, WS),
            fixture_node(),
            CreateNodeOptions {
                validate_schema: false,
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                operation_meta: None,
            },
        )
        .await?;
    let entries = spatial_entries(&storage);
    Ok((dir, entries))
}

/// Write via the shared writer at a fixed revision — exactly what the replication
/// apply path and the reindex job call.
async fn write_via_shared_writer() -> Result<(TempDir, BTreeMap<(String, String), Vec<u8>>)> {
    let (dir, storage) = fresh_storage().await?;

    let node = fixture_node();
    let revision = fixed_revision();
    let ctx = raisin_rocksdb::indexing::IndexCtx::new(TENANT, REPO, BRANCH, WS);
    let targets = raisin_rocksdb::indexing::SpatialIndexTargets::from_db(storage.db().as_ref())?;
    let state = raisin_rocksdb::spatial_state::SpatialStateStore::new(storage.db().clone());
    let policies =
        raisin_rocksdb::indexing::NodeSpatialPolicies::from_local_state(&state, &ctx, &node);

    let mut batch = rocksdb::WriteBatch::default();
    raisin_rocksdb::indexing::write_node_spatial_indexes(
        &mut batch, &targets, &ctx, &node, &revision, &policies,
    )?;
    storage
        .db()
        .write(batch)
        .map_err(|e: rocksdb::Error| raisin_error::Error::storage(e.to_string()))?;

    let entries = spatial_entries(&storage);
    Ok((dir, entries))
}

/// The repository write path and the shared writer must agree on the exact cell set
/// and on the exact value bytes for every geometry property.
///
/// A path that forgets spatial indexing produces an EMPTY set here, which is the
/// failure this test exists to catch — precisely what the replication apply path and
/// `add_node_indexes_to_batch_with_parent_id` used to do.
#[tokio::test]
async fn all_write_paths_produce_identical_spatial_entries() -> Result<()> {
    let (_d1, repository) = write_via_repository().await?;
    let (_d2, shared) = write_via_shared_writer().await?;

    assert!(
        !repository.is_empty(),
        "the repository write path must produce spatial index entries — an empty set \
         means the path skips spatial indexing entirely, the bug this test guards"
    );

    let repo_cells: Vec<&(String, String)> = repository.keys().collect();
    let shared_cells: Vec<&(String, String)> = shared.keys().collect();
    assert_eq!(
        repo_cells, shared_cells,
        "the repository path and the shared writer must index the same cells"
    );

    for (cell, value) in &repository {
        assert_eq!(
            shared.get(cell),
            Some(value),
            "value bytes differ at {cell:?}; the two paths must produce identical records"
        );
    }

    Ok(())
}

/// Both geometry properties must be indexed, at every configured precision. This is
/// the shape assertion that catches a writer handling only the first geometry
/// property it finds, or only points.
#[tokio::test]
async fn both_geometry_properties_are_indexed_at_every_precision() -> Result<()> {
    let (_dir, entries) = write_via_repository().await?;

    let precisions = raisin_rocksdb::spatial::INDEX_PRECISIONS;
    for property in ["location", "footprint"] {
        let cells: Vec<&String> = entries
            .keys()
            .filter(|(p, _)| p == property)
            .map(|(_, cell)| cell)
            .collect();
        assert_eq!(
            cells.len(),
            precisions.len(),
            "{property} must be indexed once per configured precision under the default \
             Centroid cover, got {cells:?}"
        );
        let mut lengths: Vec<usize> = cells.iter().map(|c| c.chars().count()).collect();
        lengths.sort_unstable();
        let mut expected: Vec<usize> = precisions.to_vec();
        expected.sort_unstable();
        assert_eq!(
            lengths, expected,
            "{property}'s cell lengths must be exactly the configured precisions"
        );
    }
    Ok(())
}

/// Re-running the shared writer at the same revision must reproduce identical bytes,
/// which is what makes the reindex job idempotent rather than a source of MVCC churn.
#[tokio::test]
async fn rewriting_at_the_same_revision_is_byte_identical() -> Result<()> {
    let (_d1, first) = write_via_shared_writer().await?;
    let (_d2, second) = write_via_shared_writer().await?;
    assert_eq!(
        first, second,
        "the same node at the same revision must produce identical index bytes"
    );
    Ok(())
}
