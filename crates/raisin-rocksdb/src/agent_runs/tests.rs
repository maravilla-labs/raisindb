// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The store conformance suite against the node-backed store, plus what only
//! this backend can get wrong: a run IS ordinary nodes in `raisin:system`, and
//! the distributed commit section serializes commits across nodes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use raisin_agent_runtime::conformance::{conformance_suite, create_request, scope};
use raisin_agent_runtime::events::RunEventKind;
use raisin_agent_runtime::ids::{RunId, RunScope, Seq};
use raisin_agent_runtime::service::new_record;
use raisin_agent_runtime::state::RunStatus;
use raisin_agent_runtime::store::{AgentRunStore, StoreError};
use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_models::workspace::Workspace;
use raisin_storage::scope::{BranchScope, StorageScope};
use raisin_storage::{
    BranchRepository, CommitMetadata, NodeRepository, NodeTypeRepository, RegistryRepository,
    RepositoryManagementRepository, Storage,
};

use super::layout;
use super::NodeAgentRunStore;
use crate::{RocksDBConfig, RocksDBStorage};

const TENANTS: &[&str] = &["t", "t1", "tenant-a", "tenant-ab"];

/// The real containment of the two types a run is built from: `raisin:Node`
/// admits only `raisin:Node` children, `raisin:Folder` admits anything.
fn node_type(name: &str) -> NodeType {
    let allowed = if name == "raisin:Node" {
        serde_json::json!(["raisin:Node"])
    } else {
        serde_json::json!([])
    };
    serde_json::from_value(serde_json::json!({
        "id": name, "name": name, "strict": false, "version": 1,
        "allowed_children": allowed, "versionable": true, "publishable": true,
        "auditable": false, "indexable": true,
    }))
    .expect("node type")
}

/// A storage with every conformance tenant provisioned: repository `repo`,
/// branch `main`, the `raisin:system` workspace.
async fn provisioned(dir: &tempfile::TempDir) -> Arc<RocksDBStorage> {
    let mut config = RocksDBConfig::default();
    config.path = dir.path().to_path_buf();
    let storage = Arc::new(RocksDBStorage::with_config(config).unwrap());
    for tenant in TENANTS {
        storage
            .registry()
            .register_tenant(tenant, HashMap::new())
            .await
            .unwrap();
        let repo = RepositoryConfig {
            default_language: "en".into(),
            supported_languages: vec!["en".into()],
            locale_fallback_chains: HashMap::new(),
            default_branch: "main".into(),
            description: None,
            tags: HashMap::new(),
        };
        storage
            .repository_management()
            .create_repository(tenant, "repo", repo)
            .await
            .unwrap();
        storage
            .branches()
            .create_branch(tenant, "repo", "main", "system", None, None, false, false)
            .await
            .unwrap();
        for t in ["raisin:Folder", "raisin:Node"] {
            storage
                .node_types()
                .upsert(
                    BranchScope::new(tenant, "repo", "main"),
                    node_type(t),
                    CommitMetadata::system("seed"),
                )
                .await
                .unwrap();
        }
        let mut ws = Workspace::new(layout::RUN_WORKSPACE.to_string());
        ws.config.default_branch = "main".into();
        WorkspaceService::new(storage.clone())
            .put(tenant, "repo", ws)
            .await
            .unwrap();
    }
    storage
}

fn first(req: &raisin_agent_runtime::service::CreateRun) -> RunEventKind {
    RunEventKind::RunCreated {
        subject: req.subject.clone(),
        principal: req.principal.clone(),
        budgets: req.budgets.clone(),
        input: req.input.clone(),
    }
}

#[tokio::test]
async fn node_store_passes_conformance() {
    // The suite asks for a fresh store per check through a sync factory, so
    // the storages are provisioned up front.
    let mut dirs = Vec::new();
    let mut stores = Vec::new();
    for _ in 0..12 {
        let dir = tempfile::tempdir().unwrap();
        let storage = provisioned(&dir).await;
        stores
            .push(Arc::new(NodeAgentRunStore::open(storage, None, "n".into()))
                as Arc<dyn AgentRunStore>);
        dirs.push(dir);
    }
    let stores = Mutex::new(stores);
    conformance_suite(|| stores.lock().unwrap().pop().expect("enough stores")).await;
}

fn stop_commit(
    stored: &raisin_agent_runtime::record::AgentRunRecord,
    s: &RunScope,
) -> raisin_agent_runtime::store::CommitRequest {
    let stop = raisin_agent_runtime::control::ControlCommand {
        control_id: raisin_agent_runtime::ids::ControlId("c".into()),
        kind: raisin_agent_runtime::control::ControlKind::Stop { reason: None },
        issued_by: raisin_agent_runtime::control::ActorRef::user("alice"),
        at_ms: 1,
    };
    let t = raisin_agent_runtime::lifecycle::apply_control(stored, &stop, true, 2).transition;
    raisin_agent_runtime::store::CommitRequest::from_transition(
        s,
        &t,
        raisin_agent_runtime::lifecycle::Fence::None,
        2,
    )
}

/// A run is ordinary nodes: the record and its events are readable through
/// the node API at their derived paths, on the repository's default branch.
#[tokio::test]
async fn a_run_is_ordinary_nodes_in_the_system_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let storage = provisioned(&dir).await;
    let store = NodeAgentRunStore::open(storage.clone(), None, "n".into());
    let s = scope("t");
    let req = create_request(&s, "/x");
    let rec = new_record(&req, RunId::new_v4(), 1);
    let run = rec.run_id.clone();
    store.create(rec, first(&req), None).await.unwrap();
    let stored = store.load(&s, &run).await.unwrap().unwrap();
    store.commit(stop_commit(&stored, &s)).await.unwrap();

    let at = |path: String| {
        let storage = storage.clone();
        async move {
            storage
                .nodes()
                .get_by_path(
                    StorageScope::new("t", "repo", "main", layout::RUN_WORKSPACE),
                    &path,
                    None,
                )
                .await
                .unwrap()
        }
    };
    let record = at(layout::record(&run).unwrap())
        .await
        .expect("record node");
    assert_eq!(layout::prop(&record, "status"), Some("stopped"));
    for seq in 1..=store.load(&s, &run).await.unwrap().unwrap().last_seq.0 {
        assert!(
            at(layout::event(&run, seq).unwrap()).await.is_some(),
            "event {seq}"
        );
    }
    assert!(at(layout::status_entry(RunStatus::Stopped, &run).unwrap())
        .await
        .is_some());
    assert!(at(layout::status_entry(RunStatus::Queued, &run).unwrap())
        .await
        .is_none());
    assert_eq!(
        store
            .read_events(&s, &run, Seq(0), 100)
            .await
            .unwrap()
            .len() as u64,
        store.load(&s, &run).await.unwrap().unwrap().last_seq.0
    );
}

/// With a distributed lock backend, a commit waits for the run's section on
/// ANOTHER node and answers `Busy` rather than committing concurrently.
#[tokio::test]
async fn commit_is_serialized_cluster_wide_by_the_distributed_lease() {
    let dir = tempfile::tempdir().unwrap();
    let storage = provisioned(&dir).await;
    let locks: raisin_locks::LockManagerHandle =
        Arc::new(raisin_locks::InProcessLockManager::new());
    let store = NodeAgentRunStore::open(storage, Some(locks.clone()), "node-b".into());
    let s = scope("t");
    let req = create_request(&s, "/x");
    let rec = new_record(&req, RunId::new_v4(), 1);
    let run = rec.run_id.clone();
    store.create(rec, first(&req), None).await.unwrap();
    // "node-a" holds the run's commit section.
    let key = raisin_locks::scoped_key("t", "repo", "-", &format!("agent_run_run:{run}"));
    let held = locks
        .try_acquire(&key, "node-a", std::time::Duration::from_secs(30))
        .await
        .unwrap()
        .expect("node-a takes the lease");
    let stored = store.load(&s, &run).await.unwrap().unwrap();
    let err = store.commit(stop_commit(&stored, &s)).await.unwrap_err();
    assert_eq!(err, StoreError::Busy);
    // Released: the commit goes through.
    locks.release(&key, held.token).await.unwrap();
    store.commit(stop_commit(&stored, &s)).await.unwrap();
    assert_eq!(
        store.load(&s, &run).await.unwrap().unwrap().state.status(),
        RunStatus::Stopped
    );
}

/// A database written by a build that kept runs in the `agent_runs` column
/// family still opens: the retired family is dropped on open.
#[test]
fn a_database_with_the_retired_column_family_opens_and_drops_it() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db = rocksdb::DB::open_cf(&opts, dir.path(), ["agent_runs"]).unwrap();
        db.put_cf(db.cf_handle("agent_runs").unwrap(), b"k", b"v")
            .unwrap();
    }
    let db = crate::open_db(dir.path()).expect("opens despite the retired family");
    assert!(db.cf_handle("agent_runs").is_none(), "dropped");
    drop(db);
    let on_disk = rocksdb::DB::list_cf(&rocksdb::Options::default(), dir.path()).unwrap();
    assert!(!on_disk.iter().any(|c| c == "agent_runs"), "{on_disk:?}");
}
