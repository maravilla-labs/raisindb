// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Plan Phase 13c: a package overlay's translated node name
//! (`__node_name` in `{node}.node.{locale}.yaml`) obeys localized-name
//! sibling uniqueness like every other local write. A colliding overlay is
//! one rejected entry — the rest of the package, its batch included, lands —
//! when the repository enforces uniqueness, and is installed when it does
//! not.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_models::nodes::Node;
use raisin_storage::jobs::{JobId, JobRegistry};
use raisin_storage::scope::StorageScope;
use raisin_storage::{BranchRepository, NodeRepository, RepositoryManagementRepository, Storage};
use tempfile::TempDir;

use super::content_types::{ContentEntry, InstallStats};
use super::handler::PackageInstallHandler;
use super::translation::yaml_to_overlay;
use super::types::InstallMode;
use crate::localized_name::keys::NameScope;
use crate::management::async_indexing::repair::{run_repair, RepairKind, RepairOptions};
use crate::RocksDBStorage;

const TENANT: &str = "default";
const REPO: &str = "names";
const BRANCH: &str = "main";
const WS: &str = "default";

async fn setup(enforce: bool) -> (TempDir, Arc<RocksDBStorage>) {
    let dir = TempDir::new().unwrap();
    let storage = Arc::new(RocksDBStorage::new(dir.path()).unwrap());
    let mut config = raisin_context::RepositoryConfig {
        default_language: "en".to_string(),
        supported_languages: vec!["en".into(), "fr".into()],
        ..raisin_context::RepositoryConfig::default()
    };
    config.localized_names.enforce_unique = enforce;
    storage
        .repository_management()
        .create_repository(TENANT, REPO, config)
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
    // A clean build: enforcement needs the workspace `Ready`, zero collisions.
    let options = RepairOptions {
        check_headroom: false,
        max_bytes_per_sec: 0,
        ..RepairOptions::default()
    };
    let reports = run_repair(
        &storage,
        TENANT,
        REPO,
        Some(BRANCH),
        RepairKind::LocalizedNames,
        options,
    )
    .await
    .unwrap();
    assert!(reports.iter().all(|r| r.completed), "{reports:?}");
    (dir, storage)
}

fn folder(path: &str) -> ContentEntry {
    ContentEntry::NodeDef {
        workspace: WS.to_string(),
        yaml_path: format!("content/{WS}{path}/.node.yaml"),
        node: Box::new(Node {
            id: nanoid::nanoid!(),
            node_type: "raisin:Folder".to_string(),
            name: path.rsplit('/').next().unwrap().to_string(),
            path: path.to_string(),
            workspace: Some(WS.to_string()),
            ..Default::default()
        }),
        legacy_path: None,
    }
}

fn overlay(path: &str, yaml: &str) -> ContentEntry {
    ContentEntry::TranslationFile {
        workspace: WS.to_string(),
        base_node_yaml_path: format!("content/{WS}{path}/.node.yaml"),
        locale: "fr".to_string(),
        overlay: yaml_to_overlay(serde_yaml::from_str(yaml).unwrap()).unwrap(),
    }
}

async fn install(storage: &Arc<RocksDBStorage>, entries: Vec<ContentEntry>) -> InstallStats {
    let mut stats = InstallStats::default();
    PackageInstallHandler::new(storage.clone(), Arc::new(JobRegistry::new()))
        .install_sorted_entries(
            entries,
            &HashMap::new(),
            TENANT,
            REPO,
            BRANCH,
            &JobId::new(),
            InstallMode::Sync,
            None,
            &HashMap::new(),
            None,
            &mut stats,
        )
        .await
        .expect("per-entry rejections are collected, not returned");
    stats
}

async fn id_of(storage: &RocksDBStorage, path: &str) -> String {
    storage
        .nodes()
        .get_by_path(StorageScope::new(TENANT, REPO, BRANCH, WS), path, None)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("{path} installed"))
        .id
}

/// The nodes claiming `name` in French under `parent` (the claim index,
/// which holds the selector's normalized name).
fn claimants(storage: &RocksDBStorage, parent: &str, name: &str) -> Vec<String> {
    crate::localized_name::rows::claims(
        storage.db(),
        NameScope::new(TENANT, REPO, BRANCH, WS),
        "fr",
        parent,
        name,
        None,
    )
    .unwrap()
    .into_iter()
    .map(|(_, id)| id)
    .collect()
}

/// `/site/a`, `/site/b` and `/site/c` in one batch; `b` is named `a` in
/// French (a URL value, normalized to its last segment), `c` gets a free
/// name.
fn package() -> Vec<ContentEntry> {
    vec![
        folder("/site"),
        folder("/site/a"),
        folder("/site/b"),
        folder("/site/c"),
        overlay("/site/b", "__node_name: /fr/site/a/\n"),
        overlay("/site/c", "__node_name: sea\n"),
    ]
}

#[tokio::test]
async fn package_overlay_colliding_with_a_sibling_is_refused_when_enforced() {
    let (_dir, storage) = setup(true).await;
    let stats = install(&storage, package()).await;

    assert_eq!(stats.translations_applied, 1, "only c's overlay lands");
    assert_eq!(stats.content_errors.len(), 1, "{:?}", stats.content_errors);
    assert!(
        stats.content_errors[0].contains("localized node name 'a'"),
        "{:?}",
        stats.content_errors
    );
    // The batch committed: every node and the free name are there.
    let site = id_of(&storage, "/site").await;
    let c = id_of(&storage, "/site/c").await;
    id_of(&storage, "/site/a").await;
    id_of(&storage, "/site/b").await;
    assert_eq!(claimants(&storage, &site, "sea"), vec![c]);
    assert!(claimants(&storage, &site, "a").is_empty());
}

#[tokio::test]
async fn package_overlay_colliding_with_a_sibling_is_accepted_when_not_enforced() {
    let (_dir, storage) = setup(false).await;
    let stats = install(&storage, package()).await;

    assert!(
        stats.content_errors.is_empty(),
        "{:?}",
        stats.content_errors
    );
    assert_eq!(stats.translations_applied, 2);
    let site = id_of(&storage, "/site").await;
    let b = id_of(&storage, "/site/b").await;
    assert_eq!(claimants(&storage, &site, "a"), vec![b]);
}
