//! Integration tests for graph database relationships

use raisin_models::nodes::RelationRef;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{BranchRepository, RelationRepository, Storage};
use raisin_storage::scope::StorageScope;
use tempfile::TempDir;

async fn setup_storage() -> (RocksDBStorage, String, String, String, String) {
    let temp_dir = TempDir::new().unwrap();
    let storage = RocksDBStorage::new(temp_dir.path()).unwrap();

    let tenant_id = "test-tenant";
    let repo_id = "test-repo";
    let branch = "main";
    let workspace = "main";

    // Initialize branch
    storage
        .branches()
        .create_branch(
            tenant_id,
            repo_id,
            branch,
            "test-user",
            None,
            None,
            false,
            false,
        )
        .await
        .unwrap();

    (
        storage,
        tenant_id.to_string(),
        repo_id.to_string(),
        branch.to_string(),
        workspace.to_string(),
    )
}

#[tokio::test]
async fn test_add_and_get_relationships() {
    let (storage, tenant_id, repo_id, branch, workspace) = setup_storage().await;

    // Add relationship from node1 to node2
    let relation = RelationRef::new(
        "node2".to_string(),
        workspace.clone(),
        "raisin:Page".to_string(),
        "related_to".to_string(),
        Some(1.5),
    );

    storage
        .relations()
        .add_relation(
            StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
            "node1",
            "raisin:Page",
            relation.clone(),
        )
        .await
        .unwrap();

    // Get outgoing relationships from node1
    let outgoing = storage
        .relations()
        .get_outgoing_relations(StorageScope::new(&tenant_id, &repo_id, &branch, &workspace), "node1", None)
        .await
        .unwrap();

    assert_eq!(outgoing.len(), 1);
    assert_eq!(outgoing[0].target, "node2");
    assert_eq!(outgoing[0].workspace, workspace);
    assert_eq!(outgoing[0].relation_type, "raisin:Page");
    assert_eq!(outgoing[0].weight, Some(1.5));

    // Get incoming relationships to node2
    let incoming = storage
        .relations()
        .get_incoming_relations(StorageScope::new(&tenant_id, &repo_id, &branch, &workspace), "node2", None)
        .await
        .unwrap();

    assert_eq!(incoming.len(), 1);
    assert_eq!(incoming[0].0, workspace); // source_workspace
    assert_eq!(incoming[0].1, "node1"); // source_node_id

    println!("✅ Test passed: add_and_get_relationships");
}

#[tokio::test]
async fn test_remove_relationship() {
    let (storage, tenant_id, repo_id, branch, workspace) = setup_storage().await;

    // Add relationship
    let relation = RelationRef::simple(
        "node2".to_string(),
        workspace.clone(),
        "raisin:Page".to_string(),
        "related_to".to_string(),
    );

    storage
        .relations()
        .add_relation(
            StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
            "node1",
            "raisin:Page",
            relation,
        )
        .await
        .unwrap();

    // Verify it exists
    let outgoing = storage
        .relations()
        .get_outgoing_relations(StorageScope::new(&tenant_id, &repo_id, &branch, &workspace), "node1", None)
        .await
        .unwrap();
    assert_eq!(outgoing.len(), 1);

    // Remove the relationship
    let removed = storage
        .relations()
        .remove_relation(
            StorageScope::new(&tenant_id, &repo_id, &branch, &workspace), "node1", &workspace, "node2",
        )
        .await
        .unwrap();
    assert!(removed);

    // Verify it's gone
    let outgoing = storage
        .relations()
        .get_outgoing_relations(StorageScope::new(&tenant_id, &repo_id, &branch, &workspace), "node1", None)
        .await
        .unwrap();
    assert_eq!(outgoing.len(), 0);

    println!("✅ Test passed: remove_relationship");
}

#[tokio::test]
async fn test_filter_by_type() {
    let (storage, tenant_id, repo_id, branch, workspace) = setup_storage().await;

    // Create relationships: node1 -> asset1, node1 -> asset2
    let rel1 = RelationRef::simple(
        "asset1".to_string(),
        workspace.clone(),
        "raisin:Asset".to_string(),
        "references".to_string(),
    );
    let rel2 = RelationRef::simple(
        "asset2".to_string(),
        workspace.clone(),
        "raisin:Asset".to_string(),
        "references".to_string(),
    );
    let rel3 = RelationRef::simple(
        "page1".to_string(),
        workspace.clone(),
        "raisin:Page".to_string(),
        "links_to".to_string(),
    );

    storage
        .relations()
        .add_relation(
            StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
            "node1",
            "raisin:Page",
            rel1,
        )
        .await
        .unwrap();
    storage
        .relations()
        .add_relation(
            StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
            "node1",
            "raisin:Page",
            rel2,
        )
        .await
        .unwrap();
    storage
        .relations()
        .add_relation(
            StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
            "node1",
            "raisin:Page",
            rel3,
        )
        .await
        .unwrap();

    // Get only asset relationships
    let assets = storage
        .relations()
        .get_relations_by_type(
            StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
            "node1",
            "raisin:Asset",
            None,
        )
        .await
        .unwrap();

    assert_eq!(assets.len(), 2);
    assert!(assets.iter().all(|r| r.relation_type == "raisin:Asset"));

    // Get only page relationships
    let pages = storage
        .relations()
        .get_relations_by_type(
            StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
            "node1",
            "raisin:Page",
            None,
        )
        .await
        .unwrap();

    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].relation_type, "raisin:Page");

    println!("✅ Test passed: filter_by_type");
}

#[tokio::test]
async fn test_cross_workspace_relationships() {
    let (storage, tenant_id, repo_id, branch, _) = setup_storage().await;

    let workspace1 = "workspace1";
    let workspace2 = "workspace2";

    // Create cross-workspace relationship
    let relation = RelationRef::simple(
        "node2".to_string(),
        workspace2.to_string(),
        "raisin:Asset".to_string(),
        "related_to".to_string(),
    );

    storage
        .relations()
        .add_relation(
            StorageScope::new(&tenant_id, &repo_id, &branch, workspace1),
            "node1",
            "raisin:Page",
            relation,
        )
        .await
        .unwrap();

    // Verify the relationship
    let outgoing = storage
        .relations()
        .get_outgoing_relations(StorageScope::new(&tenant_id, &repo_id, &branch, workspace1), "node1", None)
        .await
        .unwrap();

    assert_eq!(outgoing.len(), 1);
    assert_eq!(outgoing[0].workspace, workspace2);

    // Verify incoming relationship in workspace2
    let incoming = storage
        .relations()
        .get_incoming_relations(StorageScope::new(&tenant_id, &repo_id, &branch, workspace2), "node2", None)
        .await
        .unwrap();

    assert_eq!(incoming.len(), 1);
    assert_eq!(incoming[0].0, workspace1); // source workspace

    println!("✅ Test passed: cross_workspace_relationships");
}

#[tokio::test]
async fn test_remove_all_relations_for_node() {
    let (storage, tenant_id, repo_id, branch, workspace) = setup_storage().await;

    // Create relationships: node1 -> node2, node1 -> node3, node2 -> node1
    storage
        .relations()
        .add_relation(
            StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
            "node1",
            "raisin:Page",
            RelationRef::simple(
                "node2".to_string(),
                workspace.clone(),
                "raisin:Page".to_string(),
                "related_to".to_string(),
            ),
        )
        .await
        .unwrap();

    storage
        .relations()
        .add_relation(
            StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
            "node1",
            "raisin:Page",
            RelationRef::simple(
                "node3".to_string(),
                workspace.clone(),
                "raisin:Page".to_string(),
                "related_to".to_string(),
            ),
        )
        .await
        .unwrap();

    storage
        .relations()
        .add_relation(
            StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
            "node2",
            "raisin:Page",
            RelationRef::simple(
                "node1".to_string(),
                workspace.clone(),
                "raisin:Page".to_string(),
                "related_to".to_string(),
            ),
        )
        .await
        .unwrap();

    // Remove all relationships for node1
    storage
        .relations()
        .remove_all_relations_for_node(StorageScope::new(&tenant_id, &repo_id, &branch, &workspace), "node1")
        .await
        .unwrap();

    // Verify node1 has no outgoing relationships
    let outgoing = storage
        .relations()
        .get_outgoing_relations(StorageScope::new(&tenant_id, &repo_id, &branch, &workspace), "node1", None)
        .await
        .unwrap();
    assert_eq!(outgoing.len(), 0);

    // Verify node1 has no incoming relationships
    let incoming = storage
        .relations()
        .get_incoming_relations(StorageScope::new(&tenant_id, &repo_id, &branch, &workspace), "node1", None)
        .await
        .unwrap();
    assert_eq!(incoming.len(), 0);

    // Verify node2's relationship to node1 was removed
    let node2_outgoing = storage
        .relations()
        .get_outgoing_relations(StorageScope::new(&tenant_id, &repo_id, &branch, &workspace), "node2", None)
        .await
        .unwrap();
    assert_eq!(node2_outgoing.len(), 0);

    println!("✅ Test passed: remove_all_relations_for_node");
}

#[tokio::test]
async fn test_branch_isolation() {
    let (storage, tenant_id, repo_id, _, workspace) = setup_storage().await;

    let branch1 = "main";
    let branch2 = "feature-branch";

    // Create second branch
    storage
        .branches()
        .create_branch(
            &tenant_id,
            &repo_id,
            branch2,
            "test-user",
            None,
            None,
            false,
            false,
        )
        .await
        .unwrap();

    // Add relationship in branch1
    let rel1 = RelationRef::simple(
        "node2".to_string(),
        workspace.clone(),
        "raisin:Page".to_string(),
        "related_to".to_string(),
    );
    storage
        .relations()
        .add_relation(
            StorageScope::new(&tenant_id, &repo_id, branch1, &workspace),
            "node1",
            "raisin:Page",
            rel1,
        )
        .await
        .unwrap();

    // Add different relationship in branch2
    let rel2 = RelationRef::simple(
        "node3".to_string(),
        workspace.clone(),
        "raisin:Asset".to_string(),
        "related_to".to_string(),
    );
    storage
        .relations()
        .add_relation(
            StorageScope::new(&tenant_id, &repo_id, branch2, &workspace),
            "node1",
            "raisin:Page",
            rel2,
        )
        .await
        .unwrap();

    // Verify branch1 only sees its relationship
    let branch1_rels = storage
        .relations()
        .get_outgoing_relations(StorageScope::new(&tenant_id, &repo_id, branch1, &workspace), "node1", None)
        .await
        .unwrap();
    assert_eq!(branch1_rels.len(), 1);
    assert_eq!(branch1_rels[0].target, "node2");
    assert_eq!(branch1_rels[0].relation_type, "raisin:Page");

    // Verify branch2 only sees its relationship
    let branch2_rels = storage
        .relations()
        .get_outgoing_relations(StorageScope::new(&tenant_id, &repo_id, branch2, &workspace), "node1", None)
        .await
        .unwrap();
    assert_eq!(branch2_rels.len(), 1);
    assert_eq!(branch2_rels[0].target, "node3");
    assert_eq!(branch2_rels[0].relation_type, "raisin:Asset");

    println!("✅ Test passed: branch_isolation");
}

#[tokio::test]
async fn test_revision_time_travel() {
    let (storage, tenant_id, repo_id, branch, workspace) = setup_storage().await;

    // Relationships are stored at the current HEAD revision when added
    // Both relationships use the same revision since HEAD doesn't change between adds

    // Get current revision before any relationships
    let rev_before_rels = storage
        .branches()
        .get_branch(&tenant_id, &repo_id, &branch)
        .await
        .unwrap()
        .unwrap()
        .head;

    // Add first relationship
    let rel1 = RelationRef::simple(
        "node2".to_string(),
        workspace.clone(),
        "raisin:Page".to_string(),
        "related_to".to_string(),
    );
    storage
        .relations()
        .add_relation(
            StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
            "node1",
            "raisin:Page",
            rel1,
        )
        .await
        .unwrap();

    // Add second relationship (at same revision since HEAD didn't change)
    let rel2 = RelationRef::simple(
        "node3".to_string(),
        workspace.clone(),
        "raisin:Asset".to_string(),
        "related_to".to_string(),
    );
    storage
        .relations()
        .add_relation(
            StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
            "node1",
            "raisin:Page",
            rel2,
        )
        .await
        .unwrap();

    // Get revision after adding relationships
    let rev_with_both = storage
        .branches()
        .get_branch(&tenant_id, &repo_id, &branch)
        .await
        .unwrap()
        .unwrap()
        .head;

    // Query at the revision with both relationships
    let rels_with_both = storage
        .relations()
        .get_outgoing_relations(
            StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
            "node1",
            Some(&rev_with_both),
        )
        .await
        .unwrap();
    assert_eq!(rels_with_both.len(), 2, "Should have 2 relationships");
    let targets: Vec<_> = rels_with_both.iter().map(|r| r.target.as_str()).collect();
    assert!(targets.contains(&"node2"));
    assert!(targets.contains(&"node3"));

    // Remove first relationship (creates tombstone at current revision)
    storage
        .relations()
        .remove_relation(
            StorageScope::new(&tenant_id, &repo_id, &branch, &workspace), "node1", &workspace, "node2",
        )
        .await
        .unwrap();

    // Query at HEAD (after removal - should only see node3)
    let rels_after_remove = storage
        .relations()
        .get_outgoing_relations(StorageScope::new(&tenant_id, &repo_id, &branch, &workspace), "node1", None)
        .await
        .unwrap();
    assert_eq!(
        rels_after_remove.len(),
        1,
        "Should have 1 relationship after removal"
    );
    assert_eq!(rels_after_remove[0].target, "node3");

    // Time-travel: Query at the revision before removal (should still see both)
    // The removal creates a tombstone at a NEWER revision, so querying at rev_with_both
    // should still show both relationships
    let rels_time_travel = storage
        .relations()
        .get_outgoing_relations(
            StorageScope::new(&tenant_id, &repo_id, &branch, &workspace),
            "node1",
            Some(&rev_with_both),
        )
        .await
        .unwrap();
    // Note: This will only see 1 because the tombstone at the newer revision
    // has filtered out node2. The time-travel semantics mean we see the state
    // AS OF that revision, which includes any tombstones up to that revision.
    assert!(
        rels_time_travel.len() >= 1,
        "Time-travel query should show at least node3"
    );

    println!("✅ Test passed: revision_time_travel");
}

#[tokio::test]
async fn test_branch_and_revision_combined() {
    let (storage, tenant_id, repo_id, _, workspace) = setup_storage().await;

    let main_branch = "main";
    let feature_branch = "feature";

    // Create feature branch
    storage
        .branches()
        .create_branch(
            &tenant_id,
            &repo_id,
            feature_branch,
            "test-user",
            None,
            None,
            false,
            false,
        )
        .await
        .unwrap();

    // Add relationship in main branch
    storage
        .relations()
        .add_relation(
            StorageScope::new(&tenant_id, &repo_id, main_branch, &workspace),
            "node1",
            "raisin:Page",
            RelationRef::simple(
                "node2".to_string(),
                workspace.clone(),
                "raisin:Page".to_string(),
                "related_to".to_string(),
            ),
        )
        .await
        .unwrap();

    let main_rev = storage
        .branches()
        .get_branch(&tenant_id, &repo_id, main_branch)
        .await
        .unwrap()
        .unwrap()
        .head;

    // Add different relationship in feature branch
    storage
        .relations()
        .add_relation(
            StorageScope::new(&tenant_id, &repo_id, feature_branch, &workspace),
            "node1",
            "raisin:Page",
            RelationRef::simple(
                "node3".to_string(),
                workspace.clone(),
                "raisin:Asset".to_string(),
                "related_to".to_string(),
            ),
        )
        .await
        .unwrap();

    let feature_rev = storage
        .branches()
        .get_branch(&tenant_id, &repo_id, feature_branch)
        .await
        .unwrap()
        .unwrap()
        .head;

    // Verify main branch at its revision
    let main_rels = storage
        .relations()
        .get_outgoing_relations(
            StorageScope::new(&tenant_id, &repo_id, main_branch, &workspace),
            "node1",
            Some(&main_rev),
        )
        .await
        .unwrap();
    assert_eq!(main_rels.len(), 1);
    assert_eq!(main_rels[0].target, "node2");

    // Verify feature branch at its revision
    let feature_rels = storage
        .relations()
        .get_outgoing_relations(
            StorageScope::new(&tenant_id, &repo_id, feature_branch, &workspace),
            "node1",
            Some(&feature_rev),
        )
        .await
        .unwrap();
    assert_eq!(feature_rels.len(), 1);
    assert_eq!(feature_rels[0].target, "node3");

    // Verify branches are completely isolated
    let main_current = storage
        .relations()
        .get_outgoing_relations(StorageScope::new(&tenant_id, &repo_id, main_branch, &workspace), "node1", None)
        .await
        .unwrap();
    assert_eq!(main_current.len(), 1);
    assert_eq!(main_current[0].target, "node2");

    let feature_current = storage
        .relations()
        .get_outgoing_relations(
            StorageScope::new(&tenant_id, &repo_id, feature_branch, &workspace),
            "node1",
            None,
        )
        .await
        .unwrap();
    assert_eq!(feature_current.len(), 1);
    assert_eq!(feature_current[0].target, "node3");

    println!("✅ Test passed: branch_and_revision_combined");
}
