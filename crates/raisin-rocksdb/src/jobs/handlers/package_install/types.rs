// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Public types for package installation

use raisin_error::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Install mode for handling conflicts during package installation
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallMode {
    /// Skip if exists (default) - don't overwrite existing content
    #[default]
    Skip,
    /// Overwrite - delete and replace existing content
    Overwrite,
    /// Sync - update existing content, create new, leave untouched content alone
    Sync,
}

/// Resolve the effective [`InstallMode`] for a single content path, honoring
/// the package's `manifest.yaml` `sync:` policy (defaults + root filters).
///
/// An explicit operator choice of [`InstallMode::Overwrite`] always wins —
/// it is the "force" override and bypasses the package's own policy entirely.
/// Otherwise, if the package ships a `sync` config, its per-path
/// [`raisin_packages::SyncMode`] is consulted: `Skip` never touches existing
/// content, `Replace` always overwrites, and `Merge`/`Update` (not meaningful
/// for a one-shot install) fall back to the operator's chosen mode. With no
/// `sync` config at all, the operator's mode applies unchanged everywhere.
pub(super) fn effective_install_mode_for_path(
    install_mode: InstallMode,
    sync_config: Option<&raisin_packages::SyncConfig>,
    workspace: &str,
    node_path: &str,
) -> InstallMode {
    resolve_install_policy_for_path(install_mode, sync_config, workspace, node_path).mode
}

/// The effective mode for a path, plus (when the package's own `sync:`
/// policy determined or overrode it) a human-readable explanation for
/// display in dry-run previews.
pub(super) struct PathInstallPolicy {
    pub mode: InstallMode,
    /// `None` when the operator's chosen install mode applied unchanged —
    /// either because the package ships no `sync:` config, or because the
    /// operator forced [`InstallMode::Overwrite`], which always bypasses
    /// the package's policy.
    pub reason: Option<String>,
}

/// Same resolution as [`effective_install_mode_for_path`], but also reports
/// *why* — which `sync:` filter (or the top-level default) applied, for
/// surfacing in dry-run output.
pub(super) fn resolve_install_policy_for_path(
    install_mode: InstallMode,
    sync_config: Option<&raisin_packages::SyncConfig>,
    workspace: &str,
    node_path: &str,
) -> PathInstallPolicy {
    if install_mode == InstallMode::Overwrite {
        return PathInstallPolicy {
            mode: install_mode,
            reason: None,
        };
    }

    let Some(sync_config) = sync_config else {
        return PathInstallPolicy {
            mode: install_mode,
            reason: None,
        };
    };

    let full_path = format!("/{}{}", workspace, node_path);
    let (sync_mode, source) = sync_config.get_mode_and_source_for_path(&full_path);
    let mode = match sync_mode {
        raisin_packages::SyncMode::Skip => InstallMode::Skip,
        raisin_packages::SyncMode::Replace => InstallMode::Overwrite,
        // `merge`/`update` are not decisions, they are deferrals: the package
        // is saying "keep this path current, on whatever terms the operator
        // set". So the operator's mode stands.
        //
        // Read this together with the operator-side default. `InstallMode`'s
        // own default is `Skip`, so for a long time NOTHING ever reached here
        // with `Sync`: `deploy --install` sent no `mode` at all, the server
        // defaulted to `Skip`, and every `mode: merge` path — the builtins
        // included — resolved to "never update anything already installed".
        // That is why declared roles drifted from live ones. The fix is that
        // `deploy --install` now sends `mode=sync` explicitly
        // (packages/raisindb-cli/src/commands/deploy.ts), NOT a coercion here:
        // an operator who asks for `skip` must still get `skip`, or the safe
        // option stops being safe.
        raisin_packages::SyncMode::Merge | raisin_packages::SyncMode::Update => install_mode,
    };
    let mode_label = match sync_mode {
        raisin_packages::SyncMode::Skip => "skip",
        raisin_packages::SyncMode::Replace => "replace",
        raisin_packages::SyncMode::Merge => "merge",
        raisin_packages::SyncMode::Update => "update",
    };
    let reason = Some(match source {
        Some(root) => format!("package sync policy: {} (filter '{}')", mode_label, root),
        None => format!("package sync policy: {} (default)", mode_label),
    });

    PathInstallPolicy { mode, reason }
}

#[cfg(test)]
mod effective_install_mode_tests {
    use super::*;
    use raisin_packages::{SyncConfig, SyncDefaults, SyncFilter, SyncMode};

    fn sync_config() -> SyncConfig {
        SyncConfig {
            defaults: SyncDefaults {
                mode: SyncMode::Skip,
                ..SyncDefaults::default()
            },
            filters: vec![
                SyncFilter {
                    root: "/functions".to_string(),
                    mode: Some(SyncMode::Replace),
                    direction: None,
                    filter_type: Default::default(),
                    include: Vec::new(),
                    exclude: Vec::new(),
                    on_conflict: None,
                    properties: None,
                },
                SyncFilter {
                    root: "/apps".to_string(),
                    mode: Some(SyncMode::Replace),
                    direction: None,
                    filter_type: Default::default(),
                    include: Vec::new(),
                    exclude: Vec::new(),
                    on_conflict: None,
                    properties: None,
                },
            ],
            ..SyncConfig::default()
        }
    }

    #[test]
    fn no_sync_config_leaves_operator_mode_unchanged() {
        for mode in [InstallMode::Skip, InstallMode::Sync, InstallMode::Overwrite] {
            assert_eq!(
                effective_install_mode_for_path(mode, None, "stories", "/hello"),
                mode
            );
        }
    }

    #[test]
    fn default_skip_preserves_tenant_content_outside_filters() {
        let cfg = sync_config();
        assert_eq!(
            effective_install_mode_for_path(InstallMode::Sync, Some(&cfg), "stories", "/hello"),
            InstallMode::Skip
        );
        assert_eq!(
            effective_install_mode_for_path(InstallMode::Skip, Some(&cfg), "audiences", "/vip"),
            InstallMode::Skip
        );
    }

    #[test]
    fn filter_root_forces_replace_for_platform_paths() {
        let cfg = sync_config();
        assert_eq!(
            effective_install_mode_for_path(
                InstallMode::Skip,
                Some(&cfg),
                "functions",
                "/lib/handler"
            ),
            InstallMode::Overwrite
        );
        assert_eq!(
            effective_install_mode_for_path(InstallMode::Sync, Some(&cfg), "apps", "/dashboard"),
            InstallMode::Overwrite
        );
    }

    #[test]
    fn operator_overwrite_always_wins_over_manifest_policy() {
        let cfg = sync_config();
        assert_eq!(
            effective_install_mode_for_path(
                InstallMode::Overwrite,
                Some(&cfg),
                "stories",
                "/hello"
            ),
            InstallMode::Overwrite
        );
    }
}

/// Callback type for binary retrieval
///
/// This callback is provided by the transport layer which has access to BinaryStorage.
/// Arguments: (resource_key)
/// Returns: Result<Vec<u8>> - the binary data
/// Callback that records the content hash an install actually applied.
///
/// Exists because the applied-hash record is what makes a package UPDATABLE:
/// the boot-time staleness check compares the shipped content hash against the
/// recorded one, and treats "no record" as "assume current". Only the builtin
/// auto-install path used to write one, so a package installed from the
/// Integrations gallery was frozen at its install-time content forever — every
/// later fix RaisinDB shipped reached no deployed tenant, with the binary
/// carrying the fix and the content that never heard of it sitting side by side.
///
/// A callback rather than a repository handle because the record lives in a
/// concrete RocksDB repository while this handler is generic over storage.
/// Arguments: (tenant_id, repo_id, package_name, content_hash).
pub type AppliedHashRecorder = Arc<
    dyn Fn(
            String,
            String,
            String,
            String,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>>
        + Send
        + Sync,
>;

pub type BinaryRetrievalCallback = Arc<
    dyn Fn(
            String, // resource_key
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>>> + Send>>
        + Send
        + Sync,
>;

/// Callback type for binary storage (writing)
///
/// This callback is provided by the transport layer which has access to BinaryStorage.
/// Arguments: (data, content_type, extension, filename, tenant_context)
/// Returns: Result<StoredObject> - metadata about stored binary
pub type BinaryStorageCallback = Arc<
    dyn Fn(
            Vec<u8>,        // data
            Option<String>, // content_type
            Option<String>, // extension
            Option<String>, // original_name
            Option<String>, // tenant_context
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<raisin_binary::StoredObject>> + Send>,
        > + Send
        + Sync,
>;

/// Callback type for binary storage from file path (writing large files)
///
/// This callback is provided by the transport layer which has access to BinaryStorage.
/// Used for large files to avoid loading entire file into memory.
/// Arguments: (file_path, content_type, extension, filename)
/// Returns: Result<StoredObject> - metadata about stored binary
pub type BinaryStorageFromPathCallback = Arc<
    dyn Fn(
            std::path::PathBuf, // file_path
            Option<String>,     // content_type
            Option<String>,     // extension
            Option<String>,     // original_name
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<raisin_binary::StoredObject>> + Send>,
        > + Send
        + Sync,
>;

/// Result of package installation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageInstallResult {
    /// Package name
    pub package_name: String,
    /// Package version
    pub package_version: String,
    /// Number of mixins installed
    #[serde(default)]
    pub mixins_installed: usize,
    /// Number of node types installed
    pub node_types_installed: usize,
    /// Number of workspaces installed
    pub workspaces_installed: usize,
    /// Number of workspace patches applied
    pub workspace_patches_applied: usize,
    /// Number of content nodes created
    pub content_nodes_created: usize,
    /// Number of binary files installed as assets
    pub binary_files_installed: usize,
    /// Number of translations applied to content nodes
    pub translations_applied: usize,
}

// ============================================================================
// Dry Run Types
// ============================================================================

/// Result of a dry run simulation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DryRunResult {
    pub logs: Vec<DryRunLogEntry>,
    pub summary: DryRunSummary,
}

/// A single log entry from the dry run simulation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DryRunLogEntry {
    /// Log level: "info", "create", "update", "skip"
    pub level: String,
    /// Category: "node_type", "workspace", "content", "binary", "archetype", "element_type"
    pub category: String,
    /// Path or name of the item
    pub path: String,
    /// Human-readable message
    pub message: String,
    /// Action that would be taken: "create", "update", "skip"
    pub action: String,
    /// When the package's own `sync:` policy determined or overrode the
    /// action (rather than the operator's chosen install mode applying
    /// unchanged), a human-readable explanation of which rule applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
}

/// Summary of actions that would be taken
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct DryRunSummary {
    pub mixins: DryRunActionCounts,
    pub node_types: DryRunActionCounts,
    pub archetypes: DryRunActionCounts,
    pub element_types: DryRunActionCounts,
    pub workspaces: DryRunActionCounts,
    pub content_nodes: DryRunActionCounts,
    pub binary_files: DryRunActionCounts,
    pub package_assets: DryRunActionCounts,
}

/// Counts of create/update/skip actions
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct DryRunActionCounts {
    pub create: usize,
    pub update: usize,
    pub skip: usize,
}
