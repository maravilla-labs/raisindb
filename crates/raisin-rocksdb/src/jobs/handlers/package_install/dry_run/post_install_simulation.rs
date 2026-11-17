// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Dry run of the `post_install` phase: which hooks would run, in order, and
//! whether their function is there to run.

use raisin_packages::PostInstallHook;
use raisin_storage::scope::StorageScope;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{NodeRepository, Storage};
use std::io::Cursor;
use zip::ZipArchive;

use super::super::handler::PackageInstallHandler;
use super::super::post_install::POST_INSTALL_BRANCH;
use super::super::types::DryRunLogEntry;

impl<S: Storage + TransactionalStorage> PackageInstallHandler<S> {
    /// One log entry per hook (category `post_install`, action `run` or
    /// `skip`). Never runs anything.
    pub(super) async fn dry_run_post_install(
        &self,
        hooks: &[PostInstallHook],
        zip_data: &[u8],
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        logs: &mut Vec<DryRunLogEntry>,
    ) {
        let shipped = shipped_function_paths(zip_data);
        for (i, hook) in hooks.iter().enumerate() {
            let (action, message) = if branch != POST_INSTALL_BRANCH {
                (
                    "skip",
                    format!(
                        "post_install hook {} would not run: branch '{branch}' is not main",
                        i + 1
                    ),
                )
            } else {
                let scope = StorageScope::new(tenant_id, repo_id, branch, "functions");
                let exists = matches!(
                    self.storage
                        .nodes()
                        .get_by_path(scope, &hook.function, None)
                        .await,
                    Ok(Some(_))
                );
                let source = if exists {
                    "function present"
                } else if shipped.iter().any(|p| p == &hook.function) {
                    "function provided by this package"
                } else {
                    "function NOT FOUND: the install would fail at this hook"
                };
                (
                    "run",
                    format!("post_install hook {} would run as system ({source})", i + 1),
                )
            };
            logs.push(DryRunLogEntry {
                level: "info".to_string(),
                category: "post_install".to_string(),
                path: hook.function.clone(),
                message,
                action: action.to_string(),
                policy: None,
            });
        }
    }
}

/// Paths of the functions this package ships (`content/functions/<path>/.node.yaml`).
fn shipped_function_paths(zip_data: &[u8]) -> Vec<String> {
    let Ok(archive) = ZipArchive::new(Cursor::new(zip_data)) else {
        return Vec::new();
    };
    archive
        .file_names()
        .filter_map(|name| {
            name.strip_prefix("content/functions")
                .and_then(|rest| rest.strip_suffix("/.node.yaml"))
                .filter(|p| p.starts_with('/'))
                .map(str::to_string)
        })
        .collect()
}
