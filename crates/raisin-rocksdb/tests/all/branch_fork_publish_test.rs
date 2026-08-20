// SPDX-License-Identifier: BSL-1.1
//
//! Reproduces the Studio "branch-based publish" de-risking spikes end-to-end at
//! the storage layer:
//!
//!  1. `fromBranch`-style fork (upstream branch, no explicit revision) must copy
//!     the source branch's data — including the archetype registry — so that
//!     archetype lookups succeed on the new branch (previously failed with
//!     "Archetype not found").
//!  2. Branch compare (`calculate_divergence`) must work on the forked branch.
//!  3. Branch merge (`merge_branches`) must work on the forked branch.
//!
//! See plan: unblock Studio's branch-based publish/unpublish.

use raisin_context::{MergeStrategy, RepositoryConfig};
use raisin_error::Result;
use raisin_models::nodes::types::archetype::Archetype;
use raisin_models::nodes::Node;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::scope::{BranchScope, StorageScope};
use raisin_storage::{
    ArchetypeRepository, BranchRepository, CommitMetadata, CreateNodeOptions, DeleteNodeOptions,
    ListOptions, NodeRepository, RegistryRepository, RepositoryManagementRepository, Storage,
};
use std::collections::HashMap;
use tempfile::TempDir;

const TENANT: &str = "test-tenant";
const REPO: &str = "test-repo";

struct TestStorage {
    storage: RocksDBStorage,
    _temp_dir: TempDir,
}

impl TestStorage {
    /// Storage with the default tenant, repo, and `main` branch created.
    async fn new() -> Result<Self> {
        let temp_dir =
            tempfile::tempdir().map_err(|e| raisin_error::Error::Backend(e.to_string()))?;
        let storage = RocksDBStorage::new(temp_dir.path())?;

        storage
            .registry()
            .register_tenant(TENANT, HashMap::new())
            .await?;

        let repo_config = RepositoryConfig {
            default_language: "en".to_string(),
            supported_languages: vec!["en".to_string()],
            locale_fallback_chains: HashMap::new(),
            default_branch: "main".to_string(),
            description: Some("branch fork publish test".to_string()),
            tags: HashMap::new(),
        };
        storage
            .repository_management()
            .create_repository(TENANT, REPO, repo_config)
            .await?;

        storage
            .branches()
            .create_branch(TENANT, REPO, "main", "test-user", None, None, false, false)
            .await?;

        Ok(Self {
            storage,
            _temp_dir: temp_dir,
        })
    }
}

fn commit() -> CommitMetadata {
    CommitMetadata {
        message: "test".to_string(),
        actor: "test-user".to_string(),
        is_system: false,
    }
}

/// Build a minimal archetype using serde defaults (auto id, all-None options).
fn archetype(name: &str) -> Archetype {
    serde_json::from_value(serde_json::json!({ "name": name }))
        .expect("archetype should deserialize from name-only JSON")
}

/// `fromBranch: "main"` (upstream branch, no explicit revision) must produce a
/// real fork: the archetype registered on `main` resolves on `publish`.
#[tokio::test]
async fn fork_from_branch_copies_archetype_registry() -> Result<()> {
    let t = TestStorage::new().await?;
    let storage = &t.storage;

    // Register an archetype on main.
    let main_scope = BranchScope::new(TENANT, REPO, "main");
    storage
        .archetypes()
        .create(main_scope, archetype("test:StandardPage"), commit())
        .await?;

    // Sanity: it resolves on main.
    assert!(
        storage
            .archetypes()
            .get(main_scope, "test:StandardPage", None)
            .await?
            .is_some(),
        "archetype should resolve on main"
    );

    // Fork via upstream branch only (no explicit from_revision) — the Studio
    // `branches().create('publish', { fromBranch: 'main' })` path.
    storage
        .branches()
        .create_branch(
            TENANT,
            REPO,
            "publish",
            "test-user",
            None,                     // from_revision: None
            Some("main".to_string()), // upstream_branch
            false,
            false,
        )
        .await?;

    // The fix: archetype is carried onto the forked branch.
    let publish_scope = BranchScope::new(TENANT, REPO, "publish");
    let resolved = storage
        .archetypes()
        .get(publish_scope, "test:StandardPage", None)
        .await?;
    assert!(
        resolved.is_some(),
        "archetype must resolve on the forked publish branch (regression: 'Archetype not found')"
    );

    Ok(())
}

/// Control: a branch created with neither a revision nor an upstream branch is
/// still empty (the fix is additive and does not change this behavior).
#[tokio::test]
async fn fork_without_source_is_empty() -> Result<()> {
    let t = TestStorage::new().await?;
    let storage = &t.storage;

    let main_scope = BranchScope::new(TENANT, REPO, "main");
    storage
        .archetypes()
        .create(main_scope, archetype("test:StandardPage"), commit())
        .await?;

    storage
        .branches()
        .create_branch(TENANT, REPO, "blank", "test-user", None, None, false, false)
        .await?;

    let blank_scope = BranchScope::new(TENANT, REPO, "blank");
    assert!(
        storage
            .archetypes()
            .get(blank_scope, "test:StandardPage", None)
            .await?
            .is_none(),
        "a branch created with no source must remain empty"
    );

    Ok(())
}

/// Compare (divergence) and merge must work on a forked branch — the two
/// WebSocket handlers now delegate to these exact RocksDB methods.
#[tokio::test]
async fn compare_and_merge_work_on_forked_branch() -> Result<()> {
    let t = TestStorage::new().await?;
    let storage = &t.storage;

    storage
        .archetypes()
        .create(
            BranchScope::new(TENANT, REPO, "main"),
            archetype("test:StandardPage"),
            commit(),
        )
        .await?;

    storage
        .branches()
        .create_branch(
            TENANT,
            REPO,
            "publish",
            "test-user",
            None,
            Some("main".to_string()),
            false,
            false,
        )
        .await?;

    // Compare path (WS handle_branch_compare).
    let divergence = storage
        .branches_impl()
        .calculate_divergence(TENANT, REPO, "publish", "main")
        .await?;
    // Freshly forked at main's head -> no divergence.
    let _ = divergence; // just assert the call succeeds

    // Merge path (WS handle_branch_merge): merge publish back into main.
    let result = storage
        .branches_impl()
        .merge_branches(
            TENANT,
            REPO,
            "main",    // target
            "publish", // source
            MergeStrategy::ThreeWay,
            "merge publish into main",
            "test-user",
        )
        .await?;
    assert!(
        result.success,
        "merging a freshly forked branch back into its source must succeed"
    );

    Ok(())
}

const WORKSPACE: &str = "default";

/// Minimal node builder (parent derived from path; ordering label is assigned by
/// the index on create, so `order_key` here is just a placeholder).
fn make_node(path: &str, node_type: &str) -> Node {
    let name = path.rsplit('/').next().unwrap_or(path).to_string();
    let parent = match path.rsplitn(2, '/').nth(1) {
        Some(p) if !p.is_empty() => Some(p.to_string()),
        _ => None,
    };
    Node {
        id: uuid::Uuid::new_v4().to_string(),
        path: path.to_string(),
        name,
        parent,
        node_type: node_type.to_string(),
        properties: HashMap::new(),
        children: Vec::new(),
        order_key: "a0".to_string(),
        has_children: None,
        version: 1,
        archetype: None,
        created_at: Some(chrono::Utc::now()),
        updated_at: Some(chrono::Utc::now()),
        created_by: Some("test-user".to_string()),
        updated_by: Some("test-user".to_string()),
        published_at: None,
        published_by: None,
        translations: None,
        tenant_id: Some(TENANT.to_string()),
        workspace: Some(WORKSPACE.to_string()),
        owner_id: None,
        relations: Vec::new(),
    }
}

/// CreateNodeOptions with validation disabled — this test targets ordering /
/// merge / reorder behavior, not schema validation.
fn no_validation() -> CreateNodeOptions {
    CreateNodeOptions {
        validate_schema: false,
        validate_parent_allows_child: false,
        validate_workspace_allows_type: false,
        operation_meta: None,
    }
}

/// Regression: reorder after inter-branch copy/merge must self-heal colliding
/// fractional order labels.
///
/// Forking a branch and appending a child on BOTH sides mints the SAME fractional
/// order_label under one parent; merging them leaves two children sharing that
/// label, and the next reorder asked `between()` to insert between two equal
/// labels -> "First label must be less than second label". The reorder path now
/// rebalances the parent first, so the move succeeds.
#[tokio::test]
async fn reorder_self_heals_after_branch_merge_label_collision() -> Result<()> {
    let t = TestStorage::new().await?;
    let storage = &t.storage;
    let nodes = storage.nodes();

    let main = || StorageScope::new(TENANT, REPO, "main", WORKSPACE);
    let feature = || StorageScope::new(TENANT, REPO, "feature", WORKSPACE);

    // Parent + two children on main, created BEFORE the fork (shared labels).
    nodes
        .create(
            main(),
            make_node("/parent", "raisin:Folder"),
            no_validation(),
        )
        .await?;
    nodes
        .create(
            main(),
            make_node("/parent/a", "raisin:Page"),
            no_validation(),
        )
        .await?;
    nodes
        .create(
            main(),
            make_node("/parent/b", "raisin:Page"),
            no_validation(),
        )
        .await?;

    // Fork `feature` from `main` (copies ORDERED_CHILDREN verbatim).
    storage
        .branches()
        .create_branch(
            TENANT,
            REPO,
            "feature",
            "test-user",
            None,
            Some("main".to_string()),
            false,
            false,
        )
        .await?;

    // Append a child on BOTH branches. Each appends after `b`, so both get the
    // same fractional label (inc of b's label) under /parent.
    nodes
        .create(
            main(),
            make_node("/parent/c", "raisin:Page"),
            no_validation(),
        )
        .await?;
    nodes
        .create(
            feature(),
            make_node("/parent/d", "raisin:Page"),
            no_validation(),
        )
        .await?;

    // Merge feature -> main: /parent now has c and d sharing one fractional label.
    let merge = storage
        .branches_impl()
        .merge_branches(
            TENANT,
            REPO,
            "main",
            "feature",
            MergeStrategy::ThreeWay,
            "merge feature into main",
            "test-user",
        )
        .await?;
    assert!(merge.success, "merge should succeed: {:?}", merge.conflicts);

    // All four children present on main after the merge (copy must not drop any).
    let before = nodes
        .list_children(main(), "/parent", ListOptions::default())
        .await?;
    let names_before: std::collections::HashSet<_> =
        before.iter().map(|n| n.name.clone()).collect();
    assert_eq!(
        before.len(),
        4,
        "expected a,b,c,d on main after merge, got {:?}",
        names_before
    );

    // The reorder that used to crash with "First label must be less than second
    // label": move `a` after `c`. The parent is rebalanced first, then the move
    // proceeds.
    nodes
        .move_child_after(
            main(),
            "/parent",
            "a",
            "c",
            Some("move a after c"),
            Some("test-user"),
        )
        .await?;

    // After the move: still four children, and `a` immediately follows `c`.
    let after = nodes
        .list_children(main(), "/parent", ListOptions::default())
        .await?;
    let order: Vec<String> = after.iter().map(|n| n.name.clone()).collect();
    assert_eq!(
        after.len(),
        4,
        "no children lost during self-heal + move: {:?}",
        order
    );
    let ci = order.iter().position(|n| n == "c").expect("c present");
    assert_eq!(
        order.get(ci + 1).map(|s| s.as_str()),
        Some("a"),
        "`a` must immediately follow `c`, got {:?}",
        order
    );

    Ok(())
}

/// `apply_child_order_from_branch` replays a parent's sibling order from a source
/// branch onto a target branch — the primitive a selective publish needs to carry
/// node order across branches (Studio's SQL-UPSERT publish never touches the
/// ordering index on its own).
#[tokio::test]
async fn apply_child_order_from_branch_replays_sibling_order() -> Result<()> {
    let t = TestStorage::new().await?;
    let storage = &t.storage;
    let nodes = storage.nodes();

    let main = || StorageScope::new(TENANT, REPO, "main", WORKSPACE);
    let publish = || StorageScope::new(TENANT, REPO, "publish", WORKSPACE);

    // main: /parent with children a, b, c (creation order).
    nodes
        .create(
            main(),
            make_node("/parent", "raisin:Folder"),
            no_validation(),
        )
        .await?;
    for child in ["a", "b", "c"] {
        nodes
            .create(
                main(),
                make_node(&format!("/parent/{child}"), "raisin:Page"),
                no_validation(),
            )
            .await?;
    }

    // Fork publish from main (copies the a,b,c order).
    storage
        .branches()
        .create_branch(
            TENANT,
            REPO,
            "publish",
            "test-user",
            None,
            Some("main".to_string()),
            false,
            false,
        )
        .await?;

    // Reorder on main to c, a, b (the "order changed" the user asked about).
    nodes
        .move_child_after(main(), "/parent", "a", "c", None, Some("test-user"))
        .await?;
    nodes
        .move_child_after(main(), "/parent", "b", "a", None, Some("test-user"))
        .await?;
    let main_order: Vec<String> = nodes
        .list_children(main(), "/parent", ListOptions::default())
        .await?
        .iter()
        .map(|n| n.name.clone())
        .collect();
    assert_eq!(main_order, vec!["c", "a", "b"], "main reordered to c,a,b");

    // publish still has the original a,b,c order (selective publish wouldn't fix this).
    let publish_before: Vec<String> = nodes
        .list_children(publish(), "/parent", ListOptions::default())
        .await?
        .iter()
        .map(|n| n.name.clone())
        .collect();
    assert_eq!(
        publish_before,
        vec!["a", "b", "c"],
        "publish starts at a,b,c"
    );

    // Apply main's order onto publish (shared generic helper).
    raisin_storage::apply_child_order_from_branch(
        storage.nodes(),
        TENANT,
        REPO,
        "publish", // target
        "main",    // source
        WORKSPACE,
        "/parent",
        Some("test-user"),
    )
    .await?;

    let publish_after: Vec<String> = nodes
        .list_children(publish(), "/parent", ListOptions::default())
        .await?
        .iter()
        .map(|n| n.name.clone())
        .collect();
    assert_eq!(
        publish_after,
        vec!["c", "a", "b"],
        "publish order must now match main"
    );

    Ok(())
}

/// A reorder must surface in the branch diff as an `operation: "reordered"`
/// entry, so the admin-console merge UI can show it. (Regression: reorders
/// previously recorded no changed_nodes, so they were invisible in diffs.)
#[tokio::test]
async fn reorder_shows_up_in_branch_diff() -> Result<()> {
    let t = TestStorage::new().await?;
    let storage = &t.storage;
    let nodes = storage.nodes();

    let main = || StorageScope::new(TENANT, REPO, "main", WORKSPACE);

    // Parent + three children on main.
    nodes
        .create(
            main(),
            make_node("/parent", "raisin:Folder"),
            no_validation(),
        )
        .await?;
    for child in ["a", "b", "c"] {
        nodes
            .create(
                main(),
                make_node(&format!("/parent/{child}"), "raisin:Page"),
                no_validation(),
            )
            .await?;
    }

    // Fork `feature` from `main`, then reorder on the feature branch only.
    storage
        .branches()
        .create_branch(
            TENANT,
            REPO,
            "feature",
            "test-user",
            None,
            Some("main".to_string()),
            false,
            false,
        )
        .await?;
    let feature = || StorageScope::new(TENANT, REPO, "feature", WORKSPACE);
    nodes
        .move_child_after(feature(), "/parent", "a", "c", None, Some("test-user"))
        .await?;

    // Diff feature against main: the reordered child must appear as "reordered".
    let diff = storage
        .branches_impl()
        .diff_branches(TENANT, REPO, "feature", "main")
        .await?;

    let reordered: Vec<_> = diff
        .modified
        .iter()
        .filter(|n| n.operation == "reordered")
        .collect();
    assert!(
        !reordered.is_empty(),
        "expected a reordered entry in the branch diff, got modified={:?}",
        diff.modified
    );

    Ok(())
}

/// `diff_branches` must also be reachable through the `BranchRepository`
/// trait (not only the concrete `branches_impl()` handle), so generic
/// callers — e.g. the function-runtime callbacks, which are generic over
/// `S: Storage` — can compute per-node branch diffs.
#[tokio::test]
async fn diff_branches_available_via_branch_repository_trait() -> Result<()> {
    let t = TestStorage::new().await?;
    let storage = &t.storage;
    let nodes = storage.nodes();

    let main = || StorageScope::new(TENANT, REPO, "main", WORKSPACE);

    // Seed main with a parent + children, fork `feature`, then change the
    // child order on the feature branch only (an operation that records
    // revision metadata at this layer, so it surfaces in the diff).
    nodes
        .create(
            main(),
            make_node("/parent", "raisin:Folder"),
            no_validation(),
        )
        .await?;
    for child in ["a", "b", "c"] {
        nodes
            .create(
                main(),
                make_node(&format!("/parent/{child}"), "raisin:Page"),
                no_validation(),
            )
            .await?;
    }
    storage
        .branches()
        .create_branch(
            TENANT,
            REPO,
            "feature",
            "test-user",
            None,
            Some("main".to_string()),
            false,
            false,
        )
        .await?;
    let feature = || StorageScope::new(TENANT, REPO, "feature", WORKSPACE);
    nodes
        .move_child_after(feature(), "/parent", "a", "c", None, Some("test-user"))
        .await?;

    // Call through the trait explicitly (UFCS), so this compiles against the
    // `BranchRepository` surface rather than the inherent impl.
    let diff = raisin_storage::BranchRepository::diff_branches(
        storage.branches(),
        TENANT,
        REPO,
        "feature",
        "main",
    )
    .await?;

    assert!(
        diff.modified.iter().any(|n| n.operation == "reordered"),
        "trait diff must list the reordered child, got modified={:?}",
        diff.modified
    );
    assert!(
        diff.deleted.is_empty(),
        "no deletions expected, got {:?}",
        diff.deleted
    );

    // The trait path and the concrete impl must agree.
    let impl_diff = storage
        .branches_impl()
        .diff_branches(TENANT, REPO, "feature", "main")
        .await?;
    assert_eq!(diff.common_ancestor, impl_diff.common_ancestor);
    assert_eq!(diff.added.len(), impl_diff.added.len());
    assert_eq!(diff.modified.len(), impl_diff.modified.len());
    assert_eq!(diff.deleted.len(), impl_diff.deleted.len());

    Ok(())
}

// ===========================================================================
// Cross-branch node-set copy (NodeRepository::copy_nodes_across_branches)
// ===========================================================================

/// Fork an EMPTY `publish` branch (no upstream), so cross-branch copies start
/// from a blank target.
async fn create_empty_branch(storage: &RocksDBStorage, name: &str) -> Result<()> {
    storage
        .branches()
        .create_branch(TENANT, REPO, name, "test-user", None, None, false, false)
        .await?;
    Ok(())
}

fn child_names(nodes: &[Node]) -> Vec<String> {
    nodes.iter().map(|n| n.name.clone()).collect()
}

/// First promotion onto an empty branch: the whole subtree is copied as
/// `Added`, node ids are PRESERVED, the sibling order matches the source, and
/// the target branch HEAD advances to the summary's single revision.
#[tokio::test]
async fn cross_branch_copy_new_preserves_ids_and_order() -> Result<()> {
    let t = TestStorage::new().await?;
    let storage = &t.storage;
    let nodes = storage.nodes();

    let main = || StorageScope::new(TENANT, REPO, "main", WORKSPACE);
    let publish = || StorageScope::new(TENANT, REPO, "publish", WORKSPACE);

    nodes
        .create(main(), make_node("/page", "raisin:Folder"), no_validation())
        .await?;
    for child in ["a", "b", "c"] {
        nodes
            .create(
                main(),
                make_node(&format!("/page/{child}"), "raisin:Page"),
                no_validation(),
            )
            .await?;
    }
    create_empty_branch(storage, "publish").await?;

    // Call through the trait explicitly (UFCS) — generic callers (function
    // runtime, transports) use exactly this surface.
    let summary = raisin_storage::NodeRepository::copy_nodes_across_branches(
        nodes,
        TENANT,
        REPO,
        "main",
        "publish",
        WORKSPACE,
        &["/page".to_string()],
        true,  // recursive
        false, // delete_missing
        None,  // source_revision — unpinned
        None,
    )
    .await?;

    assert_eq!(summary.copied, 4, "root + 3 children copied");
    assert_eq!(summary.deleted, 0);
    assert!(
        summary
            .changes
            .iter()
            .all(|c| c.operation == raisin_models::tree::ChangeOperation::Added),
        "first promotion must classify every node as Added: {:?}",
        summary.changes
    );

    // Ids preserved per path.
    for path in ["/page", "/page/a", "/page/b", "/page/c"] {
        let src = nodes.get_by_path(main(), path, None).await?.unwrap();
        let dst = nodes
            .get_by_path(publish(), path, None)
            .await?
            .unwrap_or_else(|| panic!("'{path}' must exist on publish"));
        assert_eq!(src.id, dst.id, "node id must be preserved for '{path}'");
        assert_eq!(src.node_type, dst.node_type);
        assert_eq!(src.properties, dst.properties);
    }

    // Sibling order replayed.
    let publish_order = child_names(
        &nodes
            .list_children(publish(), "/page", ListOptions::default())
            .await?,
    );
    assert_eq!(publish_order, vec!["a", "b", "c"]);

    // HEAD advanced to the single copy revision.
    let head = storage.branches().get_head(TENANT, REPO, "publish").await?;
    assert_eq!(
        head, summary.revision,
        "publish HEAD must be the copy revision"
    );

    Ok(())
}

/// Re-promotion after a source edit: the changed node is classified as
/// `Modified`, the new property value is visible on the target, an old-value
/// property query no longer matches, and `updated_at` is stamped while
/// `created_at`/`created_by` survive.
#[tokio::test]
async fn cross_branch_copy_update_replaces_old_values() -> Result<()> {
    use raisin_models::nodes::properties::PropertyValue;

    let t = TestStorage::new().await?;
    let storage = &t.storage;
    let nodes = storage.nodes();

    let main = || StorageScope::new(TENANT, REPO, "main", WORKSPACE);
    let publish = || StorageScope::new(TENANT, REPO, "publish", WORKSPACE);

    let mut node = make_node("/doc", "raisin:Page");
    node.properties.insert(
        "status".to_string(),
        PropertyValue::String("draft".to_string()),
    );
    nodes.create(main(), node, no_validation()).await?;
    create_empty_branch(storage, "publish").await?;

    let roots = vec!["/doc".to_string()];
    nodes
        .copy_nodes_across_branches(
            TENANT, REPO, "main", "publish", WORKSPACE, &roots, true, false, None, None,
        )
        .await?;

    let first = nodes.get_by_path(publish(), "/doc", None).await?.unwrap();

    // Edit on main, then promote again.
    nodes
        .update_property_by_path(
            main(),
            "/doc",
            "status",
            PropertyValue::String("live".to_string()),
        )
        .await?;
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    let summary = nodes
        .copy_nodes_across_branches(
            TENANT, REPO, "main", "publish", WORKSPACE, &roots, true, false, None, None,
        )
        .await?;

    assert_eq!(summary.copied, 1);
    assert_eq!(
        summary.changes[0].operation,
        raisin_models::tree::ChangeOperation::Modified,
        "re-promotion of an existing node must classify as Modified"
    );

    let second = nodes.get_by_path(publish(), "/doc", None).await?.unwrap();
    assert_eq!(second.id, first.id, "id stable across promotions");
    assert_eq!(
        second.properties.get("status"),
        Some(&PropertyValue::String("live".to_string()))
    );
    assert_eq!(
        second.created_at, first.created_at,
        "creation time preserved"
    );
    assert_eq!(second.created_by, first.created_by, "creator preserved");
    assert!(
        second.updated_at > first.updated_at,
        "updated_at must be stamped on the modified target node"
    );

    // Old-value property query must NOT match on the target branch anymore.
    let stale = nodes
        .find_by_property(
            publish(),
            "status",
            &PropertyValue::String("draft".to_string()),
        )
        .await?;
    assert!(
        stale.is_empty(),
        "old property value must not be queryable on the target: {:?}",
        stale.iter().map(|n| &n.path).collect::<Vec<_>>()
    );
    let live = nodes
        .find_by_property(
            publish(),
            "status",
            &PropertyValue::String("live".to_string()),
        )
        .await?;
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].path, "/doc");

    Ok(())
}

/// `delete_missing` prunes target nodes whose ids vanished from the source
/// set — the pruned node disappears from path lookups AND from the parent's
/// child listing (ordered index tombstoned).
#[tokio::test]
async fn cross_branch_copy_delete_missing_prunes() -> Result<()> {
    let t = TestStorage::new().await?;
    let storage = &t.storage;
    let nodes = storage.nodes();

    let main = || StorageScope::new(TENANT, REPO, "main", WORKSPACE);
    let publish = || StorageScope::new(TENANT, REPO, "publish", WORKSPACE);

    nodes
        .create(main(), make_node("/page", "raisin:Folder"), no_validation())
        .await?;
    for child in ["a", "b", "c"] {
        nodes
            .create(
                main(),
                make_node(&format!("/page/{child}"), "raisin:Page"),
                no_validation(),
            )
            .await?;
    }
    create_empty_branch(storage, "publish").await?;

    let roots = vec!["/page".to_string()];
    nodes
        .copy_nodes_across_branches(
            TENANT, REPO, "main", "publish", WORKSPACE, &roots, true, false, None, None,
        )
        .await?;

    // Remove /page/b on main, promote with pruning.
    nodes
        .delete_by_path(main(), "/page/b", DeleteNodeOptions::default())
        .await?;
    let summary = nodes
        .copy_nodes_across_branches(
            TENANT, REPO, "main", "publish", WORKSPACE, &roots, true, true, None, None,
        )
        .await?;

    assert_eq!(summary.copied, 3, "root + a + c re-copied");
    assert_eq!(summary.deleted, 1, "b pruned");
    assert!(
        summary
            .changes
            .iter()
            .any(|c| c.path == "/page/b"
                && c.operation == raisin_models::tree::ChangeOperation::Deleted),
        "changes must record the pruned node: {:?}",
        summary.changes
    );

    assert!(
        nodes
            .get_by_path(publish(), "/page/b", None)
            .await?
            .is_none(),
        "pruned node must be gone from the target branch"
    );
    let publish_order = child_names(
        &nodes
            .list_children(publish(), "/page", ListOptions::default())
            .await?,
    );
    assert_eq!(publish_order, vec!["a", "c"], "child listing pruned too");

    // Source branch untouched (well, /page/b was deleted there by the test,
    // but a and c still exist on main).
    assert!(nodes.get_by_path(main(), "/page/a", None).await?.is_some());

    Ok(())
}

/// A sibling reorder on the source is replayed on re-promotion: the stale
/// ordered-children slot is tombstoned, so the target order matches the
/// source instead of duplicating the moved child.
#[tokio::test]
async fn cross_branch_copy_replays_source_reorder() -> Result<()> {
    let t = TestStorage::new().await?;
    let storage = &t.storage;
    let nodes = storage.nodes();

    let main = || StorageScope::new(TENANT, REPO, "main", WORKSPACE);
    let publish = || StorageScope::new(TENANT, REPO, "publish", WORKSPACE);

    nodes
        .create(main(), make_node("/page", "raisin:Folder"), no_validation())
        .await?;
    for child in ["a", "b", "c"] {
        nodes
            .create(
                main(),
                make_node(&format!("/page/{child}"), "raisin:Page"),
                no_validation(),
            )
            .await?;
    }
    create_empty_branch(storage, "publish").await?;

    let roots = vec!["/page".to_string()];
    nodes
        .copy_nodes_across_branches(
            TENANT, REPO, "main", "publish", WORKSPACE, &roots, true, false, None, None,
        )
        .await?;

    // Reorder on main to c, a, b.
    nodes
        .move_child_after(main(), "/page", "a", "c", None, Some("test-user"))
        .await?;
    nodes
        .move_child_after(main(), "/page", "b", "a", None, Some("test-user"))
        .await?;
    let main_order = child_names(
        &nodes
            .list_children(main(), "/page", ListOptions::default())
            .await?,
    );
    assert_eq!(main_order, vec!["c", "a", "b"]);

    nodes
        .copy_nodes_across_branches(
            TENANT, REPO, "main", "publish", WORKSPACE, &roots, true, false, None, None,
        )
        .await?;

    let publish_children = nodes
        .list_children(publish(), "/page", ListOptions::default())
        .await?;
    assert_eq!(
        publish_children.len(),
        3,
        "no duplicated children after reorder replay: {:?}",
        child_names(&publish_children)
    );
    assert_eq!(
        child_names(&publish_children),
        vec!["c", "a", "b"],
        "target order must match the source after re-promotion"
    );

    Ok(())
}

/// Reference indexes must be fully written on the target branch: a reverse
/// reference lookup (the storage primitive behind referencing-node queries)
/// finds the promoted referrer on the target.
#[tokio::test]
async fn cross_branch_copy_reference_index_queryable_on_target() -> Result<()> {
    use raisin_models::nodes::properties::{PropertyValue, RaisinReference};
    use raisin_storage::ReferenceIndexRepository;

    let t = TestStorage::new().await?;
    let storage = &t.storage;
    let nodes = storage.nodes();

    let main = || StorageScope::new(TENANT, REPO, "main", WORKSPACE);
    let publish = || StorageScope::new(TENANT, REPO, "publish", WORKSPACE);

    let target_node = make_node("/target", "raisin:Page");
    let target_id = target_node.id.clone();
    nodes.create(main(), target_node, no_validation()).await?;

    let mut referrer = make_node("/referrer", "raisin:Page");
    let referrer_id = referrer.id.clone();
    referrer.properties.insert(
        "linked".to_string(),
        PropertyValue::Reference(RaisinReference {
            id: target_id.clone(),
            workspace: WORKSPACE.to_string(),
            path: "/target".to_string(),
        }),
    );
    nodes.create(main(), referrer, no_validation()).await?;

    create_empty_branch(storage, "publish").await?;
    nodes
        .copy_nodes_across_branches(
            TENANT,
            REPO,
            "main",
            "publish",
            WORKSPACE,
            &["/target".to_string(), "/referrer".to_string()],
            true,
            false,
            None,
            None,
        )
        .await?;

    // Reverse reference lookup on the TARGET branch finds the referrer.
    let referrers = storage
        .reference_index()
        .find_referencing_nodes(publish(), WORKSPACE, &target_id, false)
        .await?;
    assert!(
        referrers
            .iter()
            .any(|(source_id, prop)| source_id == &referrer_id && prop == "linked"),
        "reverse reference index must resolve on the target branch: {:?}",
        referrers
    );

    // And the forward reference deserializes intact on the target.
    let promoted = nodes
        .get_by_path(publish(), "/referrer", None)
        .await?
        .unwrap();
    match promoted.properties.get("linked") {
        Some(PropertyValue::Reference(r)) => assert_eq!(r.id, target_id),
        other => panic!(
            "expected a reference property on the target, got {:?}",
            other
        ),
    }

    Ok(())
}

/// A promotion must be visible to change tracking: the copy revision carries
/// per-node metadata, so a branch diff of the target against the source lists
/// the promoted nodes.
#[tokio::test]
async fn cross_branch_copy_visible_in_branch_diff() -> Result<()> {
    let t = TestStorage::new().await?;
    let storage = &t.storage;
    let nodes = storage.nodes();

    let main = || StorageScope::new(TENANT, REPO, "main", WORKSPACE);

    nodes
        .create(main(), make_node("/page", "raisin:Page"), no_validation())
        .await?;
    create_empty_branch(storage, "publish").await?;

    let summary = nodes
        .copy_nodes_across_branches(
            TENANT,
            REPO,
            "main",
            "publish",
            WORKSPACE,
            &["/page".to_string()],
            true,
            false,
            None,
            None,
        )
        .await?;

    let diff = storage
        .branches_impl()
        .diff_branches(TENANT, REPO, "publish", "main")
        .await?;
    let copied_ids: Vec<&str> = summary.changes.iter().map(|c| c.node_id.as_str()).collect();
    assert!(
        diff.added
            .iter()
            .any(|n| copied_ids.contains(&n.node_id.as_str())),
        "the promoted node must appear in the target-vs-source diff: added={:?}",
        diff.added
    );

    Ok(())
}

/// Failure modes: a root whose parent path is missing on the target branch
/// must fail validation WITHOUT writing anything, and a protected target
/// branch is rejected outright.
#[tokio::test]
async fn cross_branch_copy_validation_failures() -> Result<()> {
    let t = TestStorage::new().await?;
    let storage = &t.storage;
    let nodes = storage.nodes();

    let main = || StorageScope::new(TENANT, REPO, "main", WORKSPACE);
    let publish = || StorageScope::new(TENANT, REPO, "publish", WORKSPACE);

    nodes
        .create(
            main(),
            make_node("/parent", "raisin:Folder"),
            no_validation(),
        )
        .await?;
    nodes
        .create(
            main(),
            make_node("/parent/child", "raisin:Page"),
            no_validation(),
        )
        .await?;
    create_empty_branch(storage, "publish").await?;

    let head_before = storage.branches().get_head(TENANT, REPO, "publish").await?;

    // 1) Missing target parent -> validation error, nothing written.
    let err = nodes
        .copy_nodes_across_branches(
            TENANT,
            REPO,
            "main",
            "publish",
            WORKSPACE,
            &["/parent/child".to_string()],
            false,
            false,
            None,
            None,
        )
        .await
        .expect_err("copying under a missing target parent must fail");
    assert!(
        matches!(err, raisin_error::Error::Validation(_)),
        "expected a validation error, got {err:?}"
    );
    assert!(
        nodes
            .get_by_path(publish(), "/parent/child", None)
            .await?
            .is_none(),
        "no dangling subtree may be written on failure"
    );
    let head_after = storage.branches().get_head(TENANT, REPO, "publish").await?;
    assert_eq!(
        head_before, head_after,
        "target HEAD must not move on failure"
    );

    // 2) Protected target branch -> forbidden.
    storage
        .branches()
        .create_branch(TENANT, REPO, "locked", "test-user", None, None, true, false)
        .await?;
    let err = nodes
        .copy_nodes_across_branches(
            TENANT,
            REPO,
            "main",
            "locked",
            WORKSPACE,
            &["/parent".to_string()],
            true,
            false,
            None,
            None,
        )
        .await
        .expect_err("copying onto a protected branch must fail");
    assert!(
        matches!(err, raisin_error::Error::Forbidden(_)),
        "expected a forbidden error, got {err:?}"
    );

    // 3) Missing source root -> not found.
    let err = nodes
        .copy_nodes_across_branches(
            TENANT,
            REPO,
            "main",
            "publish",
            WORKSPACE,
            &["/does-not-exist".to_string()],
            true,
            false,
            None,
            None,
        )
        .await
        .expect_err("a missing source root must fail");
    assert!(matches!(err, raisin_error::Error::NotFound(_)));

    Ok(())
}

/// REGRESSION: a node carrying a translation overlay must survive promotion.
///
/// `collect_translations_for_copy` located the key's HLC revision by walking
/// FORWARD from the locale and taking 8 bytes. An HLC is 16 (8 timestamp + 8
/// counter) and its descending encoding is a bitwise NOT, so it readily
/// contains interior NULL bytes — the slice was half an HLC and
/// `decode_descending` failed unconditionally.
///
/// The blast radius was the whole promotion, not one overlay: the error
/// propagated out of `copy_nodes_across_branches`, so a single translated node
/// anywhere in the copied set aborted the entire publish with
/// "Failed to decode translation revision". A site that had ever been
/// translated could not publish at all.
#[tokio::test]
async fn cross_branch_copy_carries_translation_overlay() -> Result<()> {
    use raisin_hlc::HLC;
    use raisin_models::nodes::properties::PropertyValue;
    use raisin_models::translations::{JsonPointer, LocaleCode, LocaleOverlay, TranslationMeta};
    use raisin_storage::TranslationRepository;

    let t = TestStorage::new().await?;
    let storage = &t.storage;
    let nodes = storage.nodes();
    let main = || StorageScope::new(TENANT, REPO, "main", WORKSPACE);

    let mut node = make_node("/doc", "raisin:Page");
    node.properties.insert(
        "title".to_string(),
        PropertyValue::String("Hello world".to_string()),
    );
    let node_id = node.id.clone();
    nodes.create(main(), node, no_validation()).await?;

    let locale = LocaleCode::parse("de").expect("valid locale");
    let mut overlay_map = HashMap::new();
    overlay_map.insert(
        JsonPointer::new("/title"),
        PropertyValue::String("Hallo Welt".to_string()),
    );
    let overlay = LocaleOverlay::properties(overlay_map);
    let meta = TranslationMeta::system(locale.clone(), HLC::new(11, 0), "init".to_string());
    storage
        .translations()
        .store_translation(
            TENANT, REPO, "main", WORKSPACE, &node_id, &locale, &overlay, &meta,
        )
        .await?;

    create_empty_branch(storage, "publish").await?;

    // Before the fix this returned Err("Failed to decode translation revision").
    let summary = raisin_storage::NodeRepository::copy_nodes_across_branches(
        nodes,
        TENANT,
        REPO,
        "main",
        "publish",
        WORKSPACE,
        &["/doc".to_string()],
        true,
        false,
        None,
        None,
    )
    .await?;
    assert_eq!(summary.copied, 1, "the translated node must be copied");

    // The overlay must land on the TARGET branch, under the SAME node id —
    // ids are preserved by promotion, and overlays are addressed by node id.
    let copied = storage
        .translations()
        // "at or before this revision" — a far-future stamp asks for the latest,
        // since promotion writes the overlay under a fresh target-branch revision.
        .get_translation(
            TENANT,
            REPO,
            "publish",
            WORKSPACE,
            &node_id,
            &locale,
            &HLC::new(u64::MAX, u64::MAX),
        )
        .await?
        .expect("the de overlay must exist on the publish branch");

    match copied {
        LocaleOverlay::Properties { data } => assert_eq!(
            data.get(&JsonPointer::new("/title")),
            Some(&PropertyValue::String("Hallo Welt".to_string())),
            "the translated value must survive the copy"
        ),
        other => panic!("expected a properties overlay, got {other:?}"),
    }

    Ok(())
}

/// A pinned promotion copies the source AS OF the pin, not as of now.
///
/// Without a pin, a promotion of a large set reads each node when its turn
/// comes, so a writer working in parallel lands partly inside the result — a
/// torn snapshot split at whatever boundary the batching happened to use. This
/// asserts the property that makes a promotion reviewable: what was decided is
/// what gets copied, however long the copy takes and whoever else is typing.
#[tokio::test]
async fn cross_branch_copy_pins_the_source_revision() -> Result<()> {
    use raisin_models::nodes::properties::PropertyValue;

    let t = TestStorage::new().await?;
    let storage = &t.storage;
    let nodes = storage.nodes();
    let main = || StorageScope::new(TENANT, REPO, "main", WORKSPACE);

    let mut node = make_node("/doc", "raisin:Page");
    node.properties
        .insert("title".to_string(), PropertyValue::String("v1".to_string()));
    let node_id = node.id.clone();
    nodes.create(main(), node, no_validation()).await?;

    // The revision an operator would capture when pressing the button.
    let pin = storage
        .branches()
        .get_branch(TENANT, REPO, "main")
        .await?
        .map(|b| b.head)
        .expect("main must have a head");

    // …then someone keeps working while the promotion is still being decided.
    let mut edited = nodes
        .get(main(), &node_id, None)
        .await?
        .expect("node must exist");
    edited
        .properties
        .insert("title".to_string(), PropertyValue::String("v2".to_string()));
    nodes
        .update(
            main(),
            edited,
            raisin_storage::UpdateNodeOptions {
                validate_schema: false,
                allow_type_change: true,
                operation_meta: None,
            },
        )
        .await?;

    create_empty_branch(storage, "publish").await?;

    raisin_storage::NodeRepository::copy_nodes_across_branches(
        nodes,
        TENANT,
        REPO,
        "main",
        "publish",
        WORKSPACE,
        &["/doc".to_string()],
        true,
        false,
        Some(&pin),
        None,
    )
    .await?;

    let live = nodes
        .get(
            StorageScope::new(TENANT, REPO, "publish", WORKSPACE),
            &node_id,
            None,
        )
        .await?
        .expect("the node must be on publish");
    assert_eq!(
        live.properties.get("title"),
        Some(&PropertyValue::String("v1".to_string())),
        "a pinned promotion must copy the pinned state, not the later edit"
    );

    // And unpinned still means "now", so the pin is doing the work.
    raisin_storage::NodeRepository::copy_nodes_across_branches(
        nodes,
        TENANT,
        REPO,
        "main",
        "publish",
        WORKSPACE,
        &["/doc".to_string()],
        true,
        false,
        None,
        None,
    )
    .await?;
    let now = nodes
        .get(
            StorageScope::new(TENANT, REPO, "publish", WORKSPACE),
            &node_id,
            None,
        )
        .await?
        .expect("still there");
    assert_eq!(
        now.properties.get("title"),
        Some(&PropertyValue::String("v2".to_string())),
        "an unpinned promotion must copy the current state"
    );

    Ok(())
}

/// A node RE-CREATED on the source with a fresh id must REPLACE the previous
/// occupant of that path on the target, not shadow it.
///
/// PATH_INDEX carries the node id in its value, not its key, so before the fix
/// the second promotion overwrote the mapping and stranded the first node: its
/// blob and indexes stayed live, visible to a table scan and to nothing else.
/// The damage compounds, because `CHILD_OF` resolves a parent by path and then
/// walks ORDERED_CHILDREN under that ONE id — children filed under a superseded
/// generation simply vanish, with no error anywhere.
///
/// This is the storage-level reproduction of a live incident: a publish branch
/// held 88 nodes across 34 paths after three `deploy --install` regenerations,
/// and an event page rendered an empty programme because its tracks hung off a
/// parent id that `CHILD_OF` no longer resolved to.
#[tokio::test]
async fn cross_branch_copy_replaces_a_different_id_at_the_same_path() -> Result<()> {
    let t = TestStorage::new().await?;
    let storage = &t.storage;
    let nodes = storage.nodes();

    let main = || StorageScope::new(TENANT, REPO, "main", WORKSPACE);
    let publish = || StorageScope::new(TENANT, REPO, "publish", WORKSPACE);
    let roots = vec!["/page".to_string()];

    nodes
        .create(main(), make_node("/page", "raisin:Folder"), no_validation())
        .await?;
    nodes
        .create(main(), make_node("/page/a", "raisin:Page"), no_validation())
        .await?;
    create_empty_branch(storage, "publish").await?;
    nodes
        .copy_nodes_across_branches(
            TENANT, REPO, "main", "publish", WORKSPACE, &roots, true, false, None, None,
        )
        .await?;

    let first_root_id = nodes
        .get_by_path(publish(), "/page", None)
        .await?
        .unwrap()
        .id;
    let first_child_id = nodes
        .get_by_path(publish(), "/page/a", None)
        .await?
        .unwrap()
        .id;

    // Re-create the subtree on main under FRESH ids — what a package install
    // does. `publish` still holds the previous generation.
    // Deepest-first: delete_by_path is not a cascade.
    nodes
        .delete_by_path(main(), "/page/a", DeleteNodeOptions::default())
        .await?;
    nodes
        .delete_by_path(main(), "/page", DeleteNodeOptions::default())
        .await?;
    nodes
        .create(main(), make_node("/page", "raisin:Folder"), no_validation())
        .await?;
    nodes
        .create(main(), make_node("/page/a", "raisin:Page"), no_validation())
        .await?;
    let second_root_id = nodes.get_by_path(main(), "/page", None).await?.unwrap().id;
    let second_child_id = nodes
        .get_by_path(main(), "/page/a", None)
        .await?
        .unwrap()
        .id;
    assert_ne!(
        first_root_id, second_root_id,
        "the re-created node must carry a new id, or this test proves nothing"
    );

    let summary = nodes
        .copy_nodes_across_branches(
            TENANT, REPO, "main", "publish", WORKSPACE, &roots, true, false, None, None,
        )
        .await?;

    // The displaced generation is reported, not silently dropped.
    assert!(
        summary.changes.iter().any(|c| c.node_id == first_root_id
            && c.operation == raisin_models::tree::ChangeOperation::Deleted),
        "the displaced node must be reported as Deleted: {:?}",
        summary.changes
    );

    // The path resolves to the new generation...
    let live_root = nodes.get_by_path(publish(), "/page", None).await?.unwrap();
    assert_eq!(live_root.id, second_root_id, "path must own the new node");

    // ...and the old generation is gone, not merely unreachable by path.
    for stale in [&first_root_id, &first_child_id] {
        assert!(
            nodes.get(publish(), stale, None).await?.is_none(),
            "displaced node '{stale}' must be tombstoned, not left as an orphan"
        );
    }

    // The relationship CHILD_OF depends on: children resolve under the id the
    // path now points at. This is the assertion that fails loudest in prod.
    let children = nodes
        .list_children(publish(), "/page", ListOptions::default())
        .await?;
    assert_eq!(
        child_names(&children),
        vec!["a"],
        "the new generation's child must be reachable from the new parent"
    );
    assert_eq!(children[0].id, second_child_id);

    Ok(())
}

/// Does a cross-branch promotion carry the node's OUTGOING RELATIONS, and
/// does re-promoting after an edge was removed on main remove it on the target?
///
/// Studio's publish pipeline replays edges by hand (`replay-relations`) on the
/// assumption that a node-by-node copy does not carry them. This pins down
/// what the primitive actually does so that assumption can be retired or kept
/// on evidence.
#[tokio::test]
async fn cross_branch_copy_carries_relations_and_their_removal() -> Result<()> {
    use raisin_models::nodes::RelationRef;
    use raisin_storage::RelationRepository;

    let t = TestStorage::new().await?;
    let storage = &t.storage;
    let nodes = storage.nodes();
    let main = || StorageScope::new(TENANT, REPO, "main", WORKSPACE);
    let publish = || StorageScope::new(TENANT, REPO, "publish", WORKSPACE);

    let (a, b, c) = (
        make_node("/a", "raisin:Page"),
        make_node("/b", "raisin:Page"),
        make_node("/c", "raisin:Page"),
    );
    for n in [&a, &b, &c] {
        nodes.create(main(), n.clone(), no_validation()).await?;
    }
    let rel = |target: &Node, ty: &str| RelationRef {
        target: target.id.clone(),
        workspace: WORKSPACE.to_string(),
        target_node_type: "raisin:Page".to_string(),
        relation_type: ty.to_string(),
        weight: None,
    };
    storage
        .relations()
        .add_relation(main(), &a.id, "raisin:Page", rel(&b, "links_to"))
        .await?;
    storage
        .relations()
        .add_relation(main(), &a.id, "raisin:Page", rel(&c, "links_to"))
        .await?;
    create_empty_branch(storage, "publish").await?;

    let roots = ["/a".to_string(), "/b".to_string(), "/c".to_string()];
    raisin_storage::NodeRepository::copy_nodes_across_branches(
        nodes, TENANT, REPO, "main", "publish", WORKSPACE, &roots, false, false, None, None,
    )
    .await?;

    let mut live: Vec<String> = storage
        .relations()
        .get_outgoing_relations(publish(), &a.id, None)
        .await?
        .into_iter()
        .map(|r| r.target)
        .collect();
    live.sort();
    let mut want = vec![b.id.clone(), c.id.clone()];
    want.sort();
    assert_eq!(live, want, "promotion must carry a's outgoing edges");

    // Remove a->c on main, re-promote a, expect the edge gone on publish.
    storage
        .relations()
        .remove_relation(main(), &a.id, WORKSPACE, &c.id, Some("links_to"))
        .await?;
    raisin_storage::NodeRepository::copy_nodes_across_branches(
        nodes,
        TENANT,
        REPO,
        "main",
        "publish",
        WORKSPACE,
        &["/a".to_string()],
        false,
        false,
        None,
        None,
    )
    .await?;
    let live: Vec<String> = storage
        .relations()
        .get_outgoing_relations(publish(), &a.id, None)
        .await?
        .into_iter()
        .map(|r| r.target)
        .collect();
    assert_eq!(
        live,
        vec![b.id.clone()],
        "re-promotion must drop the edge removed on main"
    );
    Ok(())
}
