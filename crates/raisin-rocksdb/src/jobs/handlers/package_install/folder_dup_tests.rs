// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Regression: installing content under a folder the workspace's
//! `initial_structure` already seeded must reuse that folder, not create a
//! second (bare) one beside it.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_models::nodes::Node;
use raisin_storage::jobs::{JobId, JobRegistry};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{RepositoryManagementRepository, Storage};
use tempfile::TempDir;

use super::content_types::{ContentEntry, InstallStats};
use super::handler::PackageInstallHandler;
use super::types::InstallMode;
use crate::RocksDBStorage;

const TENANT: &str = "default";
const REPO: &str = "testrepo";
const BRANCH: &str = "main";
const WS: &str = "functions";

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
    raisin_core::workspace_structure_init::create_workspace_initial_structure(
        storage.clone(),
        TENANT,
        REPO,
        WS,
    )
    .await
    .unwrap();
    Env { _dir: dir, storage }
}

fn folder_entry(node_path: &str) -> ContentEntry {
    ContentEntry::NodeDef {
        workspace: WS.to_string(),
        yaml_path: format!("content/functions{node_path}/.node.yaml"),
        node: Box::new(Node {
            id: nanoid::nanoid!(),
            node_type: "raisin:Folder".to_string(),
            name: node_path.rsplit('/').next().unwrap().to_string(),
            path: node_path.to_string(),
            workspace: Some(WS.to_string()),
            properties: HashMap::new(),
            ..Default::default()
        }),
        legacy_path: None,
    }
}

async fn install(
    env: &Env,
    entries: Vec<ContentEntry>,
) -> (raisin_error::Result<()>, InstallStats) {
    let mut stats = InstallStats::default();
    let result = PackageInstallHandler::new(env.storage.clone(), Arc::new(JobRegistry::new()))
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
        .await;
    (result, stats)
}

async fn all_nodes(env: &Env) -> Vec<Node> {
    use raisin_storage::{scope::StorageScope, NodeRepository};
    env.storage
        .nodes()
        .list_all(
            StorageScope::new(TENANT, REPO, BRANCH, WS),
            Default::default(),
        )
        .await
        .unwrap()
}

fn dump(nodes: &[Node]) -> String {
    let mut v: Vec<String> = nodes
        .iter()
        .map(|n| {
            format!(
                "{} id={} type={} title={:?}",
                n.path,
                n.id,
                n.node_type,
                n.properties.get("title")
            )
        })
        .collect();
    v.sort();
    v.join("\n")
}

#[tokio::test]
async fn install_reuses_seeded_folders() {
    let env = setup().await;
    let seeded = all_nodes(&env).await;
    assert!(
        seeded.iter().any(|n| n.path == "/lib"),
        "seed did not create /lib"
    );

    for round in 1..=2 {
        let (r, stats) = install(
            &env,
            vec![
                folder_entry("/triggers/on-publish"),
                folder_entry("/lib/shared"),
                folder_entry("/flows/x"),
            ],
        )
        .await;
        assert!(r.is_ok(), "round {round}: {:?}", r.err());
        assert!(
            stats.content_errors.is_empty(),
            "{:?}",
            stats.content_errors
        );
        let nodes = all_nodes(&env).await;
        let mut by_path: HashMap<&str, usize> = HashMap::new();
        for n in &nodes {
            *by_path.entry(n.path.as_str()).or_default() += 1;
        }
        let dups: Vec<_> = by_path.iter().filter(|(_, c)| **c > 1).collect();
        assert!(
            dups.is_empty(),
            "round {round}: duplicate paths {dups:?}\n{}",
            dump(&nodes)
        );
    }
}
