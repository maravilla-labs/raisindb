// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Regression tests for installing `raisin:Role` content from a package.
//!
//! Roles are the shape that exposed two install bugs, because `raisin:Role` is
//! the rare node type that carries a REQUIRED human-readable `name` property
//! AND marks both `role_id` and `name` `unique: true`:
//!
//! 1. `properties.name` used to decide the node's path, so
//!    `roles/author/.node.yaml` installed to `/roles/Author` while the CLI
//!    sync mapper pushed the same file to `/roles/author`. Two nodes, one
//!    `role_id`, unique-constraint failure.
//! 2. That failure propagated with `?` out of the batch loop, so one colliding
//!    role failed the whole install job and every other role vanished with it.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::jobs::{JobId, JobRegistry};
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::{RepositoryManagementRepository, Storage};
use tempfile::TempDir;

use super::content_types::{ContentEntry, InstallStats};
use super::handler::PackageInstallHandler;
use super::types::InstallMode;
use crate::RocksDBStorage;

const TENANT: &str = "default";
const REPO: &str = "testrepo";
const BRANCH: &str = "main";
const ACL_WS: &str = "raisin:access_control";

struct Env {
    _dir: TempDir,
    storage: Arc<RocksDBStorage>,
}

async fn setup() -> Env {
    let dir = TempDir::new().unwrap();
    let storage = Arc::new(RocksDBStorage::new(dir.path()).unwrap());

    storage
        .repository_management()
        .create_repository(TENANT, REPO, raisin_context::RepositoryConfig::default())
        .await
        .unwrap();
    use raisin_storage::BranchRepository;
    storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test", None, None, false, false)
        .await
        .unwrap();
    raisin_core::nodetype_init::init_repository_nodetypes(storage.clone(), TENANT, REPO, BRANCH)
        .await
        .unwrap();
    raisin_core::workspace_init::init_repository_workspaces(storage.clone(), TENANT, REPO)
        .await
        .unwrap();
    // Seeds the workspace's own folders (config/, users/, roles/, ...) and, in
    // doing so, the root nodes that by-path resolution needs.
    raisin_core::workspace_structure_init::create_workspace_initial_structure(
        storage.clone(),
        TENANT,
        REPO,
        ACL_WS,
    )
    .await
    .unwrap();
    Env { _dir: dir, storage }
}

fn handler(env: &Env) -> PackageInstallHandler<RocksDBStorage> {
    PackageInstallHandler::new(env.storage.clone(), Arc::new(JobRegistry::new()))
}

/// A `raisin:Role` content entry as the collector would produce it.
fn role_entry(node_path: &str, role_id: &str, display_name: &str) -> ContentEntry {
    role_entry_with_legacy(node_path, role_id, display_name, None)
}

fn role_entry_with_legacy(
    node_path: &str,
    role_id: &str,
    display_name: &str,
    legacy_path: Option<&str>,
) -> ContentEntry {
    let mut properties = HashMap::new();
    properties.insert(
        "role_id".to_string(),
        PropertyValue::String(role_id.to_string()),
    );
    properties.insert(
        "name".to_string(),
        PropertyValue::String(display_name.to_string()),
    );

    ContentEntry::NodeDef {
        workspace: ACL_WS.to_string(),
        yaml_path: format!("content/_raisin__access_control{node_path}/.node.yaml"),
        node: Box::new(Node {
            id: nanoid::nanoid!(),
            node_type: "raisin:Role".to_string(),
            name: node_path.rsplit('/').next().unwrap().to_string(),
            path: node_path.to_string(),
            workspace: Some(ACL_WS.to_string()),
            properties,
            ..Default::default()
        }),
        legacy_path: legacy_path.map(|p| p.to_string()),
    }
}

async fn install(
    env: &Env,
    entries: Vec<ContentEntry>,
    mode: InstallMode,
) -> (raisin_error::Result<()>, InstallStats) {
    let mut stats = InstallStats::default();
    let bundled = HashMap::new();
    let folder_types = HashMap::from([(ACL_WS.to_string(), "raisin:AclFolder".to_string())]);
    let result = handler(env)
        .install_sorted_entries(
            entries,
            &bundled,
            TENANT,
            REPO,
            BRANCH,
            &JobId::new(),
            mode,
            None,
            &folder_types,
            None,
            &mut stats,
        )
        .await;
    (result, stats)
}

/// Read a node back by path.
///
/// Uses the repository, not a bare transaction context: the transactional
/// by-path read wants a fully configured transaction (actor + auth), which is
/// what the installer builds and a read-only harness does not.
async fn node_at(env: &Env, path: &str) -> Option<Node> {
    use raisin_storage::{scope::StorageScope, NodeRepository};
    env.storage
        .nodes()
        .get_by_path(StorageScope::new(TENANT, REPO, BRANCH, ACL_WS), path, None)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_roles_display_name_does_not_decide_its_path() {
    let env = setup().await;

    let (result, stats) = install(
        &env,
        vec![
            role_entry("/roles/author", "author", "Author"),
            role_entry("/roles/editor", "editor", "Editor"),
            role_entry("/roles/viewer", "viewer", "Viewer"),
        ],
        InstallMode::Sync,
    )
    .await;

    assert!(result.is_ok(), "install failed: {:?}", result.err());
    assert!(
        stats.content_errors.is_empty(),
        "{:?}",
        stats.content_errors
    );

    for (path, role_id) in [
        ("/roles/author", "author"),
        ("/roles/editor", "editor"),
        ("/roles/viewer", "viewer"),
    ] {
        let node = node_at(&env, path)
            .await
            .unwrap_or_else(|| panic!("no role at {path}"));
        assert_eq!(
            node.properties.get("role_id"),
            Some(&PropertyValue::String(role_id.to_string()))
        );
    }

    // The capitalised display name must NOT have produced a node.
    assert!(node_at(&env, "/roles/Author").await.is_none());
}

#[tokio::test]
async fn reinstalling_updates_in_place_rather_than_duplicating() {
    let env = setup().await;

    let (r1, _) = install(
        &env,
        vec![role_entry("/roles/author", "author", "Author")],
        InstallMode::Sync,
    )
    .await;
    assert!(r1.is_ok());
    let first_id = node_at(&env, "/roles/author").await.unwrap().id;

    // Same role, changed DISPLAY name. Under the old rule this moved the node
    // to /roles/Wordsmith and then collided on the unique `role_id`.
    let (r2, stats) = install(
        &env,
        vec![role_entry("/roles/author", "author", "Wordsmith")],
        InstallMode::Sync,
    )
    .await;

    assert!(r2.is_ok(), "re-install failed: {:?}", r2.err());
    assert!(
        stats.content_errors.is_empty(),
        "{:?}",
        stats.content_errors
    );

    let again = node_at(&env, "/roles/author").await.unwrap();
    assert_eq!(again.id, first_id, "re-install must not mint a new node");
    assert_eq!(
        again.properties.get("name"),
        Some(&PropertyValue::String("Wordsmith".to_string())),
        "the display name should have been updated"
    );
    assert!(node_at(&env, "/roles/Wordsmith").await.is_none());
}

#[tokio::test]
async fn a_node_left_at_the_legacy_path_is_adopted_not_duplicated() {
    let env = setup().await;

    // Simulate what a pre-fix install left behind: the node sits at the path
    // its display name produced.
    let (seed, _) = install(
        &env,
        vec![role_entry("/roles/Author", "author", "Author")],
        InstallMode::Sync,
    )
    .await;
    assert!(seed.is_ok());
    let legacy_id = node_at(&env, "/roles/Author").await.unwrap().id;

    // Now install the same definition the way the fixed collector emits it.
    let (result, stats) = install(
        &env,
        vec![role_entry_with_legacy(
            "/roles/author",
            "author",
            "Author",
            Some("/roles/Author"),
        )],
        InstallMode::Sync,
    )
    .await;

    assert!(result.is_ok(), "install failed: {:?}", result.err());
    assert!(
        stats.content_errors.is_empty(),
        "{:?}",
        stats.content_errors
    );

    let adopted = node_at(&env, "/roles/author")
        .await
        .expect("role should now live at the path-derived location");
    assert_eq!(
        adopted.id, legacy_id,
        "the existing node must be MOVED, keeping its id -- a fresh id would \
         collide on the unique role_id and lose history"
    );
    assert!(
        node_at(&env, "/roles/Author").await.is_none(),
        "the legacy path must no longer resolve"
    );
}

/// Adoption at the WORKSPACE ROOT — the case that broke two live tenants.
///
/// `/roles/Author` -> `/roles/author` has a real parent node to move under.
/// A node that sits at the TOP LEVEL of a workspace does not: its parent is
/// `/`, which stores no node, so the adoption move was rejected with
/// `Target parent '/' not found` and one rejected entry failed the whole
/// package install. That is exactly the shape of the studio package's
/// `mcp:/Studio` node, whose display name and directory disagreed.
#[tokio::test]
async fn a_root_level_node_at_the_legacy_path_is_adopted_too() {
    let env = setup().await;

    // What a pre-fix install left at the top level of the workspace.
    let (seed, _) = install(
        &env,
        vec![role_entry("/Studio", "studio-mcp", "Studio")],
        InstallMode::Sync,
    )
    .await;
    assert!(seed.is_ok(), "seed failed: {:?}", seed.err());
    let legacy_id = node_at(&env, "/Studio").await.unwrap().id;

    // The same definition as the fixed collector emits it: path-derived name.
    let (result, stats) = install(
        &env,
        vec![role_entry_with_legacy(
            "/studio",
            "studio-mcp",
            "Studio",
            Some("/Studio"),
        )],
        InstallMode::Sync,
    )
    .await;

    assert!(result.is_ok(), "install failed: {:?}", result.err());
    assert!(
        stats.content_errors.is_empty(),
        "a root-level adoption must not be rejected: {:?}",
        stats.content_errors
    );

    let adopted = node_at(&env, "/studio")
        .await
        .expect("the node should now live at the path-derived location");
    assert_eq!(
        adopted.id, legacy_id,
        "the existing node must be MOVED, keeping its id"
    );
    assert!(
        node_at(&env, "/Studio").await.is_none(),
        "exactly one node must remain -- a twin beside it is the bug this guards"
    );
}

#[tokio::test]
async fn one_rejected_role_does_not_take_the_others_down() {
    let env = setup().await;

    // Seed a role that owns role_id "editor" at a path the package does not use.
    let (seed, _) = install(
        &env,
        vec![role_entry("/roles/incumbent", "editor", "Incumbent")],
        InstallMode::Sync,
    )
    .await;
    assert!(seed.is_ok());

    // The package declares five roles; one of them collides on the unique
    // `role_id`. The other four must still land.
    let (result, stats) = install(
        &env,
        vec![
            role_entry("/roles/alpha", "alpha", "Alpha"),
            role_entry("/roles/beta", "beta", "Beta"),
            role_entry("/roles/editor", "editor", "Editor"), // collides
            role_entry("/roles/delta", "delta", "Delta"),
            role_entry("/roles/epsilon", "epsilon", "Epsilon"),
        ],
        InstallMode::Sync,
    )
    .await;

    // The batch itself succeeds; the job fails later, on the collected errors.
    assert!(
        result.is_ok(),
        "a per-node rejection must not abort the batch: {:?}",
        result.err()
    );
    assert_eq!(
        stats.content_errors.len(),
        1,
        "expected exactly one rejection, got {:?}",
        stats.content_errors
    );
    assert!(
        stats.content_errors[0].contains("role_id") || stats.content_errors[0].contains("unique"),
        "the error should name the constraint: {:?}",
        stats.content_errors
    );

    for path in [
        "/roles/alpha",
        "/roles/beta",
        "/roles/delta",
        "/roles/epsilon",
    ] {
        assert!(
            node_at(&env, path).await.is_some(),
            "{path} should have installed despite the collision elsewhere"
        );
    }
}

/// Control: does a plain upsert + commit become visible by path in this
/// harness at all? If this fails, the harness is incomplete, not the installer.
#[tokio::test]
async fn control_plain_upsert_is_visible_by_path() {
    let env = setup().await;
    {
        let tx = env.storage.begin_context().await.unwrap();
        tx.set_tenant_repo(TENANT, REPO).unwrap();
        tx.set_branch(BRANCH).unwrap();
        tx.set_actor("test").unwrap();
        tx.set_message("seed").unwrap();
        tx.set_auth_context(raisin_models::auth::AuthContext::system())
            .unwrap();
        let mut properties = HashMap::new();
        properties.insert(
            "role_id".to_string(),
            PropertyValue::String("ctrl".to_string()),
        );
        properties.insert(
            "name".to_string(),
            PropertyValue::String("Ctrl".to_string()),
        );
        let node = Node {
            id: nanoid::nanoid!(),
            node_type: "raisin:Role".to_string(),
            name: "ctrl".to_string(),
            path: "/roles/ctrl".to_string(),
            workspace: Some(ACL_WS.to_string()),
            properties,
            ..Default::default()
        };
        tx.upsert_deep_node(ACL_WS, &node, "raisin:AclFolder")
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }
    assert!(node_at(&env, "/roles/ctrl").await.is_some());
}
