use super::*;
use raisin_storage::scope::StorageScope;

fn test_revision() -> raisin_hlc::HLC {
    raisin_hlc::HLC::new(1, 0)
}

fn scope<'a>(
    tenant_id: &'a str,
    repo_id: &'a str,
    branch: &'a str,
    workspace: &'a str,
) -> StorageScope<'a> {
    StorageScope::new(tenant_id, repo_id, branch, workspace)
}

fn make_reference(id: &str, workspace: &str, path: &str) -> RaisinReference {
    RaisinReference {
        id: id.to_string(),
        workspace: workspace.to_string(),
        path: path.to_string(),
    }
}

#[tokio::test]
async fn test_index_and_find_references() {
    let repo = InMemoryReferenceIndexRepo::new();

    let mut props = HashMap::new();
    props.insert(
        "hero".to_string(),
        PropertyValue::Reference(make_reference("1", "ws1", "/assets/hero.png")),
    );

    // Index reference as draft
    repo.index_references(
        scope("tenant1", "repo1", "main", "ws1"),
        "node1",
        &props,
        &test_revision(),
        false,
    )
    .await
    .unwrap();

    // Find nodes referencing the target
    let results = repo
        .find_referencing_nodes(
            scope("tenant1", "repo1", "main", "ws1"),
            "ws1",
            "/assets/hero.png",
            false,
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "node1");
    assert_eq!(results[0].1, "hero");

    // Get references from node
    let node_refs = repo
        .get_node_references(scope("tenant1", "repo1", "main", "ws1"), "node1", false)
        .await
        .unwrap();

    assert_eq!(node_refs.len(), 1);
    assert_eq!(node_refs[0].0, "hero");
    assert_eq!(node_refs[0].1.path, "/assets/hero.png");
}

#[tokio::test]
async fn test_publish_status_update() {
    let repo = InMemoryReferenceIndexRepo::new();

    let mut props = HashMap::new();
    props.insert(
        "image".to_string(),
        PropertyValue::Reference(make_reference("1", "ws1", "/img.png")),
    );

    // Index as draft
    repo.index_references(
        scope("tenant1", "repo1", "main", "ws1"),
        "node1",
        &props,
        &test_revision(),
        false,
    )
    .await
    .unwrap();

    // Should find in draft
    let results = repo
        .find_referencing_nodes(
            scope("tenant1", "repo1", "main", "ws1"),
            "ws1",
            "/img.png",
            false,
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 1);

    // Should NOT find in published
    let results = repo
        .find_referencing_nodes(
            scope("tenant1", "repo1", "main", "ws1"),
            "ws1",
            "/img.png",
            true,
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 0);

    // Update to published
    repo.update_reference_publish_status(
        scope("tenant1", "repo1", "main", "ws1"),
        "node1",
        &props,
        &test_revision(),
        true,
    )
    .await
    .unwrap();

    // Should now find in published
    let results = repo
        .find_referencing_nodes(
            scope("tenant1", "repo1", "main", "ws1"),
            "ws1",
            "/img.png",
            true,
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 1);

    // Should NOT find in draft anymore
    let results = repo
        .find_referencing_nodes(
            scope("tenant1", "repo1", "main", "ws1"),
            "ws1",
            "/img.png",
            false,
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 0);
}

#[tokio::test]
async fn test_workspace_isolation() {
    let repo = InMemoryReferenceIndexRepo::new();

    let mut props = HashMap::new();
    props.insert(
        "asset".to_string(),
        PropertyValue::Reference(make_reference("1", "ws1", "/shared.png")),
    );

    // Index same reference in two repos
    repo.index_references(
        scope("tenant1", "repo1", "main", "ws1"),
        "node1",
        &props,
        &test_revision(),
        false,
    )
    .await
    .unwrap();
    repo.index_references(
        scope("tenant1", "repo2", "main", "ws1"),
        "node2",
        &props,
        &test_revision(),
        false,
    )
    .await
    .unwrap();

    // repo1 should only see node1
    let results = repo
        .find_referencing_nodes(
            scope("tenant1", "repo1", "main", "ws1"),
            "ws1",
            "/shared.png",
            false,
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "node1");

    // repo2 should only see node2
    let results = repo
        .find_referencing_nodes(
            scope("tenant1", "repo2", "main", "ws1"),
            "ws1",
            "/shared.png",
            false,
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "node2");
}

#[tokio::test]
async fn test_unindex_references() {
    let repo = InMemoryReferenceIndexRepo::new();

    let mut props = HashMap::new();
    props.insert(
        "img1".to_string(),
        PropertyValue::Reference(make_reference("1", "ws1", "/img1.png")),
    );
    props.insert(
        "img2".to_string(),
        PropertyValue::Reference(make_reference("2", "ws1", "/img2.png")),
    );

    // Index references
    repo.index_references(
        scope("tenant1", "repo1", "main", "ws1"),
        "node1",
        &props,
        &test_revision(),
        false,
    )
    .await
    .unwrap();

    // Verify indexed
    let results = repo
        .find_referencing_nodes(
            scope("tenant1", "repo1", "main", "ws1"),
            "ws1",
            "/img1.png",
            false,
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 1);

    // Unindex all references
    repo.unindex_references(
        scope("tenant1", "repo1", "main", "ws1"),
        "node1",
        &props,
        &test_revision(),
    )
    .await
    .unwrap();

    // Should no longer find
    let results = repo
        .find_referencing_nodes(
            scope("tenant1", "repo1", "main", "ws1"),
            "ws1",
            "/img1.png",
            false,
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 0);

    let results = repo
        .find_referencing_nodes(
            scope("tenant1", "repo1", "main", "ws1"),
            "ws1",
            "/img2.png",
            false,
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 0);
}

#[tokio::test]
async fn test_multiple_references_to_same_target() {
    let repo = InMemoryReferenceIndexRepo::new();

    let mut props = HashMap::new();
    props.insert(
        "hero".to_string(),
        PropertyValue::Reference(make_reference("1", "ws1", "/logo.png")),
    );
    props.insert(
        "footer".to_string(),
        PropertyValue::Reference(make_reference("1", "ws1", "/logo.png")),
    );

    repo.index_references(
        scope("tenant1", "repo1", "main", "ws1"),
        "node1",
        &props,
        &test_revision(),
        false,
    )
    .await
    .unwrap();

    // Should find both property paths
    let results = repo
        .find_referencing_nodes(
            scope("tenant1", "repo1", "main", "ws1"),
            "ws1",
            "/logo.png",
            false,
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 2);

    // Get unique references (deduplicated)
    let unique = repo
        .get_unique_references(scope("tenant1", "repo1", "main", "ws1"), "node1", false)
        .await
        .unwrap();

    assert_eq!(unique.len(), 1);
    let target_key = "ws1:/logo.png";
    assert!(unique.contains_key(target_key));

    let (paths, reference) = &unique[target_key];
    assert_eq!(paths.len(), 2);
    assert!(paths.contains(&"hero".to_string()));
    assert!(paths.contains(&"footer".to_string()));
    assert_eq!(reference.path, "/logo.png");
}

#[tokio::test]
async fn test_nested_references() {
    let repo = InMemoryReferenceIndexRepo::new();

    let mut inner = HashMap::new();
    inner.insert(
        "background".to_string(),
        PropertyValue::Reference(make_reference("bg", "ws1", "/bg.png")),
    );

    let mut props = HashMap::new();
    props.insert(
        "sections".to_string(),
        PropertyValue::Array(vec![PropertyValue::Object(inner)]),
    );

    repo.index_references(
        scope("tenant1", "repo1", "main", "ws1"),
        "node1",
        &props,
        &test_revision(),
        false,
    )
    .await
    .unwrap();

    // Should find with nested path
    let node_refs = repo
        .get_node_references(scope("tenant1", "repo1", "main", "ws1"), "node1", false)
        .await
        .unwrap();

    assert_eq!(node_refs.len(), 1);
    assert_eq!(node_refs[0].0, "sections.0.background");
    assert_eq!(node_refs[0].1.path, "/bg.png");

    // Reverse lookup should work
    let results = repo
        .find_referencing_nodes(
            scope("tenant1", "repo1", "main", "ws1"),
            "ws1",
            "/bg.png",
            false,
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "node1");
    assert_eq!(results[0].1, "sections.0.background");
}
