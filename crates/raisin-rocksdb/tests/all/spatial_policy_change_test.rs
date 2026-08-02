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

//! A precision-policy change has to reach the WRITE path.
//!
//! The admin surface could set `WorkspaceConfig.spatial` all day and nothing
//! happened: the write path resolved its policy from the LOCAL STATE RECORD (a
//! cache of what the index was last *built* under) and only fell back to the
//! configuration when no record existed. So the first write pinned the policy
//! forever, and every later configuration change was inert.
//!
//! These tests go through the ordinary repository write path — no spatial API is
//! called directly — and read back the physical keys.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::{
    GeoJson, PropertyValue, SpatialPropertySchema, SpatialWorkspaceSchema,
};
use raisin_models::nodes::Node;
use raisin_models::workspace::Workspace;
use raisin_rocksdb::spatial_state::SpatialStateStore;
use raisin_rocksdb::{fractional_index, RocksDBConfig, RocksDBStorage, SpatialIndexJobHandler};
use raisin_storage::spatial::{SpatialAvailability, SpatialBuildPhase, SpatialStateSource};
use raisin_storage::{
    BranchRepository, CreateNodeOptions, NodeRepository, RegistryRepository,
    RepositoryManagementRepository, Storage, StorageScope,
};
use tempfile::TempDir;

const TENANT: &str = "spatial-policy";
const REPO: &str = "repo";
const BRANCH: &str = "main";
const WS: &str = "places";
const PROP: &str = "location";

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

        let env = Self {
            _dir: temp_dir,
            storage,
        };
        env.configure(&[9, 8]).await?;
        Ok(env)
    }

    /// Declare a precision set for `PROP` on the workspace record.
    ///
    /// This is exactly what `PUT …/spatial/config` writes.
    async fn configure(&self, precisions: &[usize]) -> Result<()> {
        let service = WorkspaceService::new(self.storage.clone());
        let mut workspace = service
            .get(TENANT, REPO, WS)
            .await?
            .unwrap_or_else(|| Workspace::new(WS.to_string()));
        workspace.name = WS.to_string();
        workspace.config.default_branch = BRANCH.to_string();

        let mut schema = workspace.config.spatial.clone().unwrap_or_default();
        schema.properties.insert(
            PROP.to_string(),
            SpatialPropertySchema {
                precisions: Some(precisions.to_vec()),
                ..Default::default()
            },
        );
        workspace.config.spatial = Some(schema);
        service.put(TENANT, REPO, workspace).await
    }

    async fn write(&self, id: &str, lon: f64, lat: f64) -> Result<()> {
        self.storage
            .nodes()
            .create(
                StorageScope::new(TENANT, REPO, BRANCH, WS),
                place(id, lon, lat),
                relaxed_create(),
            )
            .await?;
        Ok(())
    }

    fn state(&self) -> SpatialStateStore {
        SpatialStateStore::new(self.storage.db().clone())
    }

    /// Live entries per (node_id, precision), read straight off the keys.
    fn entries(&self) -> HashMap<String, Vec<usize>> {
        use raisin_rocksdb::{cf, keys};

        let db = self.storage.db();
        let handle = db.cf_handle(cf::SPATIAL_INDEX).expect("spatial CF");
        let prefix = keys::spatial_index_property_prefix(TENANT, REPO, BRANCH, WS, PROP);

        let mut out: HashMap<String, Vec<usize>> = HashMap::new();
        for item in db.prefix_iterator_cf(handle, &prefix) {
            let (key, value) = item.expect("scan");
            if !key.starts_with(&prefix) {
                break;
            }
            if value.as_ref() == raisin_rocksdb::indexing::SPATIAL_TOMBSTONE {
                continue;
            }
            let Some(parsed) = keys::parse_spatial_index_key(&key) else {
                continue;
            };
            let precisions = out.entry(parsed.node_id.to_string()).or_default();
            let p = parsed.precision();
            if !precisions.contains(&p) {
                precisions.push(p);
            }
        }
        for v in out.values_mut() {
            v.sort_unstable_by(|a, b| b.cmp(a));
        }
        out
    }

    /// The newest revision stamped on any LIVE entry, read straight off the keys.
    ///
    /// An entry newer than the read snapshot is invisible, so this is the value
    /// that has to stay at or below the branch head.
    fn newest_entry_revision(&self) -> Option<HLC> {
        use raisin_rocksdb::{cf, keys};

        let db = self.storage.db();
        let handle = db.cf_handle(cf::SPATIAL_INDEX).expect("spatial CF");
        let prefix = keys::spatial_index_property_prefix(TENANT, REPO, BRANCH, WS, PROP);

        let mut newest: Option<HLC> = None;
        for item in db.prefix_iterator_cf(handle, &prefix) {
            let (key, value) = item.expect("scan");
            if !key.starts_with(&prefix) {
                break;
            }
            if value.as_ref() == raisin_rocksdb::indexing::SPATIAL_TOMBSTONE {
                continue;
            }
            let Some(parsed) = keys::parse_spatial_index_key(&key) else {
                continue;
            };
            if newest.is_none_or(|n| parsed.revision > n) {
                newest = Some(parsed.revision);
            }
        }
        newest
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

fn place(id: &str, lon: f64, lat: f64) -> Node {
    let mut properties = HashMap::new();
    properties.insert(
        PROP.to_string(),
        PropertyValue::Geometry(GeoJson::point(lon, lat)),
    );
    Node {
        id: id.to_string(),
        name: id.to_string(),
        path: format!("/{}", id),
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

/// The first write honours the CONFIGURED precisions, not the server defaults.
#[tokio::test]
async fn the_first_write_uses_the_configured_precisions() -> Result<()> {
    let env = Env::new().await?;
    env.write("a", 8.54, 47.37).await?;

    let entries = env.entries();
    assert_eq!(
        entries.get("a").cloned().unwrap_or_default(),
        vec![9, 8],
        "the configured precision set must decide the cells"
    );

    let state = env
        .state()
        .get(TENANT, REPO, BRANCH, WS, PROP)?
        .expect("first write must create the state record");
    assert_eq!(state.precisions, vec![9, 8]);
    assert_eq!(state.phase, SpatialBuildPhase::Ready);
    Ok(())
}

/// THE regression. After a configuration change, the very next write must emit
/// the NEW precisions — plus the old ones, so rows written on either side of the
/// change stay findable while a rebuild catches up.
#[tokio::test]
async fn a_write_after_a_policy_change_picks_up_the_new_precisions() -> Result<()> {
    let env = Env::new().await?;
    env.write("a", 8.54, 47.37).await?;

    env.configure(&[6, 5]).await?;
    env.write("b", 8.55, 47.38).await?;

    let entries = env.entries();
    assert_eq!(
        entries.get("b").cloned().unwrap_or_default(),
        vec![9, 8, 6, 5],
        "a write after the change must emit configured ∪ indexed, not the stale set"
    );
    assert_eq!(
        entries.get("a").cloned().unwrap_or_default(),
        vec![9, 8],
        "rows written before the change keep their entries until a rebuild"
    );
    Ok(())
}

/// A configuration change must not move the goalposts for queries: the state
/// record still describes what is actually on disk, so the planner keeps using
/// the precision set that is complete for EVERY row.
#[tokio::test]
async fn a_policy_change_does_not_change_what_queries_may_trust() -> Result<()> {
    let env = Env::new().await?;
    env.write("a", 8.54, 47.37).await?;
    env.configure(&[6, 5]).await?;
    env.write("b", 8.55, 47.38).await?;

    let state = env.state();
    let record = state.get(TENANT, REPO, BRANCH, WS, PROP)?.unwrap();
    assert_eq!(
        record.precisions,
        vec![9, 8],
        "only a build may flip the record to the new policy"
    );

    match state.spatial_availability(TENANT, REPO, BRANCH, WS, PROP) {
        SpatialAvailability::Ready { precisions, .. } => assert_eq!(precisions, vec![9, 8]),
        other => panic!("expected Ready on the old precisions, got {:?}", other),
    }
    Ok(())
}

/// While a rebuild is in flight the entry set is a mixture of both policies, so
/// only their overlap is complete — and when there is no overlap the index must
/// declare itself unusable rather than answer partially.
#[tokio::test]
async fn a_rebuild_window_never_answers_partially() -> Result<()> {
    let env = Env::new().await?;
    env.write("a", 8.54, 47.37).await?;

    // Overlapping change: [9,8] -> [8,7], mid-rebuild.
    env.configure(&[8, 7]).await?;
    let state = env.state();
    let mut record = state
        .get(TENANT, REPO, BRANCH, WS, PROP)?
        .unwrap()
        .as_ref()
        .clone();
    record.phase = SpatialBuildPhase::Building;
    state.put(TENANT, REPO, BRANCH, WS, PROP, &record)?;

    match state.spatial_availability(TENANT, REPO, BRANCH, WS, PROP) {
        SpatialAvailability::Ready { precisions, .. } => assert_eq!(precisions, vec![8]),
        other => panic!("expected the overlap, got {:?}", other),
    }

    // Disjoint change: [9,8] -> [5,4], mid-rebuild. Nothing is complete.
    env.configure(&[5, 4]).await?;
    match state.spatial_availability(TENANT, REPO, BRANCH, WS, PROP) {
        SpatialAvailability::Unusable(reason) => {
            assert!(reason.contains("rebuilt"), "unexpected reason: {}", reason)
        }
        other => panic!("expected Unusable, got {:?}", other),
    }
    Ok(())
}

/// Every entry a REBUILD writes must be visible to a read at the branch HEAD.
///
/// # The bug
///
/// The rebuild stamped its entries at the job context's revision — wall-clock
/// `now` — but a rebuild does not advance the branch head, and a spatial read
/// discards every entry stamped after its snapshot
/// (`repositories/spatial_index/repository/scan.rs`). So a completed rebuild wrote
/// entries into the future, where no query could see them. It went unnoticed
/// because a policy change usually KEEPS some precisions: queries kept being
/// answered from the entries the original writes had left at those precisions.
/// Change to a DISJOINT set — the case below — and the crutch is gone: the index
/// reports `ready`, the census counts every entry, and every spatial query in the
/// workspace returns nothing at all.
///
/// The assertion is on the revisions themselves rather than on a query, because
/// "no entry may be stamped after the head" is the invariant; a query is one
/// consequence of breaking it.
#[tokio::test]
async fn a_rebuild_stamps_entries_at_the_node_revision_not_the_job_revision() -> Result<()> {
    let env = Env::new().await?;
    env.write("a", 8.54, 47.37).await?;
    env.write("b", 8.55, 47.38).await?;

    // A disjoint precision set: after the rebuild NOTHING the original writes
    // produced is at a precision the index still claims, so every entry a query
    // could use is one this rebuild wrote.
    env.configure(&[5, 4]).await?;

    let head = env
        .storage
        .branches()
        .get_branch(TENANT, REPO, BRANCH)
        .await?
        .expect("branch")
        .head;

    // Exactly what the enqueuer stamps: wall-clock now, which is AHEAD of the head
    // because the head only moves when something is written.
    //
    // Clamped to at least one millisecond past the head. An HLC compares by
    // (timestamp_ms, counter), and this literal carries counter 0 — so whenever
    // the setup writes above landed in the SAME millisecond as this call, the
    // head holds a higher counter and the precondition loses a race it was never
    // meant to run. That made the test fail on roughly half of fast-machine runs
    // and, worse, skip the behaviour it exists to check.
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let job_revision = HLC::new(now_ms.max(head.timestamp_ms + 1), 0);
    assert!(
        job_revision > head,
        "precondition: the job revision is ahead of the branch head ({job_revision:?} vs {head:?})"
    );

    let handler = SpatialIndexJobHandler::new(env.storage.db().clone());
    let report = handler.build(TENANT, REPO, BRANCH, WS, Some(PROP), true, job_revision)?;
    assert_eq!(report.geometries_indexed, 2);

    let entries = env.entries();
    assert_eq!(entries.len(), 2, "both nodes must be indexed: {entries:?}");
    for (node, precisions) in &entries {
        assert_eq!(precisions, &vec![5, 4], "node {node}: {precisions:?}");
    }

    let newest = env.newest_entry_revision().expect("live entries");
    assert!(
        newest <= head,
        "a rebuilt entry stamped after the branch head is invisible to every read: \
         newest entry {newest:?} > head {head:?}"
    );
    Ok(())
}
