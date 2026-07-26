//! RLS Integration Tests
//!
//! Tests for the full RLS (Row-Level Security) pipeline:
//! - User can only read own articles (ownership condition)
//! - System admin bypasses RLS
//! - Field-level filtering (except_fields)
//! - Create permission enforcement
//! - Delete permission enforcement

use raisin_context::RepositoryConfig;
use raisin_core::services::rls_filter::{
    can_create_at_path, can_perform, filter_node, filter_nodes,
};
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_error::Result;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::permissions::{Operation, Permission, PermissionScope, ResolvedPermissions};
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
const CONTENT_WS: &str = "content";

/// Setup storage with tenant, repo, branch, and workspaces
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
        description: Some("RLS integration test repository".to_string()),
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

    // Create workspaces
    let workspace_service = WorkspaceService::new(storage.clone());

    // Access control workspace
    let ac_workspace = Workspace::new(ACCESS_CONTROL_WS.to_string());
    workspace_service.put(TENANT, REPO, ac_workspace).await?;

    // Content workspace
    let content_workspace = Workspace::new(CONTENT_WS.to_string());
    workspace_service
        .put(TENANT, REPO, content_workspace)
        .await?;

    Ok((storage, temp_dir))
}

/// Create an article node with author property
fn create_article_node(
    id: &str,
    title: &str,
    author: &str,
    content: &str,
    secret_notes: &str,
) -> Node {
    let mut props = HashMap::new();
    props.insert(
        "title".to_string(),
        PropertyValue::String(title.to_string()),
    );
    props.insert(
        "author".to_string(),
        PropertyValue::String(author.to_string()),
    );
    props.insert(
        "content".to_string(),
        PropertyValue::String(content.to_string()),
    );
    props.insert(
        "secret_notes".to_string(),
        PropertyValue::String(secret_notes.to_string()),
    );

    Node {
        id: id.to_string(),
        name: id.to_string(),
        path: format!("/articles/{}", id),
        node_type: "Article".to_string(),
        archetype: None,
        properties: props,
        children: vec![],
        order_key: String::new(),
        has_children: None,
        parent: Some("articles".to_string()),
        version: 1,
        created_at: Some(chrono::Utc::now()),
        updated_at: Some(chrono::Utc::now()),
        published_at: None,
        published_by: None,
        updated_by: Some("test-user".to_string()),
        created_by: Some("test-user".to_string()),
        translations: None,
        tenant_id: Some(TENANT.to_string()),
        workspace: Some(CONTENT_WS.to_string()),
        owner_id: None,
        relations: Vec::new(),
    }
}

/// Create a permission with ownership condition using REL expression
fn permission_with_ownership(
    path: &str,
    operations: Vec<Operation>,
    property_key: &str,
) -> Permission {
    // REL expression: node.<property> == auth.user_id
    let condition = format!("node.{} == auth.user_id", property_key);
    Permission::new(path, operations).with_condition(condition)
}

/// Create a permission with except_fields
fn permission_with_field_filter(
    path: &str,
    operations: Vec<Operation>,
    except_fields: Vec<&str>,
) -> Permission {
    Permission::new(path, operations)
        .with_except_fields(except_fields.iter().map(|s| s.to_string()).collect())
}

/// Create a basic permission without conditions or field filters
fn basic_permission(path: &str, operations: Vec<Operation>) -> Permission {
    Permission::new(path, operations)
}

/// Create an auth context for a user with given permissions
fn auth_for_user(
    user_id: &str,
    permissions: Vec<Permission>,
    is_system_admin: bool,
) -> AuthContext {
    AuthContext::for_user(user_id).with_permissions(ResolvedPermissions {
        user_id: user_id.to_string(),
        email: Some(format!("{}@test.com", user_id)),
        direct_roles: vec![],
        group_roles: vec![],
        effective_roles: if is_system_admin {
            vec!["system_admin".to_string()]
        } else {
            vec![]
        },
        groups: vec![],
        permissions,
        is_system_admin,
        resolved_at: Some(std::time::Instant::now()),
    })
}

// ============================================================================
// Test 1: User can only read own articles (ownership condition)
// ============================================================================

#[tokio::test]
async fn test_user_can_only_read_own_articles() {
    let (_storage, _temp_dir) = setup_storage().await.expect("Failed to setup storage");

    // Create articles by different authors
    let alice_article = create_article_node(
        "article1",
        "Alice's Article",
        "alice",
        "Alice's content",
        "Secret notes",
    );
    let bob_article = create_article_node(
        "article2",
        "Bob's Article",
        "bob",
        "Bob's content",
        "Bob's secret",
    );

    // Alice has permission to read articles WHERE author = $auth.user_id
    let alice_permissions = vec![permission_with_ownership(
        "/articles/**",
        vec![Operation::Read],
        "author",
    )];
    let alice_auth = auth_for_user("alice", alice_permissions, false);

    let scope = PermissionScope::default();

    // Test: Alice can read her own article
    let result = filter_node(alice_article.clone(), &alice_auth, &scope);
    assert!(
        result.is_some(),
        "Alice should be able to read her own article"
    );
    assert_eq!(result.unwrap().id, "article1");

    // Test: Alice cannot read Bob's article
    let result = filter_node(bob_article.clone(), &alice_auth, &scope);
    assert!(
        result.is_none(),
        "Alice should NOT be able to read Bob's article"
    );

    // Test with filter_nodes
    let all_articles = vec![alice_article.clone(), bob_article.clone()];
    let alice_visible = filter_nodes(all_articles, &alice_auth, &scope);
    assert_eq!(alice_visible.len(), 1, "Alice should only see 1 article");
    assert_eq!(
        alice_visible[0].id, "article1",
        "Alice should only see her own article"
    );
}

// ============================================================================
// Test 2: System admin bypasses RLS
// ============================================================================

#[tokio::test]
async fn test_admin_bypasses_rls() {
    let (_storage, _temp_dir) = setup_storage().await.expect("Failed to setup storage");

    // Create a secret article
    let secret_article = create_article_node(
        "secret1",
        "Secret Article",
        "classified",
        "Top secret content",
        "Eyes only",
    );

    // Regular user has no permissions
    let regular_auth = auth_for_user("user1", vec![], false);

    // System admin has is_system_admin flag
    let admin_auth = auth_for_user("admin", vec![], true);

    let scope = PermissionScope::default();

    // Test: Regular user cannot read the article
    let result = filter_node(secret_article.clone(), &regular_auth, &scope);
    assert!(
        result.is_none(),
        "Regular user should NOT be able to read secret article"
    );

    // Test: Admin can read the article (bypasses RLS)
    let result = filter_node(secret_article.clone(), &admin_auth, &scope);
    assert!(result.is_some(), "Admin should be able to read any article");
    assert_eq!(result.unwrap().id, "secret1");

    // Test: System context also bypasses RLS
    let system_auth = AuthContext::system();
    let result = filter_node(secret_article.clone(), &system_auth, &scope);
    assert!(result.is_some(), "System context should bypass all RLS");
}

// ============================================================================
// Test 3: Field-level filtering (except_fields)
// ============================================================================

#[tokio::test]
async fn test_field_filtering_applied() {
    let (_storage, _temp_dir) = setup_storage().await.expect("Failed to setup storage");

    // Create an article with sensitive field
    let article = create_article_node(
        "article1",
        "Public Article",
        "author1",
        "Public content",
        "This is a secret note that should be filtered",
    );

    // User has permission to read articles but secret_notes field is excluded
    let user_permissions = vec![permission_with_field_filter(
        "/articles/**",
        vec![Operation::Read],
        vec!["secret_notes"],
    )];
    let user_auth = auth_for_user("user1", user_permissions, false);

    let scope = PermissionScope::default();

    // Test: User can read the article but secret_notes is filtered out
    let result = filter_node(article.clone(), &user_auth, &scope);
    assert!(result.is_some(), "User should be able to read the article");

    let filtered_article = result.unwrap();

    // Public fields should be present
    assert!(
        filtered_article.properties.contains_key("title"),
        "title should be present"
    );
    assert!(
        filtered_article.properties.contains_key("content"),
        "content should be present"
    );
    assert!(
        filtered_article.properties.contains_key("author"),
        "author should be present"
    );

    // Sensitive field should be filtered out
    assert!(
        !filtered_article.properties.contains_key("secret_notes"),
        "secret_notes should be filtered out"
    );
}

// ============================================================================
// Test 4: Create permission enforced
// ============================================================================

#[tokio::test]
async fn test_create_permission_enforced() {
    let (_storage, _temp_dir) = setup_storage().await.expect("Failed to setup storage");

    // User with only READ permission
    let reader_auth = auth_for_user(
        "reader",
        vec![basic_permission("/articles/**", vec![Operation::Read])],
        false,
    );

    // User with CREATE permission
    let creator_auth = auth_for_user(
        "creator",
        vec![basic_permission(
            "/articles/**",
            vec![Operation::Create, Operation::Read],
        )],
        false,
    );

    // User with CREATE permission only in a specific path
    let limited_creator_auth = auth_for_user(
        "limited",
        vec![basic_permission(
            "/articles/drafts/**",
            vec![Operation::Create],
        )],
        false,
    );

    let scope = PermissionScope::default();

    // Test: Reader cannot create articles
    let can_create = can_create_at_path("/articles/new-article", "Article", &reader_auth, &scope);
    assert!(!can_create, "Reader should NOT be able to create articles");

    // Test: Creator can create articles
    let can_create = can_create_at_path("/articles/new-article", "Article", &creator_auth, &scope);
    assert!(can_create, "Creator should be able to create articles");

    // Test: Limited creator can create in drafts
    let can_create = can_create_at_path(
        "/articles/drafts/new-draft",
        "Article",
        &limited_creator_auth,
        &scope,
    );
    assert!(
        can_create,
        "Limited creator should be able to create in drafts"
    );

    // Test: Limited creator cannot create in main articles
    let can_create = can_create_at_path(
        "/articles/new-article",
        "Article",
        &limited_creator_auth,
        &scope,
    );
    assert!(
        !can_create,
        "Limited creator should NOT be able to create in main articles"
    );

    // Test: System admin can create anywhere
    let admin_auth = auth_for_user("admin", vec![], true);
    let can_create = can_create_at_path("/articles/new-article", "Article", &admin_auth, &scope);
    assert!(can_create, "Admin should be able to create anywhere");
}

// ============================================================================
// Test 5: Delete permission enforced
// ============================================================================

#[tokio::test]
async fn test_delete_permission_enforced() {
    let (_storage, _temp_dir) = setup_storage().await.expect("Failed to setup storage");

    // Create an article
    let article = create_article_node("article1", "To Be Deleted", "author1", "Content", "Secret");

    // User with only READ permission
    let reader_auth = auth_for_user(
        "reader",
        vec![basic_permission("/articles/**", vec![Operation::Read])],
        false,
    );

    // User with DELETE permission
    let deleter_auth = auth_for_user(
        "deleter",
        vec![basic_permission(
            "/articles/**",
            vec![Operation::Read, Operation::Delete],
        )],
        false,
    );

    // User with ownership-based DELETE (can only delete own articles)
    let owner_permissions = vec![permission_with_ownership(
        "/articles/**",
        vec![Operation::Read, Operation::Delete],
        "author",
    )];
    let author1_auth = auth_for_user("author1", owner_permissions.clone(), false);
    let author2_auth = auth_for_user("author2", owner_permissions, false);

    let scope = PermissionScope::default();

    // Test: Reader cannot delete
    let can_delete = can_perform(&article, Operation::Delete, &reader_auth, &scope);
    assert!(!can_delete, "Reader should NOT be able to delete");

    // Test: Deleter can delete
    let can_delete = can_perform(&article, Operation::Delete, &deleter_auth, &scope);
    assert!(can_delete, "Deleter should be able to delete");

    // Test: Author1 can delete their own article
    let can_delete = can_perform(&article, Operation::Delete, &author1_auth, &scope);
    assert!(
        can_delete,
        "Author1 should be able to delete their own article"
    );

    // Test: Author2 cannot delete Author1's article
    let can_delete = can_perform(&article, Operation::Delete, &author2_auth, &scope);
    assert!(
        !can_delete,
        "Author2 should NOT be able to delete Author1's article"
    );

    // Test: System admin can delete anything
    let admin_auth = auth_for_user("admin", vec![], true);
    let can_delete = can_perform(&article, Operation::Delete, &admin_auth, &scope);
    assert!(can_delete, "Admin should be able to delete anything");
}

// ============================================================================
// Test 6: Cross-user data isolation (User A cannot see User B's data)
// ============================================================================

#[tokio::test]
async fn test_cross_user_data_isolation() {
    let (_storage, _temp_dir) = setup_storage().await.expect("Failed to setup storage");

    // Create articles owned by different users
    let alice_article1 = create_article_node(
        "alice-1",
        "Alice Article 1",
        "alice",
        "Alice content 1",
        "Alice secret 1",
    );
    let alice_article2 = create_article_node(
        "alice-2",
        "Alice Article 2",
        "alice",
        "Alice content 2",
        "Alice secret 2",
    );
    let bob_article1 = create_article_node(
        "bob-1",
        "Bob Article 1",
        "bob",
        "Bob content 1",
        "Bob secret 1",
    );
    let bob_article2 = create_article_node(
        "bob-2",
        "Bob Article 2",
        "bob",
        "Bob content 2",
        "Bob secret 2",
    );
    let charlie_article = create_article_node(
        "charlie-1",
        "Charlie Article",
        "charlie",
        "Charlie content",
        "Charlie secret",
    );

    let all_articles = vec![
        alice_article1.clone(),
        alice_article2.clone(),
        bob_article1.clone(),
        bob_article2.clone(),
        charlie_article.clone(),
    ];

    // Each user has ownership-based permissions
    let ownership_permission = permission_with_ownership(
        "/articles/**",
        vec![Operation::Read, Operation::Update, Operation::Delete],
        "author",
    );

    let alice_auth = auth_for_user("alice", vec![ownership_permission.clone()], false);
    let bob_auth = auth_for_user("bob", vec![ownership_permission.clone()], false);
    let charlie_auth = auth_for_user("charlie", vec![ownership_permission.clone()], false);

    let scope = PermissionScope::default();

    // === Test Alice's view ===
    let alice_visible = filter_nodes(all_articles.clone(), &alice_auth, &scope);
    assert_eq!(
        alice_visible.len(),
        2,
        "Alice should see exactly 2 articles"
    );
    assert!(
        alice_visible
            .iter()
            .all(|a| a.properties.get("author").and_then(|v| match v {
                PropertyValue::String(s) => Some(s.as_str()),
                _ => None,
            }) == Some("alice")),
        "Alice should only see her own articles"
    );

    // Verify Alice cannot see Bob's or Charlie's articles
    assert!(
        !alice_visible
            .iter()
            .any(|a| a.id == "bob-1" || a.id == "bob-2"),
        "Alice should NOT see Bob's articles"
    );
    assert!(
        !alice_visible.iter().any(|a| a.id == "charlie-1"),
        "Alice should NOT see Charlie's articles"
    );

    // === Test Bob's view ===
    let bob_visible = filter_nodes(all_articles.clone(), &bob_auth, &scope);
    assert_eq!(bob_visible.len(), 2, "Bob should see exactly 2 articles");
    assert!(
        bob_visible
            .iter()
            .all(|a| a.properties.get("author").and_then(|v| match v {
                PropertyValue::String(s) => Some(s.as_str()),
                _ => None,
            }) == Some("bob")),
        "Bob should only see his own articles"
    );

    // Verify Bob cannot see Alice's or Charlie's articles
    assert!(
        !bob_visible
            .iter()
            .any(|a| a.id == "alice-1" || a.id == "alice-2"),
        "Bob should NOT see Alice's articles"
    );

    // === Test Charlie's view ===
    let charlie_visible = filter_nodes(all_articles.clone(), &charlie_auth, &scope);
    assert_eq!(
        charlie_visible.len(),
        1,
        "Charlie should see exactly 1 article"
    );
    assert_eq!(
        charlie_visible[0].id, "charlie-1",
        "Charlie should only see his own article"
    );

    // === Test cross-user modification attempts ===
    // Alice tries to update Bob's article
    let can_update = can_perform(&bob_article1, Operation::Update, &alice_auth, &scope);
    assert!(
        !can_update,
        "Alice should NOT be able to update Bob's article"
    );

    // Bob tries to delete Alice's article
    let can_delete = can_perform(&alice_article1, Operation::Delete, &bob_auth, &scope);
    assert!(
        !can_delete,
        "Bob should NOT be able to delete Alice's article"
    );

    // Charlie tries to update Alice's article
    let can_update = can_perform(&alice_article2, Operation::Update, &charlie_auth, &scope);
    assert!(
        !can_update,
        "Charlie should NOT be able to update Alice's article"
    );
}

// ============================================================================
// Test 7: Role-based visibility (different roles see different content)
// ============================================================================

#[tokio::test]
async fn test_role_based_visibility() {
    let (_storage, _temp_dir) = setup_storage().await.expect("Failed to setup storage");

    // Create different types of content
    let public_article = create_content_node(
        "public-1",
        "/articles/public/news",
        "PublicArticle",
        "public",
    );
    let internal_doc = create_content_node(
        "internal-1",
        "/documents/internal/report",
        "InternalDocument",
        "internal",
    );
    let confidential_data = create_content_node(
        "confidential-1",
        "/data/confidential/financials",
        "ConfidentialData",
        "confidential",
    );
    let top_secret = create_content_node(
        "topsecret-1",
        "/classified/topsecret/plans",
        "TopSecret",
        "topsecret",
    );

    let all_content = vec![
        public_article.clone(),
        internal_doc.clone(),
        confidential_data.clone(),
        top_secret.clone(),
    ];

    // Public user - can only see public articles
    let public_auth = auth_for_user(
        "public_user",
        vec![basic_permission(
            "/articles/public/**",
            vec![Operation::Read],
        )],
        false,
    );

    // Internal employee - can see public + internal
    let employee_auth = auth_for_user(
        "employee",
        vec![
            basic_permission("/articles/public/**", vec![Operation::Read]),
            basic_permission("/documents/internal/**", vec![Operation::Read]),
        ],
        false,
    );

    // Manager - can see public + internal + confidential
    let manager_auth = auth_for_user(
        "manager",
        vec![
            basic_permission("/articles/public/**", vec![Operation::Read]),
            basic_permission("/documents/internal/**", vec![Operation::Read]),
            basic_permission("/data/confidential/**", vec![Operation::Read]),
        ],
        false,
    );

    // Executive - can see everything except top secret
    let exec_auth = auth_for_user(
        "executive",
        vec![
            basic_permission("/articles/**", vec![Operation::Read]),
            basic_permission("/documents/**", vec![Operation::Read]),
            basic_permission("/data/**", vec![Operation::Read]),
        ],
        false,
    );

    let scope = PermissionScope::default();

    // === Test Public User ===
    let public_visible = filter_nodes(all_content.clone(), &public_auth, &scope);
    assert_eq!(public_visible.len(), 1, "Public user should see 1 item");
    assert_eq!(
        public_visible[0].id, "public-1",
        "Public user should only see public article"
    );

    // Verify public user cannot see internal, confidential, or top secret
    assert!(
        filter_node(internal_doc.clone(), &public_auth, &scope).is_none(),
        "Public user should NOT see internal document"
    );
    assert!(
        filter_node(confidential_data.clone(), &public_auth, &scope).is_none(),
        "Public user should NOT see confidential data"
    );
    assert!(
        filter_node(top_secret.clone(), &public_auth, &scope).is_none(),
        "Public user should NOT see top secret"
    );

    // === Test Employee ===
    let employee_visible = filter_nodes(all_content.clone(), &employee_auth, &scope);
    assert_eq!(employee_visible.len(), 2, "Employee should see 2 items");
    let employee_ids: Vec<&str> = employee_visible.iter().map(|n| n.id.as_str()).collect();
    assert!(
        employee_ids.contains(&"public-1"),
        "Employee should see public article"
    );
    assert!(
        employee_ids.contains(&"internal-1"),
        "Employee should see internal document"
    );

    // Verify employee cannot see confidential or top secret
    assert!(
        filter_node(confidential_data.clone(), &employee_auth, &scope).is_none(),
        "Employee should NOT see confidential data"
    );
    assert!(
        filter_node(top_secret.clone(), &employee_auth, &scope).is_none(),
        "Employee should NOT see top secret"
    );

    // === Test Manager ===
    let manager_visible = filter_nodes(all_content.clone(), &manager_auth, &scope);
    assert_eq!(manager_visible.len(), 3, "Manager should see 3 items");

    // Verify manager cannot see top secret
    assert!(
        filter_node(top_secret.clone(), &manager_auth, &scope).is_none(),
        "Manager should NOT see top secret"
    );

    // === Test Executive ===
    let exec_visible = filter_nodes(all_content.clone(), &exec_auth, &scope);
    assert_eq!(
        exec_visible.len(),
        3,
        "Executive should see 3 items (not top secret)"
    );

    // Executive still cannot see top secret (not in their permission paths)
    assert!(
        filter_node(top_secret.clone(), &exec_auth, &scope).is_none(),
        "Executive should NOT see top secret (not in permitted paths)"
    );
}

// ============================================================================
// Test 8: Path-based workspace isolation
// ============================================================================

#[tokio::test]
async fn test_path_based_workspace_isolation() {
    let (_storage, _temp_dir) = setup_storage().await.expect("Failed to setup storage");

    // Create content in different workspaces/paths
    let marketing_doc = create_content_node(
        "mkt-1",
        "/departments/marketing/campaigns",
        "Document",
        "marketing",
    );
    let engineering_doc = create_content_node(
        "eng-1",
        "/departments/engineering/specs",
        "Document",
        "engineering",
    );
    let hr_doc = create_content_node("hr-1", "/departments/hr/policies", "Document", "hr");
    let finance_doc = create_content_node(
        "fin-1",
        "/departments/finance/budgets",
        "Document",
        "finance",
    );
    let shared_doc = create_content_node("shared-1", "/shared/announcements", "Document", "shared");

    let all_docs = vec![
        marketing_doc.clone(),
        engineering_doc.clone(),
        hr_doc.clone(),
        finance_doc.clone(),
        shared_doc.clone(),
    ];

    // Marketing team member - can see marketing + shared
    let marketing_auth = auth_for_user(
        "marketing_user",
        vec![
            basic_permission(
                "/departments/marketing/**",
                vec![Operation::Read, Operation::Update],
            ),
            basic_permission("/shared/**", vec![Operation::Read]),
        ],
        false,
    );

    // Engineering team member - can see engineering + shared
    let engineering_auth = auth_for_user(
        "eng_user",
        vec![
            basic_permission(
                "/departments/engineering/**",
                vec![Operation::Read, Operation::Update],
            ),
            basic_permission("/shared/**", vec![Operation::Read]),
        ],
        false,
    );

    // HR has access to all departments (for compliance)
    let hr_auth = auth_for_user(
        "hr_user",
        vec![
            basic_permission("/departments/**", vec![Operation::Read]),
            basic_permission("/shared/**", vec![Operation::Read]),
        ],
        false,
    );

    let scope = PermissionScope::default();

    // === Test Marketing User ===
    let marketing_visible = filter_nodes(all_docs.clone(), &marketing_auth, &scope);
    assert_eq!(
        marketing_visible.len(),
        2,
        "Marketing user should see 2 docs"
    );
    let mkt_ids: Vec<&str> = marketing_visible.iter().map(|n| n.id.as_str()).collect();
    assert!(
        mkt_ids.contains(&"mkt-1"),
        "Marketing should see marketing doc"
    );
    assert!(
        mkt_ids.contains(&"shared-1"),
        "Marketing should see shared doc"
    );

    // Marketing cannot see engineering, HR, or finance docs
    assert!(
        filter_node(engineering_doc.clone(), &marketing_auth, &scope).is_none(),
        "Marketing should NOT see engineering docs"
    );
    assert!(
        filter_node(hr_doc.clone(), &marketing_auth, &scope).is_none(),
        "Marketing should NOT see HR docs"
    );
    assert!(
        filter_node(finance_doc.clone(), &marketing_auth, &scope).is_none(),
        "Marketing should NOT see finance docs"
    );

    // Marketing cannot update engineering docs
    let can_update_eng = can_perform(&engineering_doc, Operation::Update, &marketing_auth, &scope);
    assert!(
        !can_update_eng,
        "Marketing should NOT be able to update engineering docs"
    );

    // === Test Engineering User ===
    let eng_visible = filter_nodes(all_docs.clone(), &engineering_auth, &scope);
    assert_eq!(eng_visible.len(), 2, "Engineering user should see 2 docs");

    // Engineering cannot see marketing, HR, or finance docs
    assert!(
        filter_node(marketing_doc.clone(), &engineering_auth, &scope).is_none(),
        "Engineering should NOT see marketing docs"
    );

    // === Test HR User (cross-department access) ===
    let hr_visible = filter_nodes(all_docs.clone(), &hr_auth, &scope);
    assert_eq!(hr_visible.len(), 5, "HR should see all 5 docs");

    // HR can read but not update other departments (only Read permission)
    let can_update_mkt = can_perform(&marketing_doc, Operation::Update, &hr_auth, &scope);
    assert!(
        !can_update_mkt,
        "HR should NOT be able to update marketing docs (read-only access)"
    );
}

// ============================================================================
// Test 9: Anonymous vs authenticated user isolation
// ============================================================================

#[tokio::test]
async fn test_anonymous_vs_authenticated_isolation() {
    let (_storage, _temp_dir) = setup_storage().await.expect("Failed to setup storage");

    // Create public and private content
    let public_page = create_content_node("pub-1", "/public/welcome", "Page", "public");
    let member_content = create_content_node("member-1", "/members/dashboard", "Page", "members");
    let premium_content = create_content_node("premium-1", "/premium/exclusive", "Page", "premium");

    let all_content = vec![
        public_page.clone(),
        member_content.clone(),
        premium_content.clone(),
    ];

    // Anonymous user - only public content
    let anon_auth = auth_for_user(
        "anonymous",
        vec![basic_permission("/public/**", vec![Operation::Read])],
        false,
    );

    // Free member - public + member content
    let member_auth = auth_for_user(
        "free_member",
        vec![
            basic_permission("/public/**", vec![Operation::Read]),
            basic_permission("/members/**", vec![Operation::Read]),
        ],
        false,
    );

    // Premium member - all content
    let premium_auth = auth_for_user(
        "premium_member",
        vec![
            basic_permission("/public/**", vec![Operation::Read]),
            basic_permission("/members/**", vec![Operation::Read]),
            basic_permission("/premium/**", vec![Operation::Read]),
        ],
        false,
    );

    let scope = PermissionScope::default();

    // === Test Anonymous User ===
    let anon_visible = filter_nodes(all_content.clone(), &anon_auth, &scope);
    assert_eq!(anon_visible.len(), 1, "Anonymous should see 1 page");
    assert_eq!(
        anon_visible[0].id, "pub-1",
        "Anonymous should only see public page"
    );

    // Anonymous cannot see member or premium content
    assert!(
        filter_node(member_content.clone(), &anon_auth, &scope).is_none(),
        "Anonymous should NOT see member content"
    );
    assert!(
        filter_node(premium_content.clone(), &anon_auth, &scope).is_none(),
        "Anonymous should NOT see premium content"
    );

    // === Test Free Member ===
    let member_visible = filter_nodes(all_content.clone(), &member_auth, &scope);
    assert_eq!(member_visible.len(), 2, "Free member should see 2 pages");

    // Free member cannot see premium content
    assert!(
        filter_node(premium_content.clone(), &member_auth, &scope).is_none(),
        "Free member should NOT see premium content"
    );

    // === Test Premium Member ===
    let premium_visible = filter_nodes(all_content.clone(), &premium_auth, &scope);
    assert_eq!(
        premium_visible.len(),
        3,
        "Premium member should see all 3 pages"
    );
}

// ============================================================================
// Test 10: Combined conditions isolation (multiple conditions must ALL pass)
// ============================================================================

#[tokio::test]
async fn test_combined_conditions_isolation() {
    let (_storage, _temp_dir) = setup_storage().await.expect("Failed to setup storage");

    // Create articles with multiple properties
    let mut props = HashMap::new();
    props.insert(
        "author".to_string(),
        PropertyValue::String("alice".to_string()),
    );
    props.insert(
        "status".to_string(),
        PropertyValue::String("published".to_string()),
    );
    props.insert(
        "department".to_string(),
        PropertyValue::String("engineering".to_string()),
    );
    let alice_published_eng = Node {
        id: "article-1".to_string(),
        name: "article-1".to_string(),
        path: "/articles/article-1".to_string(),
        node_type: "Article".to_string(),
        properties: props,
        ..Default::default()
    };

    let mut props2 = HashMap::new();
    props2.insert(
        "author".to_string(),
        PropertyValue::String("alice".to_string()),
    );
    props2.insert(
        "status".to_string(),
        PropertyValue::String("draft".to_string()),
    );
    props2.insert(
        "department".to_string(),
        PropertyValue::String("engineering".to_string()),
    );
    let alice_draft_eng = Node {
        id: "article-2".to_string(),
        name: "article-2".to_string(),
        path: "/articles/article-2".to_string(),
        node_type: "Article".to_string(),
        properties: props2,
        ..Default::default()
    };

    let mut props3 = HashMap::new();
    props3.insert(
        "author".to_string(),
        PropertyValue::String("bob".to_string()),
    );
    props3.insert(
        "status".to_string(),
        PropertyValue::String("published".to_string()),
    );
    props3.insert(
        "department".to_string(),
        PropertyValue::String("engineering".to_string()),
    );
    let bob_published_eng = Node {
        id: "article-3".to_string(),
        name: "article-3".to_string(),
        path: "/articles/article-3".to_string(),
        node_type: "Article".to_string(),
        properties: props3,
        ..Default::default()
    };

    let all_articles = vec![
        alice_published_eng.clone(),
        alice_draft_eng.clone(),
        bob_published_eng.clone(),
    ];

    // Permission: Can read articles WHERE author=$auth.user_id AND status='published'
    // Using REL expression instead of typed conditions
    let author_and_published = Permission::read_only("/articles/**")
        .with_condition("node.author == auth.user_id && node.status == 'published'");

    let alice_auth = auth_for_user("alice", vec![author_and_published.clone()], false);
    let bob_auth = auth_for_user("bob", vec![author_and_published], false);

    let scope = PermissionScope::default();

    // === Test Alice ===
    let alice_visible = filter_nodes(all_articles.clone(), &alice_auth, &scope);
    // Alice should only see her PUBLISHED article, not her draft
    assert_eq!(
        alice_visible.len(),
        1,
        "Alice should see 1 article (her published one)"
    );
    assert_eq!(
        alice_visible[0].id, "article-1",
        "Alice should see only her published article"
    );

    // Alice cannot see her draft (status condition fails)
    assert!(
        filter_node(alice_draft_eng.clone(), &alice_auth, &scope).is_none(),
        "Alice should NOT see her draft article"
    );

    // Alice cannot see Bob's published article (author condition fails)
    assert!(
        filter_node(bob_published_eng.clone(), &alice_auth, &scope).is_none(),
        "Alice should NOT see Bob's article"
    );

    // === Test Bob ===
    let bob_visible = filter_nodes(all_articles.clone(), &bob_auth, &scope);
    assert_eq!(bob_visible.len(), 1, "Bob should see 1 article");
    assert_eq!(
        bob_visible[0].id, "article-3",
        "Bob should see only his published article"
    );

    // Bob cannot see Alice's articles
    assert!(
        filter_node(alice_published_eng.clone(), &bob_auth, &scope).is_none(),
        "Bob should NOT see Alice's published article"
    );
    assert!(
        filter_node(alice_draft_eng.clone(), &bob_auth, &scope).is_none(),
        "Bob should NOT see Alice's draft article"
    );
}

// ============================================================================
// Helper: Create a generic content node for testing
// ============================================================================

fn create_content_node(id: &str, path: &str, node_type: &str, category: &str) -> Node {
    let mut props = HashMap::new();
    props.insert(
        "category".to_string(),
        PropertyValue::String(category.to_string()),
    );

    Node {
        id: id.to_string(),
        name: id.to_string(),
        path: path.to_string(),
        node_type: node_type.to_string(),
        archetype: None,
        properties: props,
        children: vec![],
        order_key: String::new(),
        has_children: None,
        parent: None,
        version: 1,
        created_at: Some(chrono::Utc::now()),
        updated_at: Some(chrono::Utc::now()),
        published_at: None,
        published_by: None,
        updated_by: Some("test-user".to_string()),
        created_by: Some("test-user".to_string()),
        translations: None,
        tenant_id: Some(TENANT.to_string()),
        workspace: Some(CONTENT_WS.to_string()),
        owner_id: None,
        relations: Vec::new(),
    }
}
