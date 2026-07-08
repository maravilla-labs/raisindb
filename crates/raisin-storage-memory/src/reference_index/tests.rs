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

// NOTE: the reverse reference index is keyed by the TARGET's stable node id
// (not its path), so `find_referencing_nodes` takes the target id — these tests
// look up by the `id` passed to `make_reference`, not the path.

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

    // Find nodes referencing the target (by target id)
    let results = repo
        .find_referencing_nodes(scope("tenant1", "repo1", "main", "ws1"), "ws1", "1", false)
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
async fn test_retarget_forward_path_updates_denorm_path() {
    let repo = InMemoryReferenceIndexRepo::new();

    let mut props = HashMap::new();
    props.insert(
        "hero".to_string(),
        PropertyValue::Reference(make_reference("1", "ws1", "/assets/hero.png")),
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

    // Target "1" moved to a new path; retarget the referrer's forward entry.
    let moved = make_reference("1", "ws1", "/assets/moved/hero.png");
    repo.retarget_forward_path(
        scope("tenant1", "repo1", "main", "ws1"),
        "node1",
        "hero",
        &moved,
        &test_revision(),
        false,
    )
    .await
    .unwrap();

    // Forward path is fresh; id/workspace preserved.
    let node_refs = repo
        .get_node_references(scope("tenant1", "repo1", "main", "ws1"), "node1", false)
        .await
        .unwrap();
    assert_eq!(node_refs.len(), 1);
    assert_eq!(node_refs[0].1.path, "/assets/moved/hero.png");
    assert_eq!(node_refs[0].1.id, "1");
    assert_eq!(node_refs[0].1.workspace, "ws1");

    // Reverse index (keyed by stable target id) is unchanged.
    let referrers = repo
        .find_referencing_nodes(scope("tenant1", "repo1", "main", "ws1"), "ws1", "1", false)
        .await
        .unwrap();
    assert_eq!(referrers, vec![("node1".to_string(), "hero".to_string())]);
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
        .find_referencing_nodes(scope("tenant1", "repo1", "main", "ws1"), "ws1", "1", false)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);

    // Should NOT find in published
    let results = repo
        .find_referencing_nodes(scope("tenant1", "repo1", "main", "ws1"), "ws1", "1", true)
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
        .find_referencing_nodes(scope("tenant1", "repo1", "main", "ws1"), "ws1", "1", true)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);

    // Should NOT find in draft anymore
    let results = repo
        .find_referencing_nodes(scope("tenant1", "repo1", "main", "ws1"), "ws1", "1", false)
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
        .find_referencing_nodes(scope("tenant1", "repo1", "main", "ws1"), "ws1", "1", false)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "node1");

    // repo2 should only see node2
    let results = repo
        .find_referencing_nodes(scope("tenant1", "repo2", "main", "ws1"), "ws1", "1", false)
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
        .find_referencing_nodes(scope("tenant1", "repo1", "main", "ws1"), "ws1", "1", false)
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
        .find_referencing_nodes(scope("tenant1", "repo1", "main", "ws1"), "ws1", "1", false)
        .await
        .unwrap();
    assert_eq!(results.len(), 0);

    let results = repo
        .find_referencing_nodes(scope("tenant1", "repo1", "main", "ws1"), "ws1", "2", false)
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

    // Should find both property paths (by target id)
    let results = repo
        .find_referencing_nodes(scope("tenant1", "repo1", "main", "ws1"), "ws1", "1", false)
        .await
        .unwrap();
    assert_eq!(results.len(), 2);

    // Get unique references (deduplicated). NOTE: the forward grouping key
    // (`reference_target_key`) is still "{workspace}:{path}" — unchanged.
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

    // Reverse lookup should work (by target id)
    let results = repo
        .find_referencing_nodes(scope("tenant1", "repo1", "main", "ws1"), "ws1", "bg", false)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "node1");
    assert_eq!(results[0].1, "sections.0.background");
}

/// The reverse index is keyed by the target's stable id, so a reference indexed
/// against an OLD path is still found by id (this is the whole point — it
/// survives the target moving to a new path). Looking up by the path string
/// must NOT match.
#[tokio::test]
async fn test_reverse_lookup_keyed_by_id_not_path() {
    let repo = InMemoryReferenceIndexRepo::new();

    let mut props = HashMap::new();
    props.insert(
        "image".to_string(),
        PropertyValue::Reference(make_reference("asset-uuid", "ws1", "/assets/old.png")),
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

    // Found by stable id...
    let by_id = repo
        .find_referencing_nodes(
            scope("tenant1", "repo1", "main", "ws1"),
            "ws1",
            "asset-uuid",
            false,
        )
        .await
        .unwrap();
    assert_eq!(by_id.len(), 1);
    assert_eq!(by_id[0].0, "node1");

    // ...but NOT by the path (proves it is not path-keyed). Even after the
    // target moves to "/assets/new.png", the id lookup above keeps working
    // with no re-indexing of the referrer.
    let by_old_path = repo
        .find_referencing_nodes(
            scope("tenant1", "repo1", "main", "ws1"),
            "ws1",
            "/assets/old.png",
            false,
        )
        .await
        .unwrap();
    assert_eq!(by_old_path.len(), 0);

    let by_new_path = repo
        .find_referencing_nodes(
            scope("tenant1", "repo1", "main", "ws1"),
            "ws1",
            "/assets/new.png",
            false,
        )
        .await
        .unwrap();
    assert_eq!(by_new_path.len(), 0);
}

/// A UUID reference whose target does not exist yet is still indexed by id and
/// is found once we look it up by that id.
#[tokio::test]
async fn test_reference_to_not_yet_existing_target() {
    let repo = InMemoryReferenceIndexRepo::new();

    let mut props = HashMap::new();
    props.insert(
        "ref".to_string(),
        // No path populated — target not created yet.
        PropertyValue::Reference(make_reference("future-uuid", "ws1", "")),
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

    let results = repo
        .find_referencing_nodes(
            scope("tenant1", "repo1", "main", "ws1"),
            "ws1",
            "future-uuid",
            false,
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "node1");
}
