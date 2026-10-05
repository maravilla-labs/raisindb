//! Plan Phase 13e: a workspace event sweeps the workspace's OWN compound
//! indexes too, and the build job it queues builds them over every node type.

use super::*;
use crate::jobs::dispatcher::JobDispatcher;
use crate::jobs::handlers::compound_index::CompoundIndexJobHandler;
use crate::repositories::{BranchRepositoryImpl, RevisionRepositoryImpl};
use raisin_events::{WorkspaceEvent, WorkspaceEventKind};
use raisin_models::nodes::properties::schema::{
    CompoundColumnType, CompoundIndexColumn, CompoundIndexDefinition,
};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::compound::CompoundBuildPhase;
use raisin_storage::jobs::{JobContext, JobType};
use raisin_storage::{
    BranchRepository, CompoundColumnValue, CompoundIndexRepository, CreateNodeOptions,
    NodeRepository, RepoScope, Storage, StorageScope, WorkspaceRepository,
};
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::test]
async fn workspace_event_queues_and_the_job_builds_a_workspace_index() {
    let dir = tempfile::TempDir::new().unwrap();
    let storage = Arc::new(crate::RocksDBStorage::new(dir.path()).unwrap());
    let (dispatcher, _receivers) = JobDispatcher::new();
    let handler = UnifiedJobEventHandler::new(
        storage.clone(),
        storage.job_registry().clone(),
        Arc::new(crate::jobs::JobDataStore::new(storage.db().clone())),
        Arc::new(dispatcher),
        storage.processing_rules_repository(),
    );
    storage
        .branches()
        .create_branch("t", "r", "main", "test", None, None, false, false)
        .await
        .unwrap();
    let mut workspace = raisin_models::workspace::Workspace::new("feed".to_string());
    workspace.compound_indexes = Some(vec![CompoundIndexDefinition {
        name: "by_cat".to_string(),
        columns: vec![CompoundIndexColumn {
            property: "cat".to_string(),
            column_type: CompoundColumnType::String,
            ascending: None,
        }],
        has_order_column: false,
        owner: None,
    }]);
    storage
        .workspaces()
        .put(RepoScope::new("t", "r"), workspace)
        .await
        .unwrap();
    // Two types, neither with a NodeType record or a compound declaration.
    for (id, node_type) in [("a", "app:Post"), ("b", "app:Note")] {
        let node = Node {
            id: id.to_string(),
            name: id.to_string(),
            path: format!("/{id}"),
            node_type: node_type.to_string(),
            properties: HashMap::from([(
                "cat".to_string(),
                PropertyValue::String("x".to_string()),
            )]),
            ..Node::default()
        };
        storage
            .nodes()
            .create(
                StorageScope::new("t", "r", "main", "feed"),
                node,
                CreateNodeOptions {
                    validate_schema: false,
                    validate_parent_allows_child: false,
                    validate_workspace_allows_type: false,
                    operation_meta: None,
                },
            )
            .await
            .unwrap();
    }

    handler
        .handle_workspace_change(&WorkspaceEvent {
            tenant_id: "t".into(),
            repository_id: "r".into(),
            workspace: "feed".into(),
            kind: WorkspaceEventKind::Updated,
            metadata: None,
        })
        .await
        .unwrap();
    let job = storage
        .job_registry()
        .list_jobs()
        .await
        .into_iter()
        .find(|job| {
            matches!(&job.job_type, JobType::CompoundIndexBuild { index_name, .. }
                if index_name == "@by_cat")
        })
        .expect("the workspace event queues a build of the workspace's index");

    let db = storage.db().clone();
    CompoundIndexJobHandler::new(
        db.clone(),
        Arc::new(RevisionRepositoryImpl::new(db.clone(), "local".to_string())),
        Arc::new(BranchRepositoryImpl::new(db)),
    )
    .handle(
        &job,
        &JobContext {
            tenant_id: "t".into(),
            repo_id: "r".into(),
            branch: "main".into(),
            workspace_id: "feed".into(),
            revision: raisin_hlc::HLC::new(0, 0),
            metadata: HashMap::new(),
        },
    )
    .await
    .unwrap();
    let state =
        crate::compound_state::read_state(storage.db(), "t", "r", "main", "feed", "@by_cat")
            .unwrap()
            .expect("record");
    assert_eq!(state.phase, CompoundBuildPhase::Ready);
    let mut ids: Vec<String> = storage
        .compound_index()
        .scan_compound_index(
            StorageScope::new("t", "r", "main", "feed"),
            "@by_cat",
            &[CompoundColumnValue::String("x".to_string())],
            false,
            true,
            None,
            None,
        )
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.node_id)
        .collect();
    ids.sort();
    assert_eq!(ids, ["a", "b"], "every node type is in the workspace index");
}
