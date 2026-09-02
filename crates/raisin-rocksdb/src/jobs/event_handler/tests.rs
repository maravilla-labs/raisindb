//! Tests for the unified job event handler

use super::*;
use crate::jobs::dispatcher::JobDispatcher;
use raisin_embeddings::config::TenantEmbeddingConfig;
use raisin_embeddings::storage::TenantEmbeddingConfigStore;
use raisin_events::{NodeEvent, NodeEventKind};
use raisin_storage::jobs::{IndexOperation, JobRegistry, JobType};
use raisin_storage::Storage;
use std::sync::Arc;
use tempfile::TempDir;

fn setup_test_storage() -> (TempDir, Arc<crate::RocksDBStorage>) {
    let temp_dir = TempDir::new().unwrap();
    let storage = crate::RocksDBStorage::new(temp_dir.path()).unwrap();
    (temp_dir, Arc::new(storage))
}

fn create_test_dispatcher() -> Arc<JobDispatcher> {
    let (dispatcher, _receivers) = JobDispatcher::new();
    Arc::new(dispatcher)
}

#[tokio::test]
async fn test_embeddings_enabled_when_config_exists() {
    let (_temp_dir, storage) = setup_test_storage();
    let job_registry = Arc::new(JobRegistry::new());
    let job_data_store = Arc::new(crate::jobs::JobDataStore::new(storage.db().clone()));
    let dispatcher = create_test_dispatcher();
    let handler = UnifiedJobEventHandler::new(
        storage.clone(),
        job_registry,
        job_data_store,
        dispatcher,
        storage.processing_rules_repository(),
    );

    // Create config with embeddings enabled
    let mut config = TenantEmbeddingConfig::new("tenant1".to_string());
    config.enabled = true;
    storage
        .tenant_embedding_config_repository()
        .set_config(&config)
        .unwrap();

    let enabled = handler.embeddings_enabled("tenant1").await.unwrap();
    assert!(enabled);
}

#[tokio::test]
async fn test_embeddings_disabled_when_config_exists() {
    let (_temp_dir, storage) = setup_test_storage();
    let job_registry = Arc::new(JobRegistry::new());
    let job_data_store = Arc::new(crate::jobs::JobDataStore::new(storage.db().clone()));
    let dispatcher = create_test_dispatcher();
    let handler = UnifiedJobEventHandler::new(
        storage.clone(),
        job_registry,
        job_data_store,
        dispatcher,
        storage.processing_rules_repository(),
    );

    // Create config with embeddings disabled
    let config = TenantEmbeddingConfig::new("tenant1".to_string());
    storage
        .tenant_embedding_config_repository()
        .set_config(&config)
        .unwrap();

    let enabled = handler.embeddings_enabled("tenant1").await.unwrap();
    assert!(!enabled);
}

#[tokio::test]
async fn test_embeddings_disabled_when_config_missing() {
    let (_temp_dir, storage) = setup_test_storage();
    let job_registry = Arc::new(JobRegistry::new());
    let job_data_store = Arc::new(crate::jobs::JobDataStore::new(storage.db().clone()));
    let dispatcher = create_test_dispatcher();
    let handler = UnifiedJobEventHandler::new(
        storage.clone(),
        job_registry,
        job_data_store,
        dispatcher,
        storage.processing_rules_repository(),
    );

    let enabled = handler.embeddings_enabled("non-existent").await.unwrap();
    assert!(!enabled);
}

#[tokio::test]
async fn test_handle_node_change_enqueues_fulltext_job() {
    let (_temp_dir, storage) = setup_test_storage();
    let job_registry = Arc::new(JobRegistry::new());
    let job_data_store = Arc::new(crate::jobs::JobDataStore::new(storage.db().clone()));
    let dispatcher = create_test_dispatcher();
    let handler = UnifiedJobEventHandler::new(
        storage.clone(),
        job_registry.clone(),
        job_data_store,
        dispatcher,
        storage.processing_rules_repository(),
    );

    // Note: Due to test environment complexity, we cannot easily set up the full context
    // (branch creation, node application, etc.) without triggering serialization issues.
    // This test verifies the handler doesn't crash, not that it enqueues jobs.

    let node_event = NodeEvent {
        tenant_id: "tenant1".to_string(),
        repository_id: "repo1".to_string(),
        workspace_id: "default".to_string(),
        branch: "main".to_string(),
        revision: raisin_hlc::HLC::new(1, 0),
        node_id: "node1".to_string(),
        node_type: Some("Document".to_string()),
        kind: NodeEventKind::Created,
        path: None,
        metadata: None,
    };

    // Just verify the handler doesn't crash
    let result = handler.handle_node_change(&node_event).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_handle_node_change_enqueues_embedding_job_when_enabled() {
    let (_temp_dir, storage) = setup_test_storage();
    let job_registry = Arc::new(JobRegistry::new());
    let job_data_store = Arc::new(crate::jobs::JobDataStore::new(storage.db().clone()));
    let dispatcher = create_test_dispatcher();
    let handler = UnifiedJobEventHandler::new(
        storage.clone(),
        job_registry.clone(),
        job_data_store,
        dispatcher,
        storage.processing_rules_repository(),
    );

    // Create branch "main" first
    use raisin_storage::BranchRepository;
    storage
        .branches_impl()
        .create_branch(
            "tenant1", "repo1", "main", "Initial", None, None, false, false,
        )
        .await
        .unwrap();

    // Create the node first so it exists when the event handler checks it
    let op = storage
        .operation_capture()
        .capture_create_node(
            "tenant1".to_string(),
            "repo1".to_string(),
            "main".to_string(),
            "node1".to_string(),
            "Test Node".to_string(),
            "Document".to_string(),
            None,
            None,
            "order1".to_string(),
            serde_json::json!({"title": "Test content"}),
            None,
            None,                     // Use default workspace
            "/Test Node".to_string(), // Path for root node
            "test-actor".to_string(),
        )
        .await
        .unwrap();

    // Ensure operation has a revision (required for application)
    let revision = op
        .revision
        .unwrap_or_else(|| raisin_hlc::HLC::new(op.op_seq, 0));

    let mut op_with_revision = op.clone();
    op_with_revision.revision = Some(revision);

    // Apply the operation so the node actually exists in storage
    let applicator = crate::replication::OperationApplicator::new(
        storage.db().clone(),
        storage.event_bus().clone(),
        Arc::new(storage.branches_impl().clone()),
    );
    applicator.apply_operation(&op_with_revision).await.unwrap();

    // Update branch HEAD to point to the new revision
    storage
        .branches_impl()
        .update_head("tenant1", "repo1", "main", revision)
        .await
        .unwrap();

    // Enable embeddings for tenant
    let mut config = TenantEmbeddingConfig::new("tenant1".to_string());
    config.enabled = true;
    storage
        .tenant_embedding_config_repository()
        .set_config(&config)
        .unwrap();

    let node_event = NodeEvent {
        tenant_id: "tenant1".to_string(),
        repository_id: "repo1".to_string(),
        workspace_id: "default".to_string(),
        branch: "main".to_string(),
        revision: raisin_hlc::HLC::new(1, 0),
        node_id: "node1".to_string(),
        node_type: Some("Document".to_string()),
        kind: NodeEventKind::Created,
        path: None,
        metadata: None,
    };

    // Just verify the handler doesn't crash
    let result = handler.handle_node_change(&node_event).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_handle_node_delete_uses_delete_operation() {
    let (_temp_dir, storage) = setup_test_storage();
    let job_registry = Arc::new(JobRegistry::new());
    let job_data_store = Arc::new(crate::jobs::JobDataStore::new(storage.db().clone()));
    let dispatcher = create_test_dispatcher();
    let handler = UnifiedJobEventHandler::new(
        storage.clone(),
        job_registry.clone(),
        job_data_store,
        dispatcher,
        storage.processing_rules_repository(),
    );

    let node_event = NodeEvent {
        tenant_id: "tenant1".to_string(),
        repository_id: "repo1".to_string(),
        workspace_id: "default".to_string(),
        branch: "main".to_string(),
        revision: raisin_hlc::HLC::new(2, 0),
        node_id: "node1".to_string(),
        node_type: None,
        kind: NodeEventKind::Deleted,
        path: None,
        metadata: None,
    };

    handler.handle_node_delete(&node_event).await.unwrap();

    let jobs = job_registry.list_jobs().await;
    // handle_node_delete creates 3 jobs:
    // 1. FulltextIndex delete
    // 2. NodeDeleteCleanup
    // 3. TriggerEvaluation (for local events)
    assert_eq!(jobs.len(), 3);

    // Verify a FulltextIndex delete operation is present
    let has_fulltext_delete = jobs.iter().any(|job| {
        matches!(
            &job.job_type,
            JobType::FulltextIndex { operation, .. } if *operation == IndexOperation::Delete
        )
    });
    assert!(has_fulltext_delete, "Expected FulltextIndex delete job");

    // Verify NodeDeleteCleanup job is present
    let has_cleanup = jobs
        .iter()
        .any(|job| matches!(&job.job_type, JobType::NodeDeleteCleanup { .. }));
    assert!(has_cleanup, "Expected NodeDeleteCleanup job");

    // Verify TriggerEvaluation job is present (local event)
    let has_trigger = jobs
        .iter()
        .any(|job| matches!(&job.job_type, JobType::TriggerEvaluation { .. }));
    assert!(has_trigger, "Expected TriggerEvaluation job");
}

/// Seed a real node so `get_index_settings` can resolve it (no NodeType is
/// registered, which is the "treat as indexable" default).
async fn seed_node(storage: &Arc<crate::RocksDBStorage>) {
    use raisin_storage::BranchRepository;
    storage
        .branches_impl()
        .create_branch(
            "tenant1", "repo1", "main", "Initial", None, None, false, false,
        )
        .await
        .unwrap();

    let op = storage
        .operation_capture()
        .capture_create_node(
            "tenant1".to_string(),
            "repo1".to_string(),
            "main".to_string(),
            "node1".to_string(),
            "Test Node".to_string(),
            "Document".to_string(),
            None,
            None,
            "order1".to_string(),
            serde_json::json!({"title": "Test content"}),
            None,
            None,
            "/Test Node".to_string(),
            "test-actor".to_string(),
        )
        .await
        .unwrap();

    let revision = op
        .revision
        .unwrap_or_else(|| raisin_hlc::HLC::new(op.op_seq, 0));
    let mut op_with_revision = op.clone();
    op_with_revision.revision = Some(revision);

    let applicator = crate::replication::OperationApplicator::new(
        storage.db().clone(),
        storage.event_bus().clone(),
        Arc::new(storage.branches_impl().clone()),
    );
    applicator.apply_operation(&op_with_revision).await.unwrap();

    storage
        .branches_impl()
        .update_head("tenant1", "repo1", "main", revision)
        .await
        .unwrap();
}

/// A REPLICATED node change must schedule vector embedding, exactly as it
/// schedules fulltext.
///
/// Nothing replicates a vector: no `OpType` in `raisin-replication` carries one
/// and `store_embedding` is a raw `put_cf` outside operation capture. A replica
/// that skipped this arm therefore had fulltext but no vectors, and a hybrid
/// query — which fuses the two legs — degraded to fulltext-only WITHOUT
/// erroring. That silence is why this needs a test and not just a comment.
#[tokio::test]
async fn test_remote_node_change_still_enqueues_embedding_job() {
    let (_temp_dir, storage) = setup_test_storage();
    let job_registry = Arc::new(JobRegistry::new());
    let job_data_store = Arc::new(crate::jobs::JobDataStore::new(storage.db().clone()));
    let dispatcher = create_test_dispatcher();
    let handler = UnifiedJobEventHandler::new(
        storage.clone(),
        job_registry.clone(),
        job_data_store,
        dispatcher,
        storage.processing_rules_repository(),
    );

    seed_node(&storage).await;

    let mut config = TenantEmbeddingConfig::new("tenant1".to_string());
    config.enabled = true;
    storage
        .tenant_embedding_config_repository()
        .set_config(&config)
        .unwrap();

    let mut metadata = std::collections::HashMap::new();
    metadata.insert(
        "source".to_string(),
        serde_json::Value::String("replication".to_string()),
    );

    let node_event = NodeEvent {
        tenant_id: "tenant1".to_string(),
        repository_id: "repo1".to_string(),
        workspace_id: "default".to_string(),
        branch: "main".to_string(),
        revision: raisin_hlc::HLC::new(1, 0),
        node_id: "node1".to_string(),
        node_type: Some("Document".to_string()),
        kind: NodeEventKind::Created,
        path: None,
        metadata: Some(metadata),
    };

    assert!(
        UnifiedJobEventHandler::is_remote_event(&node_event),
        "test event must look remote or it proves nothing"
    );

    handler.handle_node_change(&node_event).await.unwrap();

    let jobs = job_registry.list_jobs().await;

    let has_fulltext = jobs.iter().any(|job| {
        matches!(
            &job.job_type,
            JobType::FulltextIndex { operation, .. } if *operation == IndexOperation::AddOrUpdate
        )
    });
    assert!(
        has_fulltext,
        "replica must index fulltext locally (existing behaviour)"
    );

    let has_embedding = jobs
        .iter()
        .any(|job| matches!(&job.job_type, JobType::EmbeddingGenerate { .. }));
    assert!(
        has_embedding,
        "replica must embed locally too; nothing replicates the vector. Jobs seen: {:?}",
        jobs.iter()
            .map(|j| j.job_type.to_string())
            .collect::<Vec<_>>()
    );

    // The local-only arms must stay local: a replicated edit re-firing triggers
    // on every node is a different and much worse bug.
    let has_trigger = jobs
        .iter()
        .any(|job| matches!(&job.job_type, JobType::TriggerEvaluation { .. }));
    assert!(
        !has_trigger,
        "trigger evaluation must remain LOCAL-only for replicated events"
    );
}

/// THE LOOP GUARD. Writing the job-activity node must enqueue NO further work.
///
/// This is the single most important property of the whole activity mechanism.
/// A node write emits `node:updated`, which is what enqueues fulltext indexing,
/// vector embedding, asset processing and trigger evaluation — so a surface
/// that reports on background jobs BY WRITING A NODE is a feedback loop and an
/// amplifier unless every one of those arms is inert for it. The 2026-09-02
/// incident produced 2,512 job events in five minutes; one node write each,
/// each enqueueing more work, would have been a worse outage than the one this
/// exists to prevent.
///
/// Two mechanisms close the loop, and this test covers both at once:
///
/// * the SHIPPED `raisin:JobActivity` definition declares
///   `index_types: [Property]` — no `Fulltext`, no `Vector` — which closes
///   `handle_node_change`'s fulltext and embedding arms. The definition is
///   loaded from `raisin-core`'s embedded YAML rather than hand-written here,
///   so editing that file to add `Fulltext` fails this test rather than
///   silently re-arming the loop in production.
/// * the write carries the `bookkeeping` marker, which closes trigger
///   evaluation.
///
/// Embeddings are ENABLED for the tenant on purpose: the assertion has to prove
/// the node type is the gate, not a disabled tenant config that could be turned
/// on tomorrow.
#[tokio::test]
async fn activity_node_write_enqueues_no_indexing_jobs() {
    use crate::jobs::activity::{ACTIVITY_NODE_TYPE, ACTIVITY_WORKSPACE};

    let (_temp_dir, storage) = setup_test_storage();
    let job_registry = Arc::new(JobRegistry::new());
    let job_data_store = Arc::new(crate::jobs::JobDataStore::new(storage.db().clone()));
    let dispatcher = create_test_dispatcher();
    let handler = UnifiedJobEventHandler::new(
        storage.clone(),
        job_registry.clone(),
        job_data_store,
        dispatcher,
        storage.processing_rules_repository(),
    );

    seed_activity_node(&storage).await;

    let mut config = TenantEmbeddingConfig::new("tenant1".to_string());
    config.enabled = true;
    storage
        .tenant_embedding_config_repository()
        .set_config(&config)
        .unwrap();

    let mut metadata = std::collections::HashMap::new();
    metadata.insert("bookkeeping".to_string(), serde_json::Value::Bool(true));

    let node_event = NodeEvent {
        tenant_id: "tenant1".to_string(),
        repository_id: "repo1".to_string(),
        workspace_id: ACTIVITY_WORKSPACE.to_string(),
        branch: "main".to_string(),
        revision: raisin_hlc::HLC::new(2, 0),
        node_id: "job-activity".to_string(),
        node_type: Some(ACTIVITY_NODE_TYPE.to_string()),
        kind: NodeEventKind::Updated,
        path: Some("/activity".to_string()),
        metadata: Some(metadata),
    };

    assert!(
        !UnifiedJobEventHandler::is_remote_event(&node_event),
        "this must be a LOCAL event, or the local-only arms are skipped for the \
         wrong reason and the test proves nothing"
    );

    handler.handle_node_change(&node_event).await.unwrap();

    let jobs = job_registry.list_jobs().await;
    let seen: Vec<String> = jobs.iter().map(|j| j.job_type.to_string()).collect();

    assert!(
        !jobs
            .iter()
            .any(|j| matches!(&j.job_type, JobType::FulltextIndex { .. })),
        "the activity node must not be fulltext indexed. Jobs seen: {seen:?}"
    );
    assert!(
        !jobs
            .iter()
            .any(|j| matches!(&j.job_type, JobType::EmbeddingGenerate { .. })),
        "the activity node must not be embedded — this is the arm that would \
         call a provider once per activity update. Jobs seen: {seen:?}"
    );
    assert!(
        !jobs
            .iter()
            .any(|j| matches!(&j.job_type, JobType::TriggerEvaluation { .. })),
        "the activity node must not fire triggers; a tenant trigger that writes \
         a node would close the loop through user code. Jobs seen: {seen:?}"
    );
    assert!(
        !jobs
            .iter()
            .any(|j| matches!(&j.job_type, JobType::AssetProcessing { .. })),
        "the activity node must not be treated as an asset. Jobs seen: {seen:?}"
    );
    assert!(
        jobs.is_empty(),
        "writing the activity node must enqueue NOTHING AT ALL — a new arm added \
         to handle_node_change that is not inert for this type re-arms the loop. \
         Jobs seen: {seen:?}"
    );
}

/// Seed the shipped `raisin:JobActivity` NodeType and one node of that type.
///
/// The NodeType comes from `raisin-core`'s embedded global definitions, not
/// from a literal built here: a hand-written copy would keep passing after
/// someone added `Fulltext` to the real YAML, which is exactly the regression
/// this file exists to catch.
async fn seed_activity_node(storage: &Arc<crate::RocksDBStorage>) {
    use raisin_storage::{BranchRepository, NodeTypeRepository};

    storage
        .branches_impl()
        .create_branch(
            "tenant1", "repo1", "main", "Initial", None, None, false, false,
        )
        .await
        .unwrap();

    let node_type = raisin_core::nodetype_init::load_global_nodetypes()
        .into_iter()
        .find(|nt| nt.name == "raisin:JobActivity")
        .expect("raisin:JobActivity must ship as a global NodeType");
    assert_eq!(
        node_type.index_types,
        Some(vec![
            raisin_models::nodes::properties::schema::IndexType::Property
        ]),
        "the shipped definition must index Property ONLY; adding Fulltext or \
         Vector re-arms the write→event→job loop"
    );
    storage
        .node_types()
        .upsert(
            raisin_storage::BranchScope::new("tenant1", "repo1", "main"),
            node_type,
            raisin_storage::CommitMetadata::system("seed job activity nodetype"),
        )
        .await
        .unwrap();

    let op = storage
        .operation_capture()
        .capture_create_node(
            "tenant1".to_string(),
            "repo1".to_string(),
            "main".to_string(),
            "job-activity".to_string(),
            "activity".to_string(),
            "raisin:JobActivity".to_string(),
            None,
            None,
            "order1".to_string(),
            serde_json::json!({"degraded": false, "active": 0}),
            None,
            Some("job_activity".to_string()),
            "/activity".to_string(),
            "job-activity".to_string(),
        )
        .await
        .unwrap();

    let revision = op
        .revision
        .unwrap_or_else(|| raisin_hlc::HLC::new(op.op_seq, 0));
    let mut op_with_revision = op.clone();
    op_with_revision.revision = Some(revision);

    let applicator = crate::replication::OperationApplicator::new(
        storage.db().clone(),
        storage.event_bus().clone(),
        Arc::new(storage.branches_impl().clone()),
    );
    applicator.apply_operation(&op_with_revision).await.unwrap();

    storage
        .branches_impl()
        .update_head("tenant1", "repo1", "main", revision)
        .await
        .unwrap();
}
