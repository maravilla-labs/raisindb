//! Simple integration tests for operation capture functionality
//!
//! Tests that operations are correctly captured for replication

use raisin_hlc::HLC;
use raisin_models::nodes::{
    element::element_type::ElementType,
    types::{archetype::Archetype, node_type::NodeType},
};
use raisin_replication::OpType;
use raisin_rocksdb::{RocksDBConfig, RocksDBStorage};
use std::sync::Arc;
use tempfile::TempDir;

fn create_test_storage() -> (TempDir, Arc<RocksDBStorage>) {
    let temp_dir = TempDir::new().unwrap();
    let mut config = RocksDBConfig::default();
    config.path = temp_dir.path().to_path_buf();
    config.replication_enabled = true;
    config.cluster_node_id = Some("test_node".to_string());
    let storage = Arc::new(RocksDBStorage::with_config(config).unwrap());
    (temp_dir, storage)
}

#[tokio::test]
async fn test_operation_capture_enabled() {
    let (_dir, storage) = create_test_storage();

    // Verify operation capture is enabled
    assert!(storage.operation_capture().is_enabled());

    // Verify cluster node ID is set
    assert_eq!(storage.operation_capture().cluster_node_id(), "test_node");
}

#[tokio::test]
async fn test_direct_operation_capture() {
    let (_dir, storage) = create_test_storage();
    let operation_capture = storage.operation_capture().clone();

    let tenant_id = "test_tenant";
    let repo_id = "test_repo";
    let branch = "main";

    // Directly capture a SetTranslation operation
    let result = operation_capture
        .capture_set_translation(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "node1".to_string(),
            "en".to_string(),
            "title".to_string(),
            serde_json::json!({"text": "Hello World"}),
            "test_user".to_string(),
        )
        .await;

    assert!(result.is_ok());
    let op = result.unwrap();

    // Verify operation properties
    assert_eq!(op.cluster_node_id, "test_node");
    assert_eq!(op.tenant_id, tenant_id);
    assert_eq!(op.repo_id, repo_id);
    assert_eq!(op.branch, branch);
    assert_eq!(op.actor, "test_user");
    assert_eq!(op.op_seq, 1);

    // Verify it's a SetTranslation operation
    assert!(matches!(op.op_type, OpType::SetTranslation { .. }));
}

#[tokio::test]
async fn test_schema_operation_capture() {
    let (_dir, storage) = create_test_storage();
    let operation_capture = storage.operation_capture().clone();

    let tenant_id = "test_tenant";
    let repo_id = "test_repo";
    let branch = "main";

    // Capture UpdateNodeType operation
    let node_type: NodeType =
        serde_json::from_value(serde_json::json!({"name": "Article", "version": 1})).unwrap();
    let revision = HLC::now();
    let result = operation_capture
        .capture_upsert_nodetype(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "Article".to_string(),
            node_type,
            "test_user".to_string(),
            revision,
        )
        .await;

    assert!(result.is_ok());
    let op = result.unwrap();

    assert!(matches!(op.op_type, OpType::UpdateNodeType { .. }));
    assert_eq!(op.op_seq, 1);
}

#[tokio::test]
async fn test_delete_nodetype_capture() {
    let (_dir, storage) = create_test_storage();
    let operation_capture = storage.operation_capture().clone();

    let tenant_id = "test_tenant";
    let repo_id = "test_repo";
    let branch = "main";

    // Capture DeleteNodeType operation
    let revision = HLC::now();
    let result = operation_capture
        .capture_delete_nodetype(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "OldType".to_string(),
            "test_user".to_string(),
            revision,
        )
        .await;

    assert!(result.is_ok());
    let op = result.unwrap();

    assert!(matches!(op.op_type, OpType::DeleteNodeType { .. }));
    assert_eq!(op.op_seq, 1);
}

#[tokio::test]
async fn test_user_operation_capture() {
    let (_dir, storage) = create_test_storage();
    let operation_capture = storage.operation_capture().clone();

    let tenant_id = "test_tenant";

    // Capture UpdateUser operation
    let result = operation_capture
        .capture_update_user(
            tenant_id.to_string(),
            "system".to_string(),
            "main".to_string(),
            "john_doe".to_string(),
            serde_json::json!({
                "user_id": "john_doe",
                "username": "john_doe",
                "email": "john@example.com",
                "password_hash": "$2b$12$abcdefghijklmnopqrstuv",
                "tenant_id": tenant_id,
                "access_flags": {
                    "console_login": true,
                    "cli_access": true,
                    "api_access": true
                },
                "must_change_password": false,
                "created_at": "2024-01-01T00:00:00Z",
                "last_login": null,
                "is_active": true
            }),
            "admin".to_string(),
        )
        .await;

    assert!(result.is_ok());
    let op = result.unwrap();

    assert!(matches!(op.op_type, OpType::UpdateUser { .. }));
    assert_eq!(op.op_seq, 1);
}

#[tokio::test]
async fn test_delete_user_capture() {
    let (_dir, storage) = create_test_storage();
    let operation_capture = storage.operation_capture().clone();

    let tenant_id = "test_tenant";

    // Capture DeleteUser operation
    let result = operation_capture
        .capture_delete_user(
            tenant_id.to_string(),
            "system".to_string(),
            "main".to_string(),
            "old_user".to_string(),
            "admin".to_string(),
        )
        .await;

    assert!(result.is_ok());
    let op = result.unwrap();

    assert!(matches!(op.op_type, OpType::DeleteUser { .. }));
    assert_eq!(op.op_seq, 1);
}

#[tokio::test]
async fn test_multiple_operations_sequence() {
    let (_dir, storage) = create_test_storage();
    let operation_capture = storage.operation_capture().clone();

    let tenant_id = "test_tenant";
    let repo_id = "test_repo";
    let branch = "main";

    // Capture multiple operations
    let revision1 = HLC::now();
    let node_type1: NodeType =
        serde_json::from_value(serde_json::json!({"name": "Type1"})).unwrap();
    let op1 = operation_capture
        .capture_upsert_nodetype(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "Type1".to_string(),
            node_type1,
            "user".to_string(),
            revision1,
        )
        .await
        .unwrap();

    let revision2 = HLC::now();
    let archetype1: Archetype =
        serde_json::from_value(serde_json::json!({"name": "Archetype1"})).unwrap();
    let op2 = operation_capture
        .capture_upsert_archetype(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "Archetype1".to_string(),
            archetype1,
            "user".to_string(),
            revision2,
        )
        .await
        .unwrap();

    let revision3 = HLC::now();
    let element_type1: ElementType =
        serde_json::from_value(serde_json::json!({"name": "Element1"})).unwrap();
    let op3 = operation_capture
        .capture_upsert_element_type(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "Element1".to_string(),
            element_type1,
            "user".to_string(),
            revision3,
        )
        .await
        .unwrap();

    // Verify sequence numbers increment
    assert_eq!(op1.op_seq, 1);
    assert_eq!(op2.op_seq, 2);
    assert_eq!(op3.op_seq, 3);

    // Verify all have same cluster node ID
    assert_eq!(op1.cluster_node_id, "test_node");
    assert_eq!(op2.cluster_node_id, "test_node");
    assert_eq!(op3.cluster_node_id, "test_node");
}

#[tokio::test]
async fn test_operation_persistence() {
    let (_dir, storage) = create_test_storage();
    let operation_capture = storage.operation_capture().clone();

    let tenant_id = "test_tenant";
    let repo_id = "test_repo";
    let branch = "main";

    // Capture some operations
    operation_capture
        .capture_set_translation(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "node1".to_string(),
            "en".to_string(),
            "title".to_string(),
            serde_json::json!("Hello"),
            "user".to_string(),
        )
        .await
        .unwrap();

    operation_capture
        .capture_delete_translation(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "node2".to_string(),
            "fr".to_string(),
            "description".to_string(),
            "user".to_string(),
        )
        .await
        .unwrap();

    // Retrieve operations from storage
    let ops = operation_capture
        .oplog_repo()
        .get_operations_from_node(tenant_id, repo_id, "test_node")
        .unwrap();

    assert_eq!(ops.len(), 2);
    assert!(matches!(ops[0].op_type, OpType::SetTranslation { .. }));
    assert!(matches!(ops[1].op_type, OpType::DeleteTranslation { .. }));
}

#[tokio::test]
async fn test_all_schema_operations() {
    let (_dir, storage) = create_test_storage();
    let operation_capture = storage.operation_capture().clone();

    let tenant_id = "test_tenant";
    let repo_id = "test_repo";
    let branch = "main";

    // Test all schema operation types

    // NodeType operations
    let rev1 = HLC::now();
    let node_type2: NodeType =
        serde_json::from_value(serde_json::json!({"name": "TestType"})).unwrap();
    let op1 = operation_capture
        .capture_upsert_nodetype(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "TestType".to_string(),
            node_type2,
            "user".to_string(),
            rev1,
        )
        .await
        .unwrap();
    assert!(matches!(op1.op_type, OpType::UpdateNodeType { .. }));

    let rev2 = HLC::now();
    let op2 = operation_capture
        .capture_delete_nodetype(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "TestType".to_string(),
            "user".to_string(),
            rev2,
        )
        .await
        .unwrap();
    assert!(matches!(op2.op_type, OpType::DeleteNodeType { .. }));

    // Archetype operations
    let rev3 = HLC::now();
    let archetype2: Archetype =
        serde_json::from_value(serde_json::json!({"name": "TestArchetype"})).unwrap();
    let op3 = operation_capture
        .capture_upsert_archetype(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "TestArchetype".to_string(),
            archetype2,
            "user".to_string(),
            rev3,
        )
        .await
        .unwrap();
    assert!(matches!(op3.op_type, OpType::UpdateArchetype { .. }));

    let rev4 = HLC::now();
    let op4 = operation_capture
        .capture_delete_archetype(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "TestArchetype".to_string(),
            "user".to_string(),
            rev4,
        )
        .await
        .unwrap();
    assert!(matches!(op4.op_type, OpType::DeleteArchetype { .. }));

    // ElementType operations
    let rev5 = HLC::now();
    let element_type2: ElementType =
        serde_json::from_value(serde_json::json!({"name": "TestElement"})).unwrap();
    let op5 = operation_capture
        .capture_upsert_element_type(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "TestElement".to_string(),
            element_type2,
            "user".to_string(),
            rev5,
        )
        .await
        .unwrap();
    assert!(matches!(op5.op_type, OpType::UpdateElementType { .. }));

    let rev6 = HLC::now();
    let op6 = operation_capture
        .capture_delete_element_type(
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            "TestElement".to_string(),
            "user".to_string(),
            rev6,
        )
        .await
        .unwrap();
    assert!(matches!(op6.op_type, OpType::DeleteElementType { .. }));

    // Verify all were captured
    let ops = operation_capture
        .oplog_repo()
        .get_operations_from_node(tenant_id, repo_id, "test_node")
        .unwrap();

    assert_eq!(ops.len(), 6);
}
