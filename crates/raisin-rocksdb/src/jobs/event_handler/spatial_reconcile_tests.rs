//! Tests for the spatial reconciliation arm.
//!
//! The case that matters: an operator changes the precision set on an
//! already-built index. Before this, reconciliation only fired on a MISSING or
//! `NotBuilt` state record, so nothing was ever enqueued and the change took
//! effect on nothing.

use super::UnifiedJobEventHandler;
use crate::jobs::dispatcher::JobDispatcher;
use crate::spatial_state::SpatialStateStore;
use raisin_events::{NodeEvent, NodeEventKind};
use raisin_hlc::HLC;
use raisin_models::nodes::properties::{
    GeoJson, PropertyValue, SpatialPolicy, SpatialPropertySchema, SpatialWorkspaceSchema,
};
use raisin_models::nodes::Node;
use raisin_models::workspace::Workspace;
use raisin_storage::jobs::{JobContext, JobRegistry, JobType};
use raisin_storage::scope::RepoScope;
use raisin_storage::spatial::SpatialIndexState;
use raisin_storage::{Storage, WorkspaceRepository};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "t";
const REPO: &str = "r";
const BRANCH: &str = "main";
const WS: &str = "places";
const PROP: &str = "location";

struct Env {
    _dir: TempDir,
    storage: Arc<crate::RocksDBStorage>,
    registry: Arc<JobRegistry>,
    handler: UnifiedJobEventHandler,
}

impl Env {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let storage = Arc::new(crate::RocksDBStorage::new(dir.path()).unwrap());
        let registry = Arc::new(JobRegistry::new());
        let data_store = Arc::new(crate::jobs::JobDataStore::new(storage.db().clone()));
        let (dispatcher, _rx) = JobDispatcher::new();
        let handler = UnifiedJobEventHandler::new(
            storage.clone(),
            registry.clone(),
            data_store,
            Arc::new(dispatcher),
            storage.processing_rules_repository(),
        );
        Self {
            _dir: dir,
            storage,
            registry,
            handler,
        }
    }

    /// Declare a precision set on the workspace record (replicated intent).
    async fn configure(&self, precisions: Vec<usize>) {
        let mut ws = Workspace::new(WS.to_string());
        ws.config.default_branch = BRANCH.to_string();
        let mut schema = SpatialWorkspaceSchema::default();
        schema.properties.insert(
            PROP.to_string(),
            SpatialPropertySchema {
                precisions: Some(precisions),
                ..Default::default()
            },
        );
        ws.config.spatial = Some(schema);
        self.storage
            .workspaces()
            .put(RepoScope::new(TENANT, REPO), ws)
            .await
            .unwrap();
    }

    /// Pretend a build already ran under `precisions` (local reality).
    fn mark_built(&self, precisions: Vec<usize>) {
        let policy = SpatialPolicy {
            precisions,
            ..Default::default()
        };
        let state = SpatialIndexState::ready(&policy, HLC::new(1, 0));
        SpatialStateStore::new(self.storage.db().clone())
            .put(TENANT, REPO, BRANCH, WS, PROP, &state)
            .unwrap();
    }

    async fn reconcile(&self) {
        let node = geometry_node();
        let mut metadata = HashMap::new();
        metadata.insert(
            "node_data".to_string(),
            serde_json::to_value(&node).unwrap(),
        );

        let event = NodeEvent {
            tenant_id: TENANT.to_string(),
            repository_id: REPO.to_string(),
            branch: BRANCH.to_string(),
            workspace_id: WS.to_string(),
            revision: HLC::new(5, 0),
            node_id: node.id.clone(),
            node_type: Some(node.node_type.clone()),
            kind: NodeEventKind::Updated,
            path: Some(node.path.clone()),
            metadata: Some(metadata),
        };
        let context = JobContext {
            tenant_id: TENANT.to_string(),
            repo_id: REPO.to_string(),
            branch: BRANCH.to_string(),
            workspace_id: WS.to_string(),
            revision: event.revision,
            metadata: HashMap::new(),
        };

        self.handler.reconcile_spatial_index(&event, &context).await;
    }

    async fn spatial_jobs(&self) -> Vec<JobType> {
        self.registry
            .list_jobs()
            .await
            .into_iter()
            .map(|j| j.job_type)
            .filter(|t| matches!(t, JobType::SpatialIndexBuild { .. }))
            .collect()
    }
}

fn geometry_node() -> Node {
    let mut properties = HashMap::new();
    properties.insert(
        PROP.to_string(),
        PropertyValue::Geometry(GeoJson::point(8.54, 47.37)),
    );
    Node {
        id: "n1".to_string(),
        name: "n1".to_string(),
        path: "/n1".to_string(),
        node_type: "test:Place".to_string(),
        archetype: None,
        properties,
        children: Vec::new(),
        order_key: crate::fractional_index::first(),
        has_children: Some(false),
        parent: Some("/".to_string()),
        version: 1,
        created_at: Some(chrono::Utc::now()),
        updated_at: None,
        published_at: None,
        published_by: None,
        updated_by: None,
        created_by: None,
        translations: None,
        tenant_id: Some(TENANT.to_string()),
        workspace: Some(WS.to_string()),
        owner_id: None,
        relations: Vec::new(),
    }
}

/// THE regression: changing the configured precision set on an already-built
/// index must queue a REBUILD (tombstone-then-re-emit), not nothing.
#[tokio::test]
async fn a_policy_change_queues_a_rebuild() {
    let env = Env::new();
    env.mark_built(vec![9, 8, 7]);
    env.configure(vec![6, 5]).await;

    env.reconcile().await;

    let jobs = env.spatial_jobs().await;
    assert_eq!(jobs.len(), 1, "expected exactly one spatial build job");
    match &jobs[0] {
        JobType::SpatialIndexBuild {
            workspace,
            property,
            rebuild,
            ..
        } => {
            assert_eq!(workspace, WS);
            assert_eq!(property.as_deref(), Some(PROP));
            assert!(
                *rebuild,
                "a precision change must tombstone the entries it drops"
            );
        }
        other => panic!("unexpected job {:?}", other),
    }
}

/// A burst of writes under the same unchanged mismatch collapses onto one job.
#[tokio::test]
async fn repeated_events_collapse_onto_one_job() {
    let env = Env::new();
    env.mark_built(vec![9, 8, 7]);
    env.configure(vec![6, 5]).await;

    env.reconcile().await;
    env.reconcile().await;
    env.reconcile().await;

    assert_eq!(env.spatial_jobs().await.len(), 1);
}

/// Steady state: configuration and local reality agree, so nothing is queued.
#[tokio::test]
async fn a_matching_policy_queues_nothing() {
    let env = Env::new();
    env.configure(vec![9, 8]).await;
    env.mark_built(vec![9, 8]);

    env.reconcile().await;

    assert!(env.spatial_jobs().await.is_empty());
}

/// No state record at all is a gap-fill, not a rebuild: there is nothing to
/// tombstone, and a rebuild would pay the tombstone scan for no reason.
#[tokio::test]
async fn a_missing_state_record_queues_a_gap_fill() {
    let env = Env::new();
    env.configure(vec![9, 8]).await;

    env.reconcile().await;

    let jobs = env.spatial_jobs().await;
    assert_eq!(jobs.len(), 1);
    assert!(matches!(
        &jobs[0],
        JobType::SpatialIndexBuild { rebuild: false, .. }
    ));
}
