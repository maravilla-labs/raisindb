//! A deleted person never comes back through a package install: a user home
//! whose key is retired (`crate::retired_keys`) is skipped with its subtree,
//! reported by key, unless an administrator's explicit override is on.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::jobs::{JobId, JobRegistry};
use raisin_storage::scope::StorageScope;
use raisin_storage::{
    BranchRepository, DeleteNodeOptions, NodeRepository, RepositoryManagementRepository, Storage,
};
use tempfile::TempDir;

use super::content_types::{ContentEntry, InstallStats};
use super::handler::PackageInstallHandler;
use super::types::InstallMode;
use crate::RocksDBStorage;

const TENANT: &str = "default";
const BRANCH: &str = "main";
const AC: &str = "raisin:access_control";
const KEY: &str = "u_0123456789abcdefABCDEF";

async fn repo(storage: &Arc<RocksDBStorage>, repo: &str) {
    storage
        .repository_management()
        .create_repository(TENANT, repo, raisin_context::RepositoryConfig::default())
        .await
        .unwrap();
    storage
        .branches()
        .create_branch(TENANT, repo, BRANCH, "test", None, None, false, false)
        .await
        .unwrap();
    raisin_core::nodetype_init::init_repository_nodetypes(storage.clone(), TENANT, repo, BRANCH)
        .await
        .unwrap();
    raisin_core::workspace_init::init_repository_workspaces(storage.clone(), TENANT, repo)
        .await
        .unwrap();
}

fn entry(path: &str, node_type: &str, props: &[(&str, &str)]) -> ContentEntry {
    let mut properties = HashMap::new();
    for (k, v) in props {
        properties.insert(k.to_string(), PropertyValue::String(v.to_string()));
    }
    ContentEntry::NodeDef {
        workspace: AC.to_string(),
        yaml_path: format!("content/_raisin__access_control{path}/.node.yaml"),
        node: Box::new(Node {
            id: format!("id-{}", path.replace('/', "-")),
            path: path.to_string(),
            name: path.rsplit('/').next().unwrap().to_string(),
            node_type: node_type.to_string(),
            workspace: Some(AC.to_string()),
            properties,
            ..Default::default()
        }),
        legacy_path: None,
    }
}

fn package() -> Vec<ContentEntry> {
    let home = format!("/users/internal/{KEY}");
    vec![
        entry(
            &home,
            "raisin:User",
            &[("email", "gone@example.com"), ("display_name", "Gone")],
        ),
        entry(&format!("{home}/notes"), "raisin:Folder", &[]),
    ]
}

fn handler(storage: &Arc<RocksDBStorage>) -> PackageInstallHandler<RocksDBStorage> {
    let db = storage.db().clone();
    PackageInstallHandler::new(storage.clone(), Arc::new(JobRegistry::new())).with_retired_keys(
        Arc::new(move |t: &str, r: &str, k: &str| {
            crate::retired_keys::retired_and_gone(&db, t, r, k)
        }),
    )
}

async fn install(storage: &Arc<RocksDBStorage>, repo: &str, override_on: bool) -> InstallStats {
    let mut stats = InstallStats {
        allow_retired_user_keys: override_on,
        retired_keys_override_actor: override_on.then(|| "admin".to_string()),
        ..Default::default()
    };
    handler(storage)
        .install_sorted_entries(
            package(),
            &HashMap::new(),
            TENANT,
            repo,
            BRANCH,
            &JobId::new(),
            InstallMode::Sync,
            None,
            &HashMap::from([(AC.to_string(), "raisin:AclFolder".to_string())]),
            None,
            &mut stats,
        )
        .await
        .unwrap();
    assert!(
        stats.content_errors.is_empty(),
        "{:?}",
        stats.content_errors
    );
    stats
}

async fn home_exists(storage: &Arc<RocksDBStorage>, repo: &str) -> bool {
    storage
        .nodes()
        .get_by_path(
            StorageScope::new(TENANT, repo, BRANCH, AC),
            &format!("/users/internal/{KEY}"),
            None,
        )
        .await
        .unwrap()
        .is_some()
}

#[tokio::test]
async fn a_deleted_person_is_not_reinstalled_unless_an_admin_overrides() {
    let dir = TempDir::new().unwrap();
    let storage = Arc::new(RocksDBStorage::new(dir.path()).unwrap());
    repo(&storage, "r1").await;

    // Installed (a system import keeps the key), then deleted.
    let stats = install(&storage, "r1", false).await;
    assert!(stats.skipped_retired_keys.is_empty());
    assert!(home_exists(&storage, "r1").await);
    let scope = StorageScope::new(TENANT, "r1", BRANCH, AC);
    let home = storage
        .nodes()
        .get_by_path(scope, &format!("/users/internal/{KEY}"), None)
        .await
        .unwrap()
        .unwrap();
    storage
        .nodes()
        .delete(
            scope,
            &home.id,
            DeleteNodeOptions {
                cascade: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // The ledger has the key and its time, no personal data.
    let ledger = crate::retired_keys::list(storage.db(), TENANT).unwrap();
    assert_eq!(ledger.len(), 1, "{ledger:?}");
    assert_eq!(ledger[0].key, KEY);
    let json = serde_json::to_string(&ledger).unwrap();
    assert!(
        !json.contains("gone@example.com") && !json.contains("Gone"),
        "{json}"
    );

    // Reinstalling the package skips the home and its subtree, by key.
    let stats = install(&storage, "r1", false).await;
    assert_eq!(stats.skipped_retired_keys, vec![KEY.to_string()]);
    assert!(!home_exists(&storage, "r1").await);

    // Another (or a recreated) repository of the same tenant: the tenant
    // ledger still refuses it.
    repo(&storage, "r2").await;
    let stats = install(&storage, "r2", false).await;
    assert_eq!(stats.skipped_retired_keys, vec![KEY.to_string()]);
    assert!(!home_exists(&storage, "r2").await);

    // The explicit admin override restores it (deliberate recovery).
    let stats = install(&storage, "r2", true).await;
    assert!(stats.skipped_retired_keys.is_empty());
    assert!(home_exists(&storage, "r2").await);
}
