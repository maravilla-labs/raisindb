use super::*;
use raisin_context::RepositoryConfig;
use raisin_hlc::HLC;
use raisin_storage::{
    BranchRepository, RepositoryManagementRepository, RevisionMeta, RevisionRepository,
};
use std::collections::HashMap;

#[tokio::test]
async fn test_repository_crud() {
    let repo_mgmt = InMemoryRepositoryManagement::default();

    // Create repository
    let config = RepositoryConfig {
        default_branch: "main".to_string(),
        description: Some("Test repo".to_string()),
        tags: HashMap::new(),
        default_language: "en".to_string(),
        supported_languages: vec!["en".to_string()],
        locale_fallback_chains: HashMap::new(),
    };

    let info = repo_mgmt
        .create_repository("tenant-1", "repo-1", config.clone())
        .await
        .unwrap();

    assert_eq!(info.tenant_id, "tenant-1");
    assert_eq!(info.repo_id, "repo-1");
    assert_eq!(info.branches, vec!["main"]);

    // Get repository
    let retrieved = repo_mgmt
        .get_repository("tenant-1", "repo-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retrieved.repo_id, "repo-1");

    // List repositories
    let all = repo_mgmt.list_repositories().await.unwrap();
    assert_eq!(all.len(), 1);

    // Delete repository
    let deleted = repo_mgmt
        .delete_repository("tenant-1", "repo-1")
        .await
        .unwrap();
    assert!(deleted);

    let not_found = repo_mgmt
        .get_repository("tenant-1", "repo-1")
        .await
        .unwrap();
    assert!(not_found.is_none());
}

#[tokio::test]
async fn test_branch_crud() {
    let branches = InMemoryBranchRepo::default();

    // Create branch
    let branch = branches
        .create_branch(
            "tenant-1",
            "repo-1",
            "main",
            "test-user",
            None,
            None,
            false,
            false,
        )
        .await
        .unwrap();

    assert_eq!(branch.name, "main");
    assert_eq!(branch.head, HLC::new(0, 0));

    // Get branch
    let retrieved = branches
        .get_branch("tenant-1", "repo-1", "main")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retrieved.name, "main");

    // Update HEAD
    branches
        .update_head("tenant-1", "repo-1", "main", HLC::new(42, 0))
        .await
        .unwrap();

    let head = branches
        .get_head("tenant-1", "repo-1", "main")
        .await
        .unwrap();
    assert_eq!(head, HLC::new(42, 0));

    // List branches
    branches
        .create_branch(
            "tenant-1",
            "repo-1",
            "develop",
            "test-user",
            Some(HLC::new(42, 0)),
            None,
            false,
            false,
        )
        .await
        .unwrap();

    let all = branches.list_branches("tenant-1", "repo-1").await.unwrap();
    assert_eq!(all.len(), 2);

    // Delete branch
    let deleted = branches
        .delete_branch("tenant-1", "repo-1", "develop")
        .await
        .unwrap();
    assert!(deleted);
}

#[tokio::test]
async fn test_revision_tracking() {
    let revisions = InMemoryRevisionRepo::default();

    // Allocate revisions
    let rev1 = revisions.allocate_revision();
    let rev2 = revisions.allocate_revision();

    // Revisions are HLC values with incrementing counter
    assert!(rev1 < rev2);

    // Store revision metadata
    let meta = RevisionMeta {
        revision: rev1,
        parent: None,
        merge_parent: None,
        branch: "main".to_string(),
        timestamp: chrono::Utc::now(),
        actor: "user-123".to_string(),
        message: "Initial commit".to_string(),
        is_system: false,
        changed_nodes: Vec::new(),
        changed_node_types: Vec::new(),
        changed_archetypes: Vec::new(),
        changed_element_types: Vec::new(),
        operation: None,
    };

    revisions
        .store_revision_meta("tenant-1", "repo-1", meta.clone())
        .await
        .unwrap();

    // Get revision metadata
    let retrieved = revisions
        .get_revision_meta("tenant-1", "repo-1", &rev1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retrieved.message, "Initial commit");

    // Index node changes
    revisions
        .index_node_change("tenant-1", "repo-1", &rev1, "node-1")
        .await
        .unwrap();

    let node_revs = revisions
        .get_node_revisions("tenant-1", "repo-1", "node-1", 10)
        .await
        .unwrap();
    assert_eq!(node_revs, vec![rev1]);
}
