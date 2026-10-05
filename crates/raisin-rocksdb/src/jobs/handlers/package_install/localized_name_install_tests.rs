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

// ---------------------------------------------------------------------------
// A package's overlays obey the same install decision as its `.node.yaml`.
//
// `store_translation` replaces the whole locale overlay, so an overlay applied
// to an existing node on a `skip` path wipes everything editors changed since
// the first install — translated fields, the native `/__node_name`, a `Hidden`
// marker — on every `--mode sync` redeploy.
// ---------------------------------------------------------------------------

mod sync_policy {
    use super::*;
    use raisin_models::nodes::properties::PropertyValue;
    use raisin_models::translations::{JsonPointer, LocaleOverlay};
    use raisin_packages::{SyncConfig, SyncDefaults, SyncFilter, SyncMode};
    use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};

    /// `defaults: mode: skip`, with `/site/managed` under a `replace` root.
    fn skip_default_config() -> SyncConfig {
        SyncConfig {
            defaults: SyncDefaults {
                mode: SyncMode::Skip,
                ..SyncDefaults::default()
            },
            filters: vec![SyncFilter {
                root: format!("/{WS}/site/managed"),
                mode: Some(SyncMode::Replace),
                direction: None,
                filter_type: Default::default(),
                include: Vec::new(),
                exclude: Vec::new(),
                on_conflict: None,
                properties: None,
            }],
            ..SyncConfig::default()
        }
    }

    fn package() -> Vec<ContentEntry> {
        vec![
            folder("/site"),
            folder("/site/page"),
            folder("/site/hidden"),
            folder("/site/managed"),
            overlay("/site/page", "title: Titre v1\n__node_name: page-v1\n"),
            overlay("/site/hidden", "title: Masque v1\n"),
            overlay("/site/managed", "title: Gere v1\n"),
        ]
    }

    async fn install_with(
        storage: &Arc<RocksDBStorage>,
        mode: InstallMode,
        cfg: Option<&SyncConfig>,
    ) -> InstallStats {
        let mut stats = InstallStats::default();
        PackageInstallHandler::new(storage.clone(), Arc::new(JobRegistry::new()))
            .install_sorted_entries(
                package(),
                &HashMap::new(),
                TENANT,
                REPO,
                BRANCH,
                &JobId::new(),
                mode,
                cfg,
                &HashMap::new(),
                None,
                &mut stats,
            )
            .await
            .expect("per-entry rejections are collected, not returned");
        assert!(
            stats.content_errors.is_empty(),
            "{:?}",
            stats.content_errors
        );
        stats
    }

    async fn tx(storage: &RocksDBStorage) -> Box<dyn TransactionalContext> {
        let tx = storage.begin_context().await.unwrap();
        tx.set_tenant_repo(TENANT, REPO).unwrap();
        tx.set_branch(BRANCH).unwrap();
        tx.set_actor("editor").unwrap();
        tx.set_auth_context(raisin_models::auth::AuthContext::system())
            .unwrap();
        tx
    }

    async fn fr(storage: &RocksDBStorage, path: &str) -> LocaleOverlay {
        let id = id_of(storage, path).await;
        tx(storage)
            .await
            .get_translation(WS, &id, "fr")
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{path} has a fr overlay"))
    }

    fn field(overlay: &LocaleOverlay, key: &str) -> Option<String> {
        match overlay
            .properties_ref()?
            .get(&JsonPointer::new(format!("/{key}")))?
        {
            PropertyValue::String(s) => Some(s.clone()),
            other => panic!("{key}: {other:?}"),
        }
    }

    fn props(pairs: &[(&str, &str)]) -> LocaleOverlay {
        LocaleOverlay::properties(
            pairs
                .iter()
                .map(|(k, v)| {
                    (
                        JsonPointer::new(format!("/{k}")),
                        PropertyValue::String(v.to_string()),
                    )
                })
                .collect(),
        )
    }

    /// What an editor does in Studio after the first install.
    async fn editor_changes(storage: &RocksDBStorage) {
        let page = id_of(storage, "/site/page").await;
        let hidden = id_of(storage, "/site/hidden").await;
        let managed = id_of(storage, "/site/managed").await;
        let tx = tx(storage).await;
        tx.set_message("editor").unwrap();
        tx.store_translation(
            WS,
            &page,
            "fr",
            props(&[("title", "Titre editeur"), ("__node_name", "page-editeur")]),
        )
        .await
        .unwrap();
        tx.store_translation(WS, &hidden, "fr", LocaleOverlay::Hidden)
            .await
            .unwrap();
        tx.store_translation(WS, &managed, "fr", props(&[("title", "Gere editeur")]))
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    async fn first_install_then_edit(storage: &Arc<RocksDBStorage>) {
        let cfg = skip_default_config();
        install_with(storage, InstallMode::Sync, Some(&cfg)).await;
        editor_changes(storage).await;
    }

    #[tokio::test]
    async fn fresh_install_applies_overlays_even_on_skip_paths() {
        let (_dir, storage) = setup(false).await;
        let cfg = skip_default_config();
        let stats = install_with(&storage, InstallMode::Sync, Some(&cfg)).await;

        assert_eq!(stats.translations_applied, 3, "every node was created now");
        assert_eq!(stats.translations_kept, 0);
        let page = fr(&storage, "/site/page").await;
        assert_eq!(field(&page, "title").as_deref(), Some("Titre v1"));
        assert_eq!(field(&page, "__node_name").as_deref(), Some("page-v1"));
        let hidden = fr(&storage, "/site/hidden").await;
        assert_eq!(field(&hidden, "title").as_deref(), Some("Masque v1"));
    }

    #[tokio::test]
    async fn sync_redeploy_keeps_overlays_on_skip_paths_and_reapplies_replace_roots() {
        let (_dir, storage) = setup(false).await;
        first_install_then_edit(&storage).await;

        let cfg = skip_default_config();
        let stats = install_with(&storage, InstallMode::Sync, Some(&cfg)).await;

        assert_eq!(stats.translations_kept, 2, "page + hidden are skip paths");
        assert_eq!(stats.translations_applied, 1, "managed is a replace root");

        // The editor's overlay — translated field AND native node name — survives.
        let page = fr(&storage, "/site/page").await;
        assert_eq!(field(&page, "title").as_deref(), Some("Titre editeur"));
        assert_eq!(field(&page, "__node_name").as_deref(), Some("page-editeur"));
        // The Hidden marker survives.
        assert!(
            matches!(fr(&storage, "/site/hidden").await, LocaleOverlay::Hidden),
            "the editor's Hidden overlay must not be reset"
        );
        // The replace root is package-owned and re-applied.
        let managed = fr(&storage, "/site/managed").await;
        assert_eq!(field(&managed, "title").as_deref(), Some("Gere v1"));
    }

    #[tokio::test]
    async fn overwrite_redeploy_reapplies_every_overlay() {
        let (_dir, storage) = setup(false).await;
        first_install_then_edit(&storage).await;

        let cfg = skip_default_config();
        let stats = install_with(&storage, InstallMode::Overwrite, Some(&cfg)).await;

        assert_eq!(stats.translations_kept, 0);
        assert_eq!(stats.translations_applied, 3);
        let page = fr(&storage, "/site/page").await;
        assert_eq!(field(&page, "title").as_deref(), Some("Titre v1"));
        assert_eq!(field(&page, "__node_name").as_deref(), Some("page-v1"));
        let hidden = fr(&storage, "/site/hidden").await;
        assert_eq!(field(&hidden, "title").as_deref(), Some("Masque v1"));
        let managed = fr(&storage, "/site/managed").await;
        assert_eq!(field(&managed, "title").as_deref(), Some("Gere v1"));
    }

    #[tokio::test]
    async fn skip_mode_without_sync_config_keeps_existing_overlays() {
        let (_dir, storage) = setup(false).await;
        first_install_then_edit(&storage).await;

        let stats = install_with(&storage, InstallMode::Skip, None).await;

        assert_eq!(stats.translations_kept, 3);
        assert_eq!(stats.translations_applied, 0);
        assert!(matches!(
            fr(&storage, "/site/hidden").await,
            LocaleOverlay::Hidden
        ));
    }
}
