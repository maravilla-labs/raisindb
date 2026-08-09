//! The virtual-mount registry: what it must contain, and what the fast scan
//! must therefore return.
//!
//! A child module of [`super::tests`] (declared with `#[path]` at the bottom of
//! `tests.rs`) so it can reuse that file's `Env`.
//!
//! The load-bearing test here is
//! [`the_fast_scan_returns_what_the_full_enumeration_would`]. The registry is an
//! optimisation whose only correctness obligation is that it does not LOSE a
//! repository — and losing one is silent, the mount simply stops syncing. So the
//! fast path is pinned against the enumeration it replaced, over a fixture with
//! mounts spread across several repositories and repositories with none.

use raisin_storage::{CreateNodeOptions, NodeRepository, StorageScope};

use super::*;
use crate::jobs::handlers::virtual_mount_sync as vmount;
use crate::vmount_registry;
use raisin_core::services::node_service::NodeService;

/// Register the tenant so `list_tenants` (which both the fast scan and the
/// legacy enumeration start from) can see it. `setup()` creates a repository
/// without one, which is fine for the sync tests but makes every scan in this
/// file return nothing.
async fn register_tenant(env: &Env) {
    use raisin_storage::RegistryRepository;
    env.storage
        .registry()
        .register_tenant(TENANT, HashMap::new())
        .await
        .unwrap();
}

/// A second (third, …) repository, set up the way `setup()` sets up the first.
async fn add_repo(env: &Env, repo: &str) {
    env.storage
        .repository_management()
        .create_repository(TENANT, repo, raisin_context::RepositoryConfig::default())
        .await
        .unwrap();
    use raisin_storage::BranchRepository;
    env.storage
        .branches()
        .create_branch(TENANT, repo, "main", "test", None, None, false, false)
        .await
        .unwrap();
    raisin_core::nodetype_init::init_repository_nodetypes(
        env.storage.clone(),
        TENANT,
        repo,
        "main",
    )
    .await
    .unwrap();
    use raisin_storage::{RepoScope, WorkspaceRepository};
    for ws in raisin_core::workspace_init::load_global_workspaces() {
        env.storage
            .workspaces()
            .put(RepoScope::new(TENANT, repo), ws)
            .await
            .unwrap();
    }
}

fn mount_node(mount_id: &str) -> Node {
    let mut mount = Node {
        id: mount_id.to_string(),
        node_type: "raisin:VirtualMount".to_string(),
        name: mount_id.to_string(),
        path: format!("/mounts/{mount_id}"),
        workspace: Some(vmount::SYSTEM_WORKSPACE.to_string()),
        ..Default::default()
    };
    for (k, v) in [
        ("title", "Mock"),
        ("integration_ref", "/integrations/mock"),
        ("target_workspace", TARGET_WS),
        ("mount_path", MOUNT_PATH),
        ("target_branch", "main"),
    ] {
        mount
            .properties
            .insert(k.to_string(), PropertyValue::String(v.to_string()));
    }
    mount
        .properties
        .insert("enabled".to_string(), PropertyValue::Boolean(true));
    mount
}

/// Write a mount through the TRANSACTION path (`put_node`/`add_node`).
async fn write_mount_tx(env: &Env, repo: &str, mount_id: &str) {
    let tx = env.storage.begin_context().await.unwrap();
    tx.set_tenant_repo(TENANT, repo).unwrap();
    tx.set_branch("main").unwrap();
    tx.set_actor("test").unwrap();
    tx.set_auth_context(AuthContext::system()).unwrap();
    tx.set_message("test").unwrap();
    tx.upsert_deep_node(
        vmount::SYSTEM_WORKSPACE,
        &mount_node(mount_id),
        "raisin:Folder",
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

fn entries(env: &Env) -> Vec<vmount_registry::RegistryEntry> {
    vmount_registry::list_entries(env.storage.db(), TENANT).unwrap()
}

fn registered_repos(env: &Env) -> Vec<String> {
    vmount_registry::list_repos_with_mounts(env.storage.db(), TENANT).unwrap()
}

/// The enumeration the fast path replaced: every tenant × every repo.
async fn legacy_scan_scope(env: &Env) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for tenant in crate::management::list_tenants(&env.storage).await.unwrap() {
        for repo in crate::management::list_repositories(&env.storage, &tenant)
            .await
            .unwrap()
        {
            out.push((tenant.clone(), repo));
        }
    }
    out
}

/// The `(tenant, repo)` pairs the legacy scan would have found mounts in — i.e.
/// the only pairs whose omission from the fast path could change behaviour.
async fn legacy_scope_with_mounts(env: &Env) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (tenant, repo) in legacy_scan_scope(env).await {
        let branch = vmount::check::config_branch(&env.storage, &tenant, &repo).await;
        let svc: NodeService<RocksDBStorage> = NodeService::new_with_context(
            env.storage.clone(),
            tenant.clone(),
            repo.clone(),
            branch,
            vmount::SYSTEM_WORKSPACE.to_string(),
        )
        .with_auth(AuthContext::system());
        if !svc
            .list_by_type("raisin:VirtualMount")
            .await
            .unwrap()
            .is_empty()
        {
            out.push((tenant, repo));
        }
    }
    out.sort();
    out
}

async fn fast_scan_scope(env: &Env) -> Vec<(String, String)> {
    let mut out = vmount::check::scan_scope(&env.storage, None, None)
        .await
        .unwrap();
    out.sort();
    out
}

// ---- write side ----

#[tokio::test(flavor = "multi_thread")]
async fn a_mount_write_registers_its_repo() {
    let env = setup().await;
    register_tenant(&env).await;
    assert!(registered_repos(&env).is_empty(), "nothing registered yet");

    write_mount_tx(&env, REPO, "m1").await;

    assert_eq!(registered_repos(&env), vec![REPO.to_string()]);
    let e = entries(&env);
    assert_eq!(e.len(), 1, "one entry per mount, got {e:?}");
    assert_eq!(e[0].branch, "main");
    assert_eq!(e[0].mount_id, "m1");
}

/// The repository API is a SECOND node writer. A registry entry written only on
/// the transaction path would leave every mount created through
/// `storage.nodes().create(...)` invisible to the scan — the mirrored-path drift
/// this whole design is arranged to prevent.
#[tokio::test(flavor = "multi_thread")]
async fn the_repository_write_path_registers_too() {
    let env = setup().await;
    register_tenant(&env).await;

    // Parent folder first: `create` validates the parent exists.
    let mut folder = Node {
        id: "mounts-folder".to_string(),
        node_type: "raisin:Folder".to_string(),
        name: "mounts".to_string(),
        path: "/mounts".to_string(),
        workspace: Some(vmount::SYSTEM_WORKSPACE.to_string()),
        ..Default::default()
    };
    folder.parent = Some("/".to_string());
    let scope = StorageScope::new(TENANT, REPO, "main", vmount::SYSTEM_WORKSPACE);
    env.storage
        .nodes()
        .create(
            scope,
            folder,
            CreateNodeOptions {
                validate_schema: false,
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    env.storage
        .nodes()
        .create(
            StorageScope::new(TENANT, REPO, "main", vmount::SYSTEM_WORKSPACE),
            mount_node("m-repo"),
            CreateNodeOptions {
                validate_schema: false,
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert_eq!(
        registered_repos(&env),
        vec![REPO.to_string()],
        "a mount created through the repository API must register"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_mount_removes_its_entry() {
    let env = setup().await;
    register_tenant(&env).await;
    write_mount_tx(&env, REPO, "m1").await;
    write_mount_tx(&env, REPO, "m2").await;
    assert_eq!(entries(&env).len(), 2);

    let tx = begin_on(&env, "main").await;
    tx.delete_node(vmount::SYSTEM_WORKSPACE, "m1")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let left = entries(&env);
    assert_eq!(
        left.len(),
        1,
        "only the deleted mount's entry goes, {left:?}"
    );
    assert_eq!(left[0].mount_id, "m2");
    assert_eq!(
        registered_repos(&env),
        vec![REPO.to_string()],
        "the repo must stay listed while a mount survives — a per-repo key \
         would have stranded m2 here"
    );

    let tx = begin_on(&env, "main").await;
    tx.delete_node(vmount::SYSTEM_WORKSPACE, "m2")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(
        registered_repos(&env).is_empty(),
        "the last mount going takes the repo out of the scan"
    );
}

// ---- the equivalence that matters ----

/// Regression: a REGISTRY key that is NOT a repository must not be reported as
/// one.
///
/// `list_repositories_for_tenant` seeks to `{tenant}\0repos\0` with
/// `prefix_iterator_cf` — which is bounded by the column family's prefix
/// EXTRACTOR, not by the seek key, so it keeps walking past the prefix — and
/// then accepted anything with `parts.len() >= 3` as a repository, reading
/// `parts[2]` as the id. Every registry namespace sorting after `repos` was
/// therefore reported as a (duplicate) repository.
///
/// This is not hypothetical and it predates this change: it is why the OLD
/// mount scan visited some repositories two and three times per 60-second tick.
/// The mount registry made it visible because `{tenant}\0vmounts\0{repo}\0…`
/// sorts after `{tenant}\0repos\0` and carries a real repo id in `parts[2]`,
/// so each mount minted another copy of its own repository.
///
/// Without the `starts_with` guard in `management::helpers` this returns
/// `["vm-test", "vm-test", "vm-test"]`.
#[tokio::test(flavor = "multi_thread")]
async fn a_mount_registry_entry_is_not_mistaken_for_a_repository() {
    let env = setup().await;
    register_tenant(&env).await;
    write_mount_tx(&env, REPO, "m1").await;
    write_mount_tx(&env, REPO, "m2").await;

    let repos = crate::management::list_repositories(&env.storage, TENANT)
        .await
        .unwrap();
    assert_eq!(
        repos,
        vec![REPO.to_string()],
        "one repository exists; the two mount entries must not be counted as repositories"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_fast_scan_returns_what_the_full_enumeration_would() {
    let env = setup().await;
    register_tenant(&env).await;
    add_repo(&env, "repo-b").await;
    add_repo(&env, "repo-c").await;
    add_repo(&env, "repo-empty").await;

    write_mount_tx(&env, REPO, "m1").await;
    write_mount_tx(&env, REPO, "m2").await; // two in one repo
    write_mount_tx(&env, "repo-b", "mb").await;
    write_mount_tx(&env, "repo-c", "mc").await;
    // repo-empty deliberately has none.

    let legacy = legacy_scope_with_mounts(&env).await;
    assert_eq!(
        legacy,
        vec![
            (TENANT.to_string(), "repo-b".to_string()),
            (TENANT.to_string(), "repo-c".to_string()),
            (TENANT.to_string(), REPO.to_string()),
        ],
        "fixture sanity"
    );
    assert_eq!(fast_scan_scope(&env).await, legacy);

    // …and it stays equivalent as mounts come and go.
    let tx = begin_on(&env, "main").await;
    tx.delete_node(vmount::SYSTEM_WORKSPACE, "m1")
        .await
        .unwrap();
    tx.delete_node(vmount::SYSTEM_WORKSPACE, "m2")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let legacy = legacy_scope_with_mounts(&env).await;
    assert!(!legacy.contains(&(TENANT.to_string(), REPO.to_string())));
    assert_eq!(fast_scan_scope(&env).await, legacy);
}

/// An explicit `repo_filter` must NOT be answered out of the registry: naming a
/// repository means "look here", and consulting the index would make the one
/// targeted invocation an operator has the one call that cannot see an
/// unregistered mount.
#[tokio::test(flavor = "multi_thread")]
async fn an_explicit_repo_filter_bypasses_the_registry() {
    let env = setup().await;
    register_tenant(&env).await;
    assert!(registered_repos(&env).is_empty());

    let scope = vmount::check::scan_scope(&env.storage, None, Some(REPO.to_string()))
        .await
        .unwrap();
    assert_eq!(scope, vec![(TENANT.to_string(), REPO.to_string())]);
}

// ---- self-heal ----

/// The drift alarm. A registry entry lost by any means leaves the mount syncing
/// never, with no error anywhere — so the reconcile has to find it and put it
/// back.
#[tokio::test(flavor = "multi_thread")]
async fn the_reconcile_repairs_a_deliberately_missing_entry() {
    let env = setup().await;
    register_tenant(&env).await;
    add_repo(&env, "repo-b").await;
    write_mount_tx(&env, REPO, "m1").await;
    write_mount_tx(&env, "repo-b", "mb").await;

    // Simulate the drift: rip one entry out behind the write path's back.
    let cf_registry = crate::cf_handle(env.storage.db(), crate::cf::REGISTRY).unwrap();
    env.storage
        .db()
        .delete_cf(
            cf_registry,
            vmount_registry::registry_key(TENANT, REPO, "main", "m1"),
        )
        .unwrap();

    assert_eq!(
        registered_repos(&env),
        vec!["repo-b".to_string()],
        "the fast scan has now lost the mount — this is the silent failure"
    );
    assert!(!fast_scan_scope(&env)
        .await
        .contains(&(TENANT.to_string(), REPO.to_string())));

    let report = vmount::registry_backfill::reconcile_registry(&env.storage, None, None)
        .await
        .unwrap();

    assert_eq!(
        report.repaired, 1,
        "the missing entry must be named: {report:?}"
    );
    assert_eq!(report.mounts_found, 2);
    assert_eq!(report.pruned, 0, "nothing was actually dead: {report:?}");
    assert_eq!(
        fast_scan_scope(&env).await,
        legacy_scope_with_mounts(&env).await,
        "after the repair the fast path agrees with the full enumeration again"
    );

    // A second pass over healthy state is a no-op.
    let report = vmount::registry_backfill::reconcile_registry(&env.storage, None, None)
        .await
        .unwrap();
    assert_eq!(report.repaired, 0);
    assert_eq!(report.pruned, 0);
}

/// The other half of the audit: an entry with no mount behind it is dropped, so
/// a fork's copy of a mount config node (which no scan ever reads — see
/// `check::config_branch`) cannot keep a repository in the fast scan forever.
#[tokio::test(flavor = "multi_thread")]
async fn the_reconcile_prunes_an_entry_with_no_mount_behind_it() {
    let env = setup().await;
    register_tenant(&env).await;
    write_mount_tx(&env, REPO, "m1").await;

    let cf_registry = crate::cf_handle(env.storage.db(), crate::cf::REGISTRY).unwrap();
    env.storage
        .db()
        .put_cf(
            cf_registry,
            vmount_registry::registry_key(TENANT, REPO, "fork-branch", "ghost"),
            b"1",
        )
        .unwrap();
    assert_eq!(entries(&env).len(), 2);

    let report = vmount::registry_backfill::reconcile_registry(&env.storage, None, None)
        .await
        .unwrap();
    assert_eq!(report.pruned, 1, "{report:?}");
    assert_eq!(report.repaired, 0, "{report:?}");

    let left = entries(&env);
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].mount_id, "m1");
}
