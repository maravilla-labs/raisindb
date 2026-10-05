//! A translation conflict's resolution writes the chosen overlay at the merge
//! revision (plan Phase 11; the Phase 2 item 10 gap).
//!
//! Merge apply wrote nothing for a translation conflict — the copy replayed
//! the source's versions and the newest won, so `keep-ours` was a no-op
//! whenever the source edit was later — and instead rewrote the NODE with the
//! chosen side's content, though only the overlay was in dispute.

use raisin_context::{ConflictResolution, MergeStrategy, RepositoryConfig, ResolutionType};
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::translations::{JsonPointer, LocaleCode, LocaleOverlay};
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{
    BranchRepository, RegistryRepository, RepositoryManagementRepository, Storage,
    TranslationRepository,
};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

const T: &str = "mt-tenant";
const R: &str = "mt-repo";
const WS: &str = "content";
const NODE: &str = "translated";

fn title(text: &str) -> LocaleOverlay {
    let mut data = HashMap::new();
    data.insert(
        JsonPointer::new("/title"),
        PropertyValue::String(text.into()),
    );
    LocaleOverlay::Properties { data }
}

async fn env() -> (Arc<RocksDBStorage>, TempDir) {
    let dir = TempDir::new().unwrap();
    let storage = Arc::new(RocksDBStorage::new(dir.path()).unwrap());
    storage
        .registry()
        .register_tenant(T, HashMap::new())
        .await
        .unwrap();
    storage
        .repository_management()
        .create_repository(
            T,
            R,
            RepositoryConfig {
                default_language: "en".to_string(),
                supported_languages: vec!["en".into(), "fr".into()],
                locale_fallback_chains: HashMap::new(),
                default_branch: "main".to_string(),
                description: None,
                tags: HashMap::new(),
                localized_names: Default::default(),
            },
        )
        .await
        .unwrap();
    storage
        .branches()
        .create_branch(T, R, "main", "u", None, None, false, false)
        .await
        .unwrap();
    let mut workspace = raisin_models::workspace::Workspace::new(WS.to_string());
    workspace.config.default_branch = "main".to_string();
    WorkspaceService::new(storage.clone())
        .put(T, R, workspace)
        .await
        .unwrap();
    (storage, dir)
}

async fn begin(
    storage: &RocksDBStorage,
    branch: &str,
) -> Box<dyn raisin_storage::transactional::TransactionalContext> {
    let ctx = storage.begin_context().await.unwrap();
    ctx.set_tenant_repo(T, R).unwrap();
    ctx.set_branch(branch).unwrap();
    ctx.set_actor("u").unwrap();
    ctx.set_auth_context(AuthContext::system()).unwrap();
    ctx.set_message("edit").unwrap();
    ctx.set_validate_schema(false).unwrap();
    ctx
}

async fn translate(storage: &RocksDBStorage, branch: &str, text: &str) {
    let ctx = begin(storage, branch).await;
    ctx.store_translation(WS, NODE, "fr", title(text))
        .await
        .unwrap();
    ctx.commit().await.unwrap();
}

/// A node on main, forked, then `fr` translated differently on both sides —
/// the SOURCE last, so "newest wins" would pick theirs.
async fn diverged() -> (Arc<RocksDBStorage>, TempDir) {
    let (storage, dir) = env().await;
    let ctx = begin(&storage, "main").await;
    let node = Node {
        id: NODE.to_string(),
        name: "translated".to_string(),
        path: "/translated".to_string(),
        parent: Some("/".to_string()),
        node_type: "test:Page".to_string(),
        ..Default::default()
    };
    ctx.add_node(WS, &node).await.unwrap();
    ctx.commit().await.unwrap();
    storage
        .branches()
        .create_branch(
            T,
            R,
            "feature",
            "u",
            None,
            Some("main".into()),
            false,
            false,
        )
        .await
        .unwrap();
    translate(&storage, "main", "ours").await;
    translate(&storage, "feature", "theirs").await;

    let attempt = storage
        .branches_impl()
        .merge_branches(T, R, "main", "feature", MergeStrategy::ThreeWay, "try", "u")
        .await
        .unwrap();
    assert!(
        attempt
            .conflicts
            .iter()
            .any(|c| c.node_id == NODE && c.translation_locale.as_deref() == Some("fr")),
        "the overlays must conflict: {:?}",
        attempt.conflicts
    );
    (storage, dir)
}

async fn resolved_fr(kind: ResolutionType, payload: serde_json::Value) -> Option<LocaleOverlay> {
    let (storage, _dir) = diverged().await;
    let result = storage
        .branches_impl()
        .resolve_merge_with_resolutions(
            T,
            R,
            "main",
            "feature",
            vec![ConflictResolution {
                node_id: NODE.to_string(),
                resolution_type: kind,
                resolved_properties: payload,
                translation_locale: Some("fr".to_string()),
            }],
            "resolved",
            "u",
        )
        .await
        .unwrap();
    assert!(result.success);
    let head = storage.branches().get_head(T, R, "main").await.unwrap();
    storage
        .translations()
        .get_translation(
            T,
            R,
            "main",
            WS,
            NODE,
            &LocaleCode::parse("fr").unwrap(),
            &head,
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn keep_ours_keeps_the_target_overlay_over_a_later_source_edit() {
    assert_eq!(
        resolved_fr(ResolutionType::KeepOurs, serde_json::Value::Null).await,
        Some(title("ours"))
    );
}

#[tokio::test]
async fn keep_theirs_writes_the_source_overlay() {
    assert_eq!(
        resolved_fr(ResolutionType::KeepTheirs, serde_json::Value::Null).await,
        Some(title("theirs"))
    );
}

#[tokio::test]
async fn manual_resolution_writes_the_given_overlay_or_deletes_it() {
    assert_eq!(
        resolved_fr(
            ResolutionType::Manual,
            serde_json::json!({ "/title": "merged" })
        )
        .await,
        Some(title("merged"))
    );
    assert_eq!(
        resolved_fr(
            ResolutionType::Manual,
            serde_json::json!({ "type": "hidden" })
        )
        .await,
        Some(LocaleOverlay::Hidden)
    );
    assert_eq!(
        resolved_fr(ResolutionType::Manual, serde_json::Value::Null).await,
        None
    );
}
