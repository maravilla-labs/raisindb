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

//! Forking a branch must carry the SPATIAL INDEX across.
//!
//! # The bug
//!
//! The branch is part of the spatial key prefix
//! (`{tenant}\0{repo}\0{branch}\0{ws}\0geo\0{property}\0{geohash}\0{~rev}\0{node_id}`),
//! so entries do NOT carry over implicitly. `copy_branch_indexes` copied twelve
//! column families and the spatial index was not among them, so a fork got an
//! EMPTY spatial index: `find_within_radius` on the fork returned zero rows
//! while the nodes themselves were plainly there. Silent wrong results.
//!
//! # Why it is the same bug twice
//!
//! The same file previously and silently dropped `ARCHETYPES`/`ELEMENT_TYPES`.
//! The copy list is now derived from `branches::cf_registry`, an exhaustive
//! table that a unit test forces to cover every `cf::` constant.
//!
//! # What this test proves that a unit test cannot
//!
//! That the copied bytes actually answer an INDEX-BACKED query on the fork —
//! including for a NESTED geometry path (`venue.geo`), whose property segment
//! contains dots and pushes the geohash to key part 6 — and that the two
//! branches are genuinely independent afterwards.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::{GeoJson, PropertyValue};
use raisin_models::nodes::Node;
use raisin_models::workspace::Workspace;
use raisin_rocksdb::repositories::spatial_index::SpatialIndexRepository;
use raisin_rocksdb::{fractional_index, RocksDBConfig, RocksDBStorage};
use raisin_storage::spatial::SpatialPreFilter;
use raisin_storage::{
    BranchRepository, CreateNodeOptions, NodeRepository, RegistryRepository,
    RepositoryManagementRepository, Storage, StorageScope,
};
use tempfile::TempDir;

const TENANT: &str = "fork-spatial";
const REPO: &str = "repo";
const MAIN: &str = "main";
const FORK: &str = "publish";
const WS: &str = "places";

/// Top-level geometry property.
const FLAT_PROP: &str = "location";
/// Nested geometry property — indexed under its DOT PATH, which is one key
/// segment containing dots but never a null byte.
const NESTED_PROP: &str = "venue.geo";

// Zurich Hauptbahnhof.
const ZRH_LON: f64 = 8.5402;
const ZRH_LAT: f64 = 47.3782;

/// Far-future read snapshot, so every write is visible.
fn max_rev() -> HLC {
    HLC::new(u64::MAX / 2, 0)
}

struct Env {
    _dir: TempDir,
    storage: Arc<RocksDBStorage>,
}

impl Env {
    async fn new() -> Result<Self> {
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
                    default_branch: MAIN.to_string(),
                    description: None,
                    tags: HashMap::new(),
                },
            )
            .await?;
        storage
            .branches()
            .create_branch(TENANT, REPO, MAIN, "system", None, None, false, false)
            .await?;

        let workspace_service = WorkspaceService::new(storage.clone());
        let mut workspace = Workspace::new(WS.to_string());
        workspace.config.default_branch = MAIN.to_string();
        workspace_service.put(TENANT, REPO, workspace).await?;

        Ok(Self {
            _dir: temp_dir,
            storage,
        })
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

    /// An INDEX-BACKED radius query — this is the spatial index scan, not a
    /// table scan, so an unforked index shows up as an empty result.
    fn within(&self, branch: &str, property: &str, radius: f64) -> Vec<String> {
        let mut ids: Vec<String> = SpatialIndexRepository::new(self.storage.db().clone())
            .find_within_radius(
                TENANT,
                REPO,
                branch,
                WS,
                property,
                ZRH_LON,
                ZRH_LAT,
                radius,
                &max_rev(),
                1000,
                raisin_rocksdb::spatial::INDEX_PRECISIONS,
                &SpatialPreFilter::default(),
            )
            .expect("radius query must not error")
            .into_iter()
            .map(|r| r.node_id)
            .collect();
        ids.sort();
        ids
    }
}

fn relaxed_create() -> CreateNodeOptions {
    CreateNodeOptions {
        validate_schema: false,
        validate_parent_allows_child: false,
        validate_workspace_allows_type: false,
        operation_meta: None,
    }
}

fn node_with(id: &str, properties: HashMap<String, PropertyValue>) -> Node {
    Node {
        id: id.to_string(),
        name: id.to_string(),
        path: format!("/{id}"),
        node_type: "test:Place".to_string(),
        archetype: None,
        properties,
        children: Vec::new(),
        order_key: fractional_index::first(),
        has_children: Some(false),
        parent: Some("/".to_string()),
        version: 1,
        created_at: Some(chrono::Utc::now()),
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

/// A node carrying a TOP-LEVEL geometry.
fn flat_place(id: &str, lon: f64, lat: f64) -> Node {
    let mut properties = HashMap::new();
    properties.insert(
        FLAT_PROP.to_string(),
        PropertyValue::Geometry(GeoJson::point(lon, lat)),
    );
    node_with(id, properties)
}

/// A node carrying a geometry NESTED inside an object, indexed as `venue.geo`.
fn nested_place(id: &str, lon: f64, lat: f64) -> Node {
    let mut venue = HashMap::new();
    venue.insert(
        "geo".to_string(),
        PropertyValue::Geometry(GeoJson::point(lon, lat)),
    );
    venue.insert(
        "name".to_string(),
        PropertyValue::String("Hauptbahnhof".to_string()),
    );

    let mut properties = HashMap::new();
    properties.insert("venue".to_string(), PropertyValue::Object(venue));
    node_with(id, properties)
}

/// Offset a point by `east_m` metres.
fn east(metres: f64) -> (f64, f64) {
    let per_deg_lat = 111_195.0_f64;
    let per_deg_lon = per_deg_lat * ZRH_LAT.to_radians().cos();
    (ZRH_LON + metres / per_deg_lon, ZRH_LAT)
}

// ---------------------------------------------------------------------------
// The regression
// ---------------------------------------------------------------------------

/// THE BUG, end to end. Create geometry on `main`, fork, query the FORK.
///
/// Before the fix this returned an empty vector for both properties while the
/// node records themselves forked fine — so the data was there and the query
/// said "no results", with no error anywhere.
#[tokio::test]
async fn a_fork_answers_spatial_queries_with_the_parents_geometry() -> Result<()> {
    let env = Env::new().await?;

    env.storage
        .nodes()
        .create(
            env.scope(MAIN),
            flat_place("hb", ZRH_LON, ZRH_LAT),
            relaxed_create(),
        )
        .await?;
    env.storage
        .nodes()
        .create(
            env.scope(MAIN),
            nested_place("venue-1", ZRH_LON, ZRH_LAT),
            relaxed_create(),
        )
        .await?;

    // Precondition: the index answers on the source branch.
    assert_eq!(env.within(MAIN, FLAT_PROP, 100.0), vec!["hb".to_string()]);
    assert_eq!(
        env.within(MAIN, NESTED_PROP, 100.0),
        vec!["venue-1".to_string()],
    );

    env.fork(MAIN, FORK).await?;

    assert_eq!(
        env.within(FORK, FLAT_PROP, 100.0),
        vec!["hb".to_string()],
        "the fork's spatial index must contain the parent's geometry — an \
         unforked index silently answers every radius query with zero rows",
    );
    assert_eq!(
        env.within(FORK, NESTED_PROP, 100.0),
        vec!["venue-1".to_string()],
        "a NESTED geometry path must fork too: its property segment carries \
         dots and pushes the geohash to key part 6, which is exactly the \
         'fixed part count' trap that lost ARCHETYPES/ELEMENT_TYPES",
    );

    Ok(())
}

/// The fork must be a fork, not an alias: writes on either side stay put.
///
/// This is the other half of getting the copy right. A copier that got the key
/// rewrite wrong could leave entries under the SOURCE branch's prefix and this
/// would catch it, where the test above would not.
#[tokio::test]
async fn a_fork_and_its_parent_index_independently_afterwards() -> Result<()> {
    let env = Env::new().await?;

    env.storage
        .nodes()
        .create(
            env.scope(MAIN),
            flat_place("shared", ZRH_LON, ZRH_LAT),
            relaxed_create(),
        )
        .await?;
    env.fork(MAIN, FORK).await?;

    // A geometry written only on the fork.
    let (fork_lon, fork_lat) = east(30.0);
    env.storage
        .nodes()
        .create(
            env.scope(FORK),
            flat_place("fork-only", fork_lon, fork_lat),
            relaxed_create(),
        )
        .await?;

    // A geometry written only on main, AFTER the fork.
    let (main_lon, main_lat) = east(60.0);
    env.storage
        .nodes()
        .create(
            env.scope(MAIN),
            flat_place("main-only", main_lon, main_lat),
            relaxed_create(),
        )
        .await?;

    assert_eq!(
        env.within(FORK, FLAT_PROP, 500.0),
        vec!["fork-only".to_string(), "shared".to_string()],
        "the fork must see what it inherited plus its own write, and NOT a \
         later write on main",
    );
    assert_eq!(
        env.within(MAIN, FLAT_PROP, 500.0),
        vec!["main-only".to_string(), "shared".to_string()],
        "main must not see the fork's write",
    );

    Ok(())
}

/// A nested geometry written on the fork after the fork must also stay put.
///
/// Nested paths go through the same key builder but a different property
/// segment, and the fork's copy rewrites the branch by splitting on `\0` —
/// dots in `venue.geo` must not disturb that.
#[tokio::test]
async fn a_nested_geometry_written_on_the_fork_stays_on_the_fork() -> Result<()> {
    let env = Env::new().await?;

    env.storage
        .nodes()
        .create(
            env.scope(MAIN),
            nested_place("inherited", ZRH_LON, ZRH_LAT),
            relaxed_create(),
        )
        .await?;
    env.fork(MAIN, FORK).await?;

    let (lon, lat) = east(40.0);
    env.storage
        .nodes()
        .create(
            env.scope(FORK),
            nested_place("fork-venue", lon, lat),
            relaxed_create(),
        )
        .await?;

    assert_eq!(
        env.within(FORK, NESTED_PROP, 500.0),
        vec!["fork-venue".to_string(), "inherited".to_string()],
    );
    assert_eq!(
        env.within(MAIN, NESTED_PROP, 500.0),
        vec!["inherited".to_string()],
        "the fork's nested geometry must not leak back onto main",
    );

    Ok(())
}

/// Forking a branch with NO geometry must not invent any.
#[tokio::test]
async fn forking_an_empty_spatial_index_yields_an_empty_one() -> Result<()> {
    let env = Env::new().await?;
    env.fork(MAIN, FORK).await?;
    assert!(env.within(FORK, FLAT_PROP, 10_000.0).is_empty());
    Ok(())
}
