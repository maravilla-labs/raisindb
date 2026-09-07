// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Regression: a dry run must see the content a previous install created.
//!
//! The dry-run transaction opened without an auth context, so every existence
//! lookup came back empty and an installed package previewed as all `create`
//! with `skip: 0` — the opposite of what the install then did.

use std::collections::HashMap;
use std::io::{Cursor, Write};
use std::sync::Arc;

use raisin_models::nodes::Node;
use raisin_storage::jobs::{JobId, JobRegistry};
use raisin_storage::{RepositoryManagementRepository, Storage};
use tempfile::TempDir;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use super::content_types::{ContentEntry, InstallStats};
use super::handler::PackageInstallHandler;
use super::types::InstallMode;
use crate::RocksDBStorage;

const TENANT: &str = "default";
const REPO: &str = "testrepo";
const BRANCH: &str = "main";
const WS: &str = "functions";

async fn setup() -> (TempDir, Arc<RocksDBStorage>) {
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
    (dir, storage)
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

/// A package whose only content is the folder `/lib/shared` in `functions`.
fn package_zip() -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let mut zip = ZipWriter::new(Cursor::new(&mut buf));
        let opts = SimpleFileOptions::default();
        zip.start_file("manifest.yaml", opts).unwrap();
        zip.write_all(b"name: dry-run-test\nversion: 1.0.0\n")
            .unwrap();
        zip.start_file("content/functions/lib/shared/.node.yaml", opts)
            .unwrap();
        zip.write_all(b"node_type: raisin:Folder\nproperties:\n  title: Shared\n")
            .unwrap();
        zip.finish().unwrap();
    }
    buf
}

#[tokio::test]
async fn dry_run_sees_installed_content() {
    let (_dir, storage) = setup().await;
    let handler = PackageInstallHandler::new(storage.clone(), Arc::new(JobRegistry::new()));

    // Before anything is installed the node is new.
    let before = handler
        .dry_run(TENANT, REPO, BRANCH, &package_zip(), InstallMode::Skip)
        .await
        .unwrap();
    assert_eq!(before.summary.content_nodes.create, 1, "{:?}", before.logs);
    assert_eq!(before.summary.content_nodes.skip, 0);

    // Install it for real.
    let mut stats = InstallStats::default();
    handler
        .install_sorted_entries(
            vec![folder_entry("/lib/shared")],
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
        .unwrap();
    assert!(
        stats.content_errors.is_empty(),
        "{:?}",
        stats.content_errors
    );

    // Now the dry run must report the existing node as skipped, not created.
    let after = handler
        .dry_run(TENANT, REPO, BRANCH, &package_zip(), InstallMode::Skip)
        .await
        .unwrap();
    assert_eq!(
        after.summary.content_nodes.create, 0,
        "installed node previewed as create:\n{:#?}",
        after.logs
    );
    assert_eq!(after.summary.content_nodes.skip, 1, "{:#?}", after.logs);

    let synced = handler
        .dry_run(TENANT, REPO, BRANCH, &package_zip(), InstallMode::Sync)
        .await
        .unwrap();
    assert_eq!(synced.summary.content_nodes.update, 1, "{:#?}", synced.logs);
}
