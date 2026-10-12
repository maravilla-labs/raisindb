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
