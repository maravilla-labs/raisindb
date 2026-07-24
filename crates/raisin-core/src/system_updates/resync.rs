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

//! Content-hash driven resync of built-in definitions into existing repositories.
//!
//! # Why this replaces the version gate
//!
//! The original startup resync (`nodetype_init::init_repository_nodetypes`) only
//! wrote a global NodeType into an existing repo when the YAML's `version:` field
//! was strictly greater than the version already registered there. Editing a
//! definition without also bumping that integer was a silent no-op for every
//! pre-existing tenant: new repos got the new schema, old repos kept the old one
//! and rejected writes carrying the new properties. That footgun cost a full
//! release cycle each time it was hit.
//!
//! This module gates on the **content hash** instead — the same hash
//! `check_pending_updates` already tracks — so any edit propagates, bump or no
//! bump. To keep that safe, changes are classified first: additive/relaxing
//! changes apply automatically, while breaking ones (property removed, type
//! changed, required added, …) are left pending for an explicit, forced admin
//! apply and surface in the admin console's system-updates view.

use super::apply::{apply_nodetype, apply_workspace};
use super::breaking_changes::{
    detect_nodetype_breaking_changes, detect_workspace_breaking_changes,
};
use raisin_error::Result;
use raisin_models::nodes::types::NodeType;
use raisin_models::workspace::Workspace;
use raisin_storage::system_updates::{ResourceType, SystemUpdateRepository};
use raisin_storage::{
    scope::{BranchScope, RepoScope},
    NodeTypeRepository, Storage, WorkspaceRepository,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// How aggressively the startup resync applies changed built-in definitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutoApplyPolicy {
    /// Never write on startup. Changes are only ever recorded as pending and
    /// must be applied through the admin endpoint / console.
    Off,
    /// Apply changes that cannot invalidate existing data (new properties,
    /// relaxed constraints, …); leave breaking changes pending. Default.
    #[default]
    NonBreaking,
    /// Apply everything, including breaking changes. Use only when the
    /// deployment's data is known to tolerate the new schema.
    All,
}

/// What a resync pass did for one repository.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResyncOutcome {
    /// Definitions written and recorded as applied.
    pub applied: usize,
    /// Definitions that changed but were withheld (breaking under
    /// `NonBreaking`, or anything under `Off`).
    pub pending: usize,
    /// Definitions whose content hash already matched — no work done.
    pub unchanged: usize,
}

impl ResyncOutcome {
    fn merge(&mut self, other: ResyncOutcome) {
        self.applied += other.applied;
        self.pending += other.pending;
        self.unchanged += other.unchanged;
    }
}

/// Resync NodeType definitions into one repository, gated on content hash.
///
/// `definitions` is `(NodeType, content_hash)` as produced by
/// `nodetype_init::load_global_nodetypes_with_hashes` — or, once definition
/// overlays are in play, by the resolved definition stack.
pub async fn resync_nodetypes<S: Storage, R: SystemUpdateRepository>(
    storage: Arc<S>,
    system_update_repo: &R,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    definitions: &[(NodeType, String)],
    policy: AutoApplyPolicy,
) -> Result<ResyncOutcome> {
    let mut outcome = ResyncOutcome::default();

    for (nodetype, hash) in definitions {
        let applied = system_update_repo
            .get_applied(tenant_id, repo_id, ResourceType::NodeType, &nodetype.name)
            .await?;

        if applied.as_ref().map(|a| a.content_hash.as_str()) == Some(hash.as_str()) {
            outcome.unchanged += 1;
            continue;
        }

        // A definition that has never been recorded as applied may still exist
        // in the repo (written by the pre-hash-tracking init paths). Compare
        // against what is actually stored so a first-ever hash record does not
        // look like an unclassifiable change.
        let current = storage
            .node_types()
            .get(
                BranchScope::new(tenant_id, repo_id, branch),
                &nodetype.name,
                None,
            )
            .await?;

        let breaking = match &current {
            Some(existing) => detect_nodetype_breaking_changes(existing, nodetype),
            // Not present at all → creating it can't break anything.
            None => vec![],
        };

        if should_apply(policy, breaking.is_empty()) {
            apply_nodetype(
                storage.clone(),
                system_update_repo,
                tenant_id,
                repo_id,
                branch,
                nodetype,
                hash,
                "system",
            )
            .await?;
            outcome.applied += 1;
        } else {
            outcome.pending += 1;
            tracing::warn!(
                tenant_id = %tenant_id,
                repo_id = %repo_id,
                nodetype = %nodetype.name,
                breaking_changes = breaking.len(),
                policy = ?policy,
                "Built-in NodeType changed but was NOT auto-applied; \
                 apply it from the admin console (system updates)"
            );
        }
    }

    Ok(outcome)
}

/// Merge a built-in Workspace definition with what is stored, preserving
/// package-contributed entries in the two allow-lists.
///
/// Builtin packages deliberately EXTEND a workspace's allow-lists when they
/// install — `builtin-packages/ai-tools/manifest.yaml` adds `raisin:AIAgent` to
/// `functions`, the AI task/tool types to `raisin:access_control`, and so on.
/// The workspace YAML is the *base*, not the complete picture.
///
/// So a plain replace during the unattended resync would strip exactly those
/// additions on every server — and because a removal is classified as breaking,
/// it would instead park `default`, `functions` and `raisin:access_control` in
/// the pending list permanently, on a brand-new install, with no action that
/// safely clears them (a forced apply would really delete the package's types
/// and break AI agents and messaging until the packages reinstalled).
///
/// Unioning removes both failure modes: new entries in the YAML land, entries a
/// package added survive, and the allow-lists can never shrink by accident. A
/// genuine removal is a deliberate act and stays the operator's call through the
/// explicit admin apply, which still replaces wholesale.
fn merge_workspace_allow_lists(base: &Workspace, stored: &Workspace) -> Workspace {
    fn union(from_yaml: &[String], from_storage: &[String]) -> Vec<String> {
        let mut merged = from_yaml.to_vec();
        for existing in from_storage {
            if !merged.contains(existing) {
                merged.push(existing.clone());
            }
        }
        merged
    }

    let mut merged = base.clone();
    merged.allowed_node_types = union(&base.allowed_node_types, &stored.allowed_node_types);
    merged.allowed_root_node_types = union(
        &base.allowed_root_node_types,
        &stored.allowed_root_node_types,
    );
    merged
}

/// Resync Workspace definitions into one repository, gated on content hash.
///
/// Allow-lists are merged rather than replaced — see
/// [`merge_workspace_allow_lists`] for why that is required for correctness,
/// not just convenience.
pub async fn resync_workspaces<S: Storage, R: SystemUpdateRepository>(
    storage: Arc<S>,
    system_update_repo: &R,
    tenant_id: &str,
    repo_id: &str,
    definitions: &[(Workspace, String)],
    policy: AutoApplyPolicy,
) -> Result<ResyncOutcome> {
    let mut outcome = ResyncOutcome::default();

    for (workspace, hash) in definitions {
        let applied = system_update_repo
            .get_applied(tenant_id, repo_id, ResourceType::Workspace, &workspace.name)
            .await?;

        if applied.as_ref().map(|a| a.content_hash.as_str()) == Some(hash.as_str()) {
            outcome.unchanged += 1;
            continue;
        }

        let current = storage
            .workspaces()
            .get(RepoScope::new(tenant_id, repo_id), &workspace.name)
            .await?;

        // Merge first, then classify: what we are about to write is the merged
        // definition, so that is what must be checked for breaking changes.
        let effective = match &current {
            Some(existing) => merge_workspace_allow_lists(workspace, existing),
            None => workspace.clone(),
        };

        let breaking = match &current {
            Some(existing) => detect_workspace_breaking_changes(existing, &effective),
            None => vec![],
        };

        if should_apply(policy, breaking.is_empty()) {
            apply_workspace(
                storage.clone(),
                system_update_repo,
                tenant_id,
                repo_id,
                &effective,
                hash,
                "system",
            )
            .await?;
            outcome.applied += 1;
        } else {
            outcome.pending += 1;
            tracing::warn!(
                tenant_id = %tenant_id,
                repo_id = %repo_id,
                workspace = %workspace.name,
                breaking_changes = breaking.len(),
                policy = ?policy,
                "Built-in Workspace changed but was NOT auto-applied; \
                 apply it from the admin console (system updates)"
            );
        }
    }

    Ok(outcome)
}

/// Resync both NodeTypes and Workspaces for one repository.
pub async fn resync_repository_definitions<S: Storage, R: SystemUpdateRepository>(
    storage: Arc<S>,
    system_update_repo: &R,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    nodetypes: &[(NodeType, String)],
    workspaces: &[(Workspace, String)],
    policy: AutoApplyPolicy,
) -> Result<ResyncOutcome> {
    let mut outcome = resync_nodetypes(
        storage.clone(),
        system_update_repo,
        tenant_id,
        repo_id,
        branch,
        nodetypes,
        policy,
    )
    .await?;

    outcome.merge(
        resync_workspaces(
            storage,
            system_update_repo,
            tenant_id,
            repo_id,
            workspaces,
            policy,
        )
        .await?,
    );

    Ok(outcome)
}

fn should_apply(policy: AutoApplyPolicy, is_non_breaking: bool) -> bool {
    match policy {
        AutoApplyPolicy::Off => false,
        AutoApplyPolicy::NonBreaking => is_non_breaking,
        AutoApplyPolicy::All => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashmap::DashMap;
    use raisin_storage::system_updates::AppliedDefinition;
    use raisin_storage_memory::InMemoryStorage;

    /// Minimal in-memory `SystemUpdateRepository` for these tests. The only
    /// production implementation is RocksDB-backed; the resync logic under test
    /// is storage-agnostic, so a map keeps the tests fast and focused.
    #[derive(Default)]
    struct MemoryUpdateRepo {
        applied: DashMap<String, AppliedDefinition>,
    }

    impl MemoryUpdateRepo {
        fn key(tenant: &str, repo: &str, rt: ResourceType, name: &str) -> String {
            format!("{}\0{}\0{}\0{}", tenant, repo, rt, name)
        }
    }

    #[async_trait::async_trait]
    impl raisin_storage::SystemUpdateRepository for MemoryUpdateRepo {
        async fn get_applied(
            &self,
            tenant_id: &str,
            repo_id: &str,
            resource_type: ResourceType,
            name: &str,
        ) -> Result<Option<AppliedDefinition>> {
            Ok(self
                .applied
                .get(&Self::key(tenant_id, repo_id, resource_type, name))
                .map(|e| e.clone()))
        }

        async fn set_applied(
            &self,
            tenant_id: &str,
            repo_id: &str,
            resource_type: ResourceType,
            name: &str,
            entry: AppliedDefinition,
        ) -> Result<()> {
            self.applied
                .insert(Self::key(tenant_id, repo_id, resource_type, name), entry);
            Ok(())
        }

        async fn list_applied(
            &self,
            _tenant_id: &str,
            _repo_id: &str,
        ) -> Result<Vec<(ResourceType, String, AppliedDefinition)>> {
            Ok(vec![])
        }

        async fn delete_applied(
            &self,
            tenant_id: &str,
            repo_id: &str,
            resource_type: ResourceType,
            name: &str,
        ) -> Result<()> {
            self.applied
                .remove(&Self::key(tenant_id, repo_id, resource_type, name));
            Ok(())
        }
    }

    const TENANT: &str = "t";
    const REPO: &str = "r";
    const BRANCH: &str = "main";

    fn prop(name: &str, required: bool) -> String {
        format!(
            "  - name: {}\n    type: String\n    required: {}\n",
            name, required
        )
    }

    /// Build a NodeType the way the loaders do — from YAML — so the tests
    /// exercise the same shape the global definitions have. `version` is
    /// deliberately pinned at 1 in every variant: these tests are about the
    /// content hash gate, and must pass with the version field frozen.
    fn nodetype(props: Vec<String>) -> NodeType {
        serde_yaml::from_str(&format!(
            "name: test:Thing\nversion: 1\nproperties:\n{}",
            props.concat()
        ))
        .expect("test NodeType YAML should parse")
    }

    async fn stored(storage: &Arc<InMemoryStorage>) -> NodeType {
        storage
            .node_types()
            .get(BranchScope::new(TENANT, REPO, BRANCH), "test:Thing", None)
            .await
            .unwrap()
            .expect("test:Thing should be stored")
    }

    /// The whole point of the hash gate: an edit propagates even though the
    /// YAML `version:` field never moved. Under the old version gate this was
    /// a silent no-op and the repo kept rejecting writes for the new property.
    #[tokio::test]
    async fn test_additive_change_applies_without_a_version_bump() {
        let storage = Arc::new(InMemoryStorage::default());
        let repo = MemoryUpdateRepo::default();

        let v1 = vec![(nodetype(vec![prop("title", true)]), "hash-v1".to_string())];
        let outcome = resync_nodetypes(
            storage.clone(),
            &repo,
            TENANT,
            REPO,
            BRANCH,
            &v1,
            AutoApplyPolicy::NonBreaking,
        )
        .await
        .unwrap();
        assert_eq!(outcome.applied, 1);

        // Same definition, new property, SAME version field, new content hash.
        let v2 = vec![(
            nodetype(vec![prop("title", true), prop("sync_policy", false)]),
            "hash-v2".to_string(),
        )];
        let outcome = resync_nodetypes(
            storage.clone(),
            &repo,
            TENANT,
            REPO,
            BRANCH,
            &v2,
            AutoApplyPolicy::NonBreaking,
        )
        .await
        .unwrap();
        assert_eq!(outcome.applied, 1, "additive change must auto-apply");
        assert_eq!(outcome.pending, 0);

        let props = stored(&storage).await.properties.unwrap();
        assert!(props
            .iter()
            .any(|p| p.name.as_deref() == Some("sync_policy")));
    }

    #[tokio::test]
    async fn test_unchanged_hash_is_a_noop() {
        let storage = Arc::new(InMemoryStorage::default());
        let repo = MemoryUpdateRepo::default();
        let defs = vec![(nodetype(vec![prop("title", true)]), "hash".to_string())];

        for expected in [(1, 0), (0, 1)] {
            let outcome = resync_nodetypes(
                storage.clone(),
                &repo,
                TENANT,
                REPO,
                BRANCH,
                &defs,
                AutoApplyPolicy::NonBreaking,
            )
            .await
            .unwrap();
            assert_eq!((outcome.applied, outcome.unchanged), expected);
        }
    }

    #[tokio::test]
    async fn test_breaking_change_stays_pending_but_applies_under_all() {
        let storage = Arc::new(InMemoryStorage::default());
        let repo = MemoryUpdateRepo::default();

        let v1 = vec![(
            nodetype(vec![prop("title", true), prop("legacy", false)]),
            "hash-v1".to_string(),
        )];
        resync_nodetypes(
            storage.clone(),
            &repo,
            TENANT,
            REPO,
            BRANCH,
            &v1,
            AutoApplyPolicy::NonBreaking,
        )
        .await
        .unwrap();

        // Dropping a property is breaking — existing nodes may carry it.
        let v2 = vec![(nodetype(vec![prop("title", true)]), "hash-v2".to_string())];

        let outcome = resync_nodetypes(
            storage.clone(),
            &repo,
            TENANT,
            REPO,
            BRANCH,
            &v2,
            AutoApplyPolicy::NonBreaking,
        )
        .await
        .unwrap();
        assert_eq!(outcome.pending, 1, "breaking change must be withheld");
        assert_eq!(outcome.applied, 0);
        assert!(
            stored(&storage)
                .await
                .properties
                .unwrap()
                .iter()
                .any(|p| p.name.as_deref() == Some("legacy")),
            "the withheld change must not have been written"
        );

        // An operator who accepts the risk can force it through.
        let outcome = resync_nodetypes(
            storage.clone(),
            &repo,
            TENANT,
            REPO,
            BRANCH,
            &v2,
            AutoApplyPolicy::All,
        )
        .await
        .unwrap();
        assert_eq!(outcome.applied, 1);
        assert!(!stored(&storage)
            .await
            .properties
            .unwrap()
            .iter()
            .any(|p| p.name.as_deref() == Some("legacy")));
    }

    #[tokio::test]
    async fn test_policy_off_writes_nothing() {
        let storage = Arc::new(InMemoryStorage::default());
        let repo = MemoryUpdateRepo::default();
        let defs = vec![(nodetype(vec![prop("title", true)]), "hash".to_string())];

        let outcome = resync_nodetypes(
            storage.clone(),
            &repo,
            TENANT,
            REPO,
            BRANCH,
            &defs,
            AutoApplyPolicy::Off,
        )
        .await
        .unwrap();
        assert_eq!(outcome.pending, 1);
        assert_eq!(outcome.applied, 0);
        assert!(storage
            .node_types()
            .get(BranchScope::new(TENANT, REPO, BRANCH), "test:Thing", None)
            .await
            .unwrap()
            .is_none());
    }

    fn workspace(name: &str, allowed: &[&str], roots: &[&str]) -> Workspace {
        let mut ws = Workspace::new(name.to_string());
        ws.allowed_node_types = allowed.iter().map(|s| s.to_string()).collect();
        ws.allowed_root_node_types = roots.iter().map(|s| s.to_string()).collect();
        ws
    }

    /// Regression guard for the ai-tools case: a builtin package extends
    /// `functions` with `raisin:AIAgent` (and `default` /
    /// `raisin:access_control` with the AI task/tool types) via
    /// `workspace_patches`. The resync must neither strip those nor park the
    /// workspace in the pending list forever on a fresh install.
    #[tokio::test]
    async fn test_package_added_types_survive_workspace_resync() {
        let storage = Arc::new(InMemoryStorage::default());
        let repo = MemoryUpdateRepo::default();

        // What the package install left in storage: YAML base + its addition.
        let stored = workspace(
            "functions",
            &["raisin:Folder", "raisin:Function", "raisin:AIAgent"],
            &["raisin:Folder"],
        );
        storage
            .workspaces()
            .put(RepoScope::new(TENANT, REPO), stored)
            .await
            .unwrap();

        // The embedded YAML knows nothing about raisin:AIAgent.
        let defs = vec![(
            workspace(
                "functions",
                &["raisin:Folder", "raisin:Function", "raisin:Trigger"],
                &["raisin:Folder"],
            ),
            "hash-v2".to_string(),
        )];

        let outcome = resync_workspaces(
            storage.clone(),
            &repo,
            TENANT,
            REPO,
            &defs,
            AutoApplyPolicy::NonBreaking,
        )
        .await
        .unwrap();

        assert_eq!(
            outcome.pending, 0,
            "a package-extended workspace must not be permanently pending"
        );
        assert_eq!(outcome.applied, 1);

        let result = storage
            .workspaces()
            .get(RepoScope::new(TENANT, REPO), "functions")
            .await
            .unwrap()
            .unwrap();

        assert!(
            result
                .allowed_node_types
                .contains(&"raisin:AIAgent".to_string()),
            "the package's node type must survive: {:?}",
            result.allowed_node_types
        );
        assert!(
            result
                .allowed_node_types
                .contains(&"raisin:Trigger".to_string()),
            "the YAML's new node type must land: {:?}",
            result.allowed_node_types
        );
    }

    #[test]
    fn test_merge_unions_both_allow_lists_without_duplicates() {
        let base = workspace("w", &["a", "b"], &["r1"]);
        let stored = workspace("w", &["b", "c"], &["r1", "r2"]);
        let merged = merge_workspace_allow_lists(&base, &stored);
        assert_eq!(merged.allowed_node_types, vec!["a", "b", "c"]);
        assert_eq!(merged.allowed_root_node_types, vec!["r1", "r2"]);
    }

    #[test]
    fn test_policy_gating() {
        assert!(!should_apply(AutoApplyPolicy::Off, true));
        assert!(!should_apply(AutoApplyPolicy::Off, false));
        assert!(should_apply(AutoApplyPolicy::NonBreaking, true));
        assert!(!should_apply(AutoApplyPolicy::NonBreaking, false));
        assert!(should_apply(AutoApplyPolicy::All, true));
        assert!(should_apply(AutoApplyPolicy::All, false));
    }

    #[test]
    fn test_default_policy_is_non_breaking() {
        assert_eq!(AutoApplyPolicy::default(), AutoApplyPolicy::NonBreaking);
    }
}
