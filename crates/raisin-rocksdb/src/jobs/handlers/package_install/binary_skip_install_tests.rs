// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! A binary asset on a path the package's `.raisin-sync.yaml` marks `skip`
//! is never rewritten once it exists — the rule `.node.yaml` entries and
//! translation overlays already followed. Binaries used to share the `sync`
//! hash comparison, so a redeploy whose bytes differed from the stored
//! `content_hash` rebuilt the asset from scratch and wiped every property an
//! editor had set since (reported by a customer whose defaults were `skip`).

use std::collections::HashMap;
use std::sync::Arc;

use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::jobs::{JobId, JobRegistry};
use raisin_storage::scope::StorageScope;
use raisin_storage::{BranchRepository, NodeRepository, RepositoryManagementRepository, Storage};
use tempfile::TempDir;

use super::content_types::{ContentEntry, InstallStats};
use super::handler::PackageInstallHandler;
use super::types::{BinaryStorageCallback, InstallMode};
use crate::RocksDBStorage;

const TENANT: &str = "default";
const REPO: &str = "assets";
const BRANCH: &str = "main";
const WS: &str = "default";

async fn setup() -> (TempDir, Arc<RocksDBStorage>) {
    let dir = TempDir::new().unwrap();
    let storage = Arc::new(RocksDBStorage::new(dir.path()).unwrap());
    storage
        .repository_management()
        .create_repository(TENANT, REPO, raisin_context::RepositoryConfig::default())
        .await
        .unwrap();
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
    (dir, storage)
}

/// Stores nothing; hands back a plausible blob record.
fn fake_store() -> BinaryStorageCallback {
    Arc::new(|data, mime, _ext, name, _tenant| {
        Box::pin(async move {
            let now = chrono::Utc::now();
            Ok(raisin_binary::StoredObject {
                key: nanoid::nanoid!(),
                url: "/blob".to_string(),
                name,
                size: data.len() as i64,
                mime_type: mime,
                created_at: now,
                updated_at: now,
            })
        })
    })
}

fn image(parent: &str, bytes: &[u8], hash: &str) -> ContentEntry {
    ContentEntry::BinaryFile {
        workspace: WS.to_string(),
        zip_path: format!("content/{WS}/{parent}/logo.png"),
        filename: "logo.png".to_string(),
        parent_path: parent.to_string(),
        data: bytes.to_vec(),
        metadata: Box::new(None),
        content_hash: hash.to_string(),
    }
}

/// `defaults: skip`, with `/default/owned` handed to the package.
fn policy() -> raisin_packages::SyncConfig {
    serde_yaml::from_str(
        "defaults:\n  mode: skip\nfilters:\n  - root: /default/owned\n    mode: replace\n",
    )
    .unwrap()
}

async fn install(
    storage: &Arc<RocksDBStorage>,
    entries: Vec<ContentEntry>,
    policy: Option<&raisin_packages::SyncConfig>,
) -> InstallStats {
    let mut stats = InstallStats::default();
    let store = fake_store();
    PackageInstallHandler::new(storage.clone(), Arc::new(JobRegistry::new()))
        .install_sorted_entries(
            entries,
            &HashMap::new(),
            TENANT,
            REPO,
            BRANCH,
            &JobId::new(),
            InstallMode::Sync,
            policy,
            &HashMap::new(),
            Some(&store),
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

async fn stored_hash(storage: &RocksDBStorage, path: &str) -> String {
    let node = storage
        .nodes()
        .get_by_path(StorageScope::new(TENANT, REPO, BRANCH, WS), path, None)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("{path} installed"));
    match node.properties.get("content_hash") {
        Some(PropertyValue::String(s)) => s.clone(),
        other => panic!("content_hash on {path}: {other:?}"),
    }
}

#[tokio::test]
async fn existing_asset_on_a_skip_path_survives_a_changed_redeploy() {
    let (_dir, storage) = setup().await;
    install(
        &storage,
        vec![image("site", b"v1", "hash-v1")],
        Some(&policy()),
    )
    .await;

    let stats = install(
        &storage,
        vec![image("site", b"v2", "hash-v2")],
        Some(&policy()),
    )
    .await;

    assert_eq!(stats.content_nodes_skipped, 1);
    assert_eq!(stats.content_nodes_synced, 0);
    assert_eq!(stored_hash(&storage, "/site/logo.png").await, "hash-v1");
}

#[tokio::test]
async fn existing_asset_on_a_replace_path_is_updated() {
    let (_dir, storage) = setup().await;
    install(
        &storage,
        vec![image("owned", b"v1", "hash-v1")],
        Some(&policy()),
    )
    .await;

    let stats = install(
        &storage,
        vec![image("owned", b"v2", "hash-v2")],
        Some(&policy()),
    )
    .await;

    assert_eq!(stats.content_nodes_synced, 1);
    assert_eq!(stored_hash(&storage, "/owned/logo.png").await, "hash-v2");
}

#[tokio::test]
async fn without_a_policy_sync_still_updates_a_changed_asset() {
    let (_dir, storage) = setup().await;
    install(&storage, vec![image("site", b"v1", "hash-v1")], None).await;

    install(&storage, vec![image("site", b"v2", "hash-v2")], None).await;

    assert_eq!(stored_hash(&storage, "/site/logo.png").await, "hash-v2");
}

/// A node that references an asset the SAME package ships as a binary file is
/// accepted on a fresh repository. The binaries used to be installed after
/// every node, so the reference was checked before the asset existed and the
/// node was rejected ("Referenced node not found"): the first install of a
/// package failed and only the engine's retry, seconds later, succeeded.
#[tokio::test]
async fn a_node_referencing_a_bundled_binary_installs_on_a_fresh_repo() {
    use raisin_models::nodes::properties::RaisinReference;
    use raisin_models::nodes::Node;

    let (_dir, storage) = setup().await;
    let page = ContentEntry::NodeDef {
        workspace: WS.to_string(),
        yaml_path: "content/default/doc/.node.yaml".to_string(),
        legacy_path: None,
        node: Box::new(Node {
            id: nanoid::nanoid!(),
            node_type: "raisin:Folder".to_string(),
            name: "doc".to_string(),
            path: "/doc".to_string(),
            workspace: Some(WS.to_string()),
            properties: HashMap::from([(
                "file".to_string(),
                PropertyValue::Reference(RaisinReference {
                    id: "/media/logo.png".to_string(),
                    workspace: WS.to_string(),
                    path: String::new(),
                }),
            )]),
            ..Default::default()
        }),
    };
    // Package order: the node first, the binary after it, as the collector
    // produces them.
    let sorted = super::install_content::reference_sort::sort_by_references(vec![
        page,
        image("media", b"png", "hash-1"),
    ]);
    let mut entries = sorted.ordered;
    entries.extend(sorted.circular);
    entries.extend(sorted.other);

    install(&storage, entries, None).await; // asserts no rejected entries

    for path in ["/doc", "/media/logo.png"] {
        assert!(
            storage
                .nodes()
                .get_by_path(StorageScope::new(TENANT, REPO, BRANCH, WS), path, None)
                .await
                .unwrap()
                .is_some(),
            "{path} installed"
        );
    }
}
