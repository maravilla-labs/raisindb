//! Permission Resolution Tests
//!
//! Tests for the PermissionService's ability to resolve user permissions from:
//! - Direct roles
//! - Group roles
//! - Role inheritance
//! - Cycle detection
//! - Deduplication

use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_core::PermissionService;
use raisin_error::Result;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::workspace::Workspace;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{
    BranchRepository, RegistryRepository, RepositoryManagementRepository, Storage,
};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "test-tenant";
const REPO: &str = "test-repo";
const BRANCH: &str = "main";
const ACCESS_CONTROL_WS: &str = "raisin:access_control";

/// Setup storage with tenant, repo, branch, and access_control workspace
async fn setup_storage() -> Result<(Arc<RocksDBStorage>, TempDir)> {
    let temp_dir = tempfile::tempdir().map_err(|e| raisin_error::Error::Backend(e.to_string()))?;
    let storage = RocksDBStorage::new(temp_dir.path())?;
    let storage = Arc::new(storage);

    // Initialize tenant
    let registry = storage.registry();
    registry.register_tenant(TENANT, HashMap::new()).await?;

    // Create repository
    let repo_mgmt = storage.repository_management();
    let repo_config = RepositoryConfig {
        default_branch: BRANCH.to_string(),
        description: Some("Permission resolution test repository".to_string()),
        tags: HashMap::new(),
        default_language: "en".to_string(),
        supported_languages: vec!["en".to_string()],
        locale_fallback_chains: HashMap::new(),
    };
    repo_mgmt
        .create_repository(TENANT, REPO, repo_config)
        .await?;

    // Create branch
    let branches = storage.branches();
    branches
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await?;

    // Create access_control workspace with ROOT node
    let workspace = Workspace::new(ACCESS_CONTROL_WS.to_string());
    let workspace_service = WorkspaceService::new(storage.clone());
    workspace_service.put(TENANT, REPO, workspace).await?;

    Ok((storage, temp_dir))
}

/// Helper to create a User node in access_control workspace
fn create_user_node(user_id: &str, email: &str, roles: Vec<&str>, groups: Vec<&str>) -> Node {
    let mut props = HashMap::new();
    props.insert(
        "email".to_string(),
        PropertyValue::String(email.to_string()),
    );
    props.insert(
        "roles".to_string(),
        PropertyValue::Array(
            roles
                .iter()
                .map(|r| PropertyValue::String(r.to_string()))
                .collect(),
        ),
    );
    props.insert(
        "groups".to_string(),
        PropertyValue::Array(
            groups
                .iter()
                .map(|g| PropertyValue::String(g.to_string()))
                .collect(),
        ),
    );

    Node {
        id: user_id.to_string(),
        name: user_id.to_string(),
        path: format!("/users/{}", user_id),
        node_type: "raisin:User".to_string(),
        archetype: None,
        properties: props,
        children: vec![],
        order_key: String::new(),
        has_children: None,
        parent: Some("users".to_string()),
        version: 1,
        created_at: Some(chrono::Utc::now()),
        updated_at: Some(chrono::Utc::now()),
        published_at: None,
        published_by: None,
        updated_by: Some("test-user".to_string()),
        created_by: Some("test-user".to_string()),
        translations: None,
        tenant_id: Some(TENANT.to_string()),
        workspace: Some(ACCESS_CONTROL_WS.to_string()),
        owner_id: None,
        relations: Vec::new(),
    }
}

/// Helper to create a Group node
fn create_group_node(group_id: &str, group_name: &str, roles: Vec<&str>) -> Node {
    let mut props = HashMap::new();
    props.insert(
        "group_id".to_string(),
        PropertyValue::String(group_id.to_string()),
    );
    props.insert(
        "name".to_string(),
        PropertyValue::String(group_name.to_string()),
    );
    props.insert(
        "roles".to_string(),
        PropertyValue::Array(
            roles
                .iter()
                .map(|r| PropertyValue::String(r.to_string()))
                .collect(),
        ),
    );

    Node {
        id: group_id.to_string(),
        name: group_id.to_string(),
        path: format!("/groups/{}", group_id),
        node_type: "raisin:Group".to_string(),
        archetype: None,
        properties: props,
        children: vec![],
        order_key: String::new(),
        has_children: None,
        parent: Some("groups".to_string()),
        version: 1,
        created_at: Some(chrono::Utc::now()),
        updated_at: Some(chrono::Utc::now()),
        published_at: None,
        published_by: None,
        updated_by: Some("test-user".to_string()),
        created_by: Some("test-user".to_string()),
        translations: None,
        tenant_id: Some(TENANT.to_string()),
        workspace: Some(ACCESS_CONTROL_WS.to_string()),
        owner_id: None,
        relations: Vec::new(),
    }
}

/// Helper to create a Role node with permissions
fn create_role_node(
    role_id: &str,
    inherits: Vec<&str>,
    permissions: Vec<serde_json::Value>,
) -> Node {
    let mut props = HashMap::new();
    props.insert(
        "role_id".to_string(),
        PropertyValue::String(role_id.to_string()),
    );
    props.insert(
        "name".to_string(),
        PropertyValue::String(format!("Role {}", role_id)),
    );
    props.insert(
        "inherits".to_string(),
        PropertyValue::Array(
            inherits
                .iter()
                .map(|r| PropertyValue::String(r.to_string()))
                .collect(),
        ),
    );

    // Convert serde_json::Value permissions to PropertyValue
    let perm_values: Vec<PropertyValue> = permissions
        .into_iter()
        .map(|p| json_to_property_value(p))
        .collect();
    props.insert("permissions".to_string(), PropertyValue::Array(perm_values));

    Node {
        id: role_id.to_string(),
        name: role_id.to_string(),
        path: format!("/roles/{}", role_id),
        node_type: "raisin:Role".to_string(),
        archetype: None,
        properties: props,
        children: vec![],
        order_key: String::new(),
        has_children: None,
        parent: Some("roles".to_string()),
        version: 1,
        created_at: Some(chrono::Utc::now()),
        updated_at: Some(chrono::Utc::now()),
        published_at: None,
        published_by: None,
        updated_by: Some("test-user".to_string()),
        created_by: Some("test-user".to_string()),
        translations: None,
        tenant_id: Some(TENANT.to_string()),
        workspace: Some(ACCESS_CONTROL_WS.to_string()),
        owner_id: None,
        relations: Vec::new(),
    }
}

/// Create parent folder nodes (users, groups, roles)
fn create_folder_node(name: &str) -> Node {
    Node {
        id: name.to_string(),
        name: name.to_string(),
        path: format!("/{}", name),
        node_type: "raisin:AclFolder".to_string(),
        archetype: None,
        properties: HashMap::new(),
        children: vec![],
        order_key: String::new(),
        has_children: None,
        parent: Some("/".to_string()),
        version: 1,
        created_at: Some(chrono::Utc::now()),
        updated_at: Some(chrono::Utc::now()),
        published_at: None,
        published_by: None,
        updated_by: Some("test-user".to_string()),
        created_by: Some("test-user".to_string()),
        translations: None,
        tenant_id: Some(TENANT.to_string()),
        workspace: Some(ACCESS_CONTROL_WS.to_string()),
        owner_id: None,
        relations: Vec::new(),
    }
}

/// Convert serde_json::Value to PropertyValue
fn json_to_property_value(value: serde_json::Value) -> PropertyValue {
    match value {
        serde_json::Value::Null => PropertyValue::Null,
        serde_json::Value::Bool(b) => PropertyValue::Boolean(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                PropertyValue::Integer(i)
            } else if let Some(f) = n.as_f64() {
                PropertyValue::Float(f)
            } else {
                PropertyValue::Null
            }
        }
        serde_json::Value::String(s) => PropertyValue::String(s),
        serde_json::Value::Array(arr) => {
            PropertyValue::Array(arr.into_iter().map(json_to_property_value).collect())
        }
        serde_json::Value::Object(obj) => {
            let map: HashMap<String, PropertyValue> = obj
                .into_iter()
                .map(|(k, v)| (k, json_to_property_value(v)))
                .collect();
            PropertyValue::Object(map)
        }
    }
}

/// Create a simple permission JSON
fn permission_json(path: &str, operations: Vec<&str>) -> serde_json::Value {
    serde_json::json!({
        "path": path,
        "operations": operations
    })
}

/// Create the parent folder structure in access_control workspace
async fn setup_folder_structure(storage: &Arc<RocksDBStorage>) -> Result<()> {
    let nodes_impl = storage.nodes_impl();

    // Create users folder
    let users_folder = create_folder_node("users");
    nodes_impl
        .add(TENANT, REPO, BRANCH, ACCESS_CONTROL_WS, users_folder)
        .await?;

    // Create groups folder
    let groups_folder = create_folder_node("groups");
    nodes_impl
        .add(TENANT, REPO, BRANCH, ACCESS_CONTROL_WS, groups_folder)
        .await?;

    // Create roles folder
    let roles_folder = create_folder_node("roles");
    nodes_impl
        .add(TENANT, REPO, BRANCH, ACCESS_CONTROL_WS, roles_folder)
        .await?;

    Ok(())
}

// ============================================================================
// Test 1: Direct role resolution
// ============================================================================

#[tokio::test]
async fn test_direct_role_resolution() {
    let (storage, _temp_dir) = setup_storage().await.expect("Failed to setup storage");

    // Create folder structure
    setup_folder_structure(&storage)
        .await
        .expect("Failed to setup folders");

    // Create a role with read permission on articles
    let role = create_role_node(
        "reader",
        vec![],
        vec![permission_json("content.articles.**", vec!["read"])],
    );
    storage
        .nodes_impl()
        .add(TENANT, REPO, BRANCH, ACCESS_CONTROL_WS, role)
        .await
        .expect("Failed to create role");

    // Create a user with the reader role directly
    let user = create_user_node("user1", "user1@test.com", vec!["reader"], vec![]);
    storage
        .nodes_impl()
        .add(TENANT, REPO, BRANCH, ACCESS_CONTROL_WS, user)
        .await
        .expect("Failed to create user");

    // Resolve permissions
    let service = PermissionService::new(storage.clone());
    let result = service
        .resolve_for_user_id(TENANT, REPO, BRANCH, "user1")
        .await
        .expect("Failed to resolve permissions");

    assert!(result.is_some(), "User should be found");
    let resolved = result.unwrap();

    assert_eq!(resolved.user_id, "user1");
    assert_eq!(resolved.direct_roles, vec!["reader"]);
    assert_eq!(resolved.effective_roles.len(), 1);
    assert!(resolved.effective_roles.contains(&"reader".to_string()));
    assert_eq!(resolved.permissions.len(), 1);
    assert_eq!(resolved.permissions[0].path, "content.articles.**");
}

// ============================================================================
// Test 2: Group role aggregation
// ============================================================================

#[tokio::test]
async fn test_group_role_aggregation() {
    let (storage, _temp_dir) = setup_storage().await.expect("Failed to setup storage");

    // Create folder structure
    setup_folder_structure(&storage)
        .await
        .expect("Failed to setup folders");

    // Create role
    let editor_role = create_role_node(
        "editor",
        vec![],
        vec![permission_json("content.**", vec!["read", "update"])],
    );
    storage
        .nodes_impl()
        .add(TENANT, REPO, BRANCH, ACCESS_CONTROL_WS, editor_role)
        .await
        .expect("Failed to create role");

    // Create a group with the editor role
    // Note: groups are looked up by "name" property, so use the group name
    let group = create_group_node("editors-group", "editors", vec!["editor"]);
    storage
        .nodes_impl()
        .add(TENANT, REPO, BRANCH, ACCESS_CONTROL_WS, group)
        .await
        .expect("Failed to create group");

    // Create a user in the editors group (no direct roles)
    let user = create_user_node("user2", "user2@test.com", vec![], vec!["editors"]);
    storage
        .nodes_impl()
        .add(TENANT, REPO, BRANCH, ACCESS_CONTROL_WS, user)
        .await
        .expect("Failed to create user");

    // Resolve permissions
    let service = PermissionService::new(storage.clone());
    let result = service
        .resolve_for_user_id(TENANT, REPO, BRANCH, "user2")
        .await
        .expect("Failed to resolve permissions");

    assert!(result.is_some(), "User should be found");
    let resolved = result.unwrap();

    assert_eq!(resolved.user_id, "user2");
    assert!(resolved.direct_roles.is_empty(), "No direct roles");
    assert_eq!(resolved.groups, vec!["editors"]);
    assert_eq!(resolved.group_roles, vec!["editor"]);
    assert!(resolved.effective_roles.contains(&"editor".to_string()));
    assert_eq!(resolved.permissions.len(), 1);
}

// ============================================================================
// Test 3: Role inheritance chain
// ============================================================================

#[tokio::test]
async fn test_role_inheritance_chain() {
    let (storage, _temp_dir) = setup_storage().await.expect("Failed to setup storage");

    // Create folder structure
    setup_folder_structure(&storage)
        .await
        .expect("Failed to setup folders");

    // Create a chain: super_admin inherits admin, admin inherits editor, editor inherits reader
    let reader_role = create_role_node("reader", vec![], vec![permission_json("**", vec!["read"])]);
    let editor_role = create_role_node(
        "editor",
        vec!["reader"],
        vec![permission_json("content.**", vec!["update"])],
    );
    let admin_role = create_role_node(
        "admin",
        vec!["editor"],
        vec![permission_json("**", vec!["delete"])],
    );
    let super_admin_role = create_role_node(
        "super_admin",
        vec!["admin"],
        vec![permission_json(
            "system.**",
            vec!["create", "read", "update", "delete"],
        )],
    );

    for role in [reader_role, editor_role, admin_role, super_admin_role] {
        storage
            .nodes_impl()
            .add(TENANT, REPO, BRANCH, ACCESS_CONTROL_WS, role)
            .await
            .expect("Failed to create role");
    }

    // Create user with only super_admin role
    let user = create_user_node("user3", "user3@test.com", vec!["super_admin"], vec![]);
    storage
        .nodes_impl()
        .add(TENANT, REPO, BRANCH, ACCESS_CONTROL_WS, user)
        .await
        .expect("Failed to create user");

    // Resolve permissions
    let service = PermissionService::new(storage.clone());
    let result = service
        .resolve_for_user_id(TENANT, REPO, BRANCH, "user3")
        .await
        .expect("Failed to resolve permissions");

    assert!(result.is_some(), "User should be found");
    let resolved = result.unwrap();

    // Should have all 4 roles through inheritance
    assert_eq!(resolved.effective_roles.len(), 4);
    assert!(resolved
        .effective_roles
        .contains(&"super_admin".to_string()));
    assert!(resolved.effective_roles.contains(&"admin".to_string()));
    assert!(resolved.effective_roles.contains(&"editor".to_string()));
    assert!(resolved.effective_roles.contains(&"reader".to_string()));

    // Should have permissions from all roles (4 permissions)
    assert_eq!(resolved.permissions.len(), 4);
}

// ============================================================================
// Test 4: Role inheritance cycle detection
// ============================================================================

#[tokio::test]
async fn test_role_inheritance_cycle_detection() {
    let (storage, _temp_dir) = setup_storage().await.expect("Failed to setup storage");

    // Create folder structure
    setup_folder_structure(&storage)
        .await
        .expect("Failed to setup folders");

    // Create a cycle: role_a inherits role_b, role_b inherits role_c, role_c inherits role_a
    let role_a = create_role_node(
        "role_a",
        vec!["role_b"],
        vec![permission_json("path_a.**", vec!["read"])],
    );
    let role_b = create_role_node(
        "role_b",
        vec!["role_c"],
        vec![permission_json("path_b.**", vec!["read"])],
    );
    let role_c = create_role_node(
        "role_c",
        vec!["role_a"], // Creates cycle back to role_a
        vec![permission_json("path_c.**", vec!["read"])],
    );

    for role in [role_a, role_b, role_c] {
        storage
            .nodes_impl()
            .add(TENANT, REPO, BRANCH, ACCESS_CONTROL_WS, role)
            .await
            .expect("Failed to create role");
    }

    // Create user with role_a
    let user = create_user_node("user4", "user4@test.com", vec!["role_a"], vec![]);
    storage
        .nodes_impl()
        .add(TENANT, REPO, BRANCH, ACCESS_CONTROL_WS, user)
        .await
        .expect("Failed to create user");

    // This should NOT infinite loop - cycle detection should prevent it
    let service = PermissionService::new(storage.clone());
    let result = service
        .resolve_for_user_id(TENANT, REPO, BRANCH, "user4")
        .await;

    // Should succeed without hanging
    assert!(result.is_ok(), "Should not infinite loop on cycle");
    let resolved = result.unwrap().unwrap();

    // Should have exactly 3 roles (each visited once)
    assert_eq!(resolved.effective_roles.len(), 3);
    assert!(resolved.effective_roles.contains(&"role_a".to_string()));
    assert!(resolved.effective_roles.contains(&"role_b".to_string()));
    assert!(resolved.effective_roles.contains(&"role_c".to_string()));
}

// ============================================================================
// Test 5: Permission deduplication (role deduplication)
// ============================================================================

#[tokio::test]
async fn test_role_deduplication() {
    let (storage, _temp_dir) = setup_storage().await.expect("Failed to setup storage");

    // Create folder structure
    setup_folder_structure(&storage)
        .await
        .expect("Failed to setup folders");

    // Create two roles that both have the same permission
    let role1 = create_role_node(
        "role1",
        vec![],
        vec![permission_json("content.**", vec!["read"])],
    );
    let role2 = create_role_node(
        "role2",
        vec![],
        vec![permission_json("content.**", vec!["read"])], // Same permission
    );

    for role in [role1, role2] {
        storage
            .nodes_impl()
            .add(TENANT, REPO, BRANCH, ACCESS_CONTROL_WS, role)
            .await
            .expect("Failed to create role");
    }

    // Create two groups, each with different roles
    let group1 = create_group_node("group1-id", "group1", vec!["role1"]);
    let group2 = create_group_node("group2-id", "group2", vec!["role2"]);

    for group in [group1, group2] {
        storage
            .nodes_impl()
            .add(TENANT, REPO, BRANCH, ACCESS_CONTROL_WS, group)
            .await
            .expect("Failed to create group");
    }

    // Create user with role1 directly AND in both groups
    let user = create_user_node(
        "user5",
        "user5@test.com",
        vec!["role1"],
        vec!["group1", "group2"],
    );
    storage
        .nodes_impl()
        .add(TENANT, REPO, BRANCH, ACCESS_CONTROL_WS, user)
        .await
        .expect("Failed to create user");

    // Resolve permissions
    let service = PermissionService::new(storage.clone());
    let result = service
        .resolve_for_user_id(TENANT, REPO, BRANCH, "user5")
        .await
        .expect("Failed to resolve permissions");

    assert!(result.is_some(), "User should be found");
    let resolved = result.unwrap();

    // Roles should be deduplicated
    assert_eq!(resolved.effective_roles.len(), 2); // role1 and role2, not 3

    // Note: The current implementation doesn't deduplicate permissions themselves,
    // only roles. So we may have duplicate permissions from role1 and role2.
    // This is expected behavior - permissions are collected from all roles.
    // The RLS filter will handle overlapping permissions.
}
