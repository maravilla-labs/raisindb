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
    ArchetypeRepository, BranchRepository, CommitMetadata, CreateNodeOptions, ListOptions,
    NodeRepository, RegistryRepository, RepositoryManagementRepository, Storage,
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
