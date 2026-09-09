// SPDX-License-Identifier: BSL-1.1
//
//! A merge conflict resolution must actually change the data.
//!
//! `resolve_merge_with_resolutions` used to record each resolution in the merge
//! commit's `changed_nodes` list and then write nothing — the applying step was
//! a `// TODO`. Because `copy_branch_indexes` then replays the SOURCE branch's
//! entries into the target at their original revisions, the outcome was always
//! "newest revision wins": `keep-ours` was a no-op whenever the source edit was
//! later, and `keep-theirs` was a no-op whenever the target edit was later. The
//! call returned `success: true` either way, so nothing anywhere reported that
//! the user's choice had been discarded.
//!
//! These tests make the two sides disagree, resolve each way, and read the
//! value back. They deliberately drive writes through the TRANSACTION context
//! rather than the node repository, because conflict detection walks
//! `RevisionMeta.changed_nodes` and only the transaction layer records it.

use raisin_context::{ConflictResolution, MergeStrategy, RepositoryConfig, ResolutionType};
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_error::Result;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::scope::StorageScope;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{
    BranchRepository, NodeRepository, RegistryRepository, RepositoryManagementRepository, Storage,
};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "test-tenant";
const REPO: &str = "test-repo";
const WS: &str = "content";
const NODE_ID: &str = "conflicted-node";

struct Env {
    storage: Arc<RocksDBStorage>,
    _temp_dir: TempDir,
}

impl Env {
    async fn new() -> Result<Self> {
        let temp_dir =
            tempfile::tempdir().map_err(|e| raisin_error::Error::Backend(e.to_string()))?;
        let storage = Arc::new(RocksDBStorage::new(temp_dir.path())?);

        storage
            .registry()
            .register_tenant(TENANT, HashMap::new())
            .await?;
        storage
            .repository_management()
            .create_repository(
                TENANT,
                REPO,
                RepositoryConfig {
                    default_language: "en".to_string(),
                    supported_languages: vec!["en".to_string()],
                    locale_fallback_chains: HashMap::new(),
                    default_branch: "main".to_string(),
                    description: Some("merge resolution test".to_string()),
                    tags: HashMap::new(),
                },
            )
            .await?;
        storage
            .branches()
            .create_branch(TENANT, REPO, "main", "test-user", None, None, false, false)
            .await?;

        let mut workspace = raisin_models::workspace::Workspace::new(WS.to_string());
        workspace.config.default_branch = "main".to_string();
        WorkspaceService::new(storage.clone())
            .put(TENANT, REPO, workspace)
            .await?;

        Ok(Self {
            storage,
            _temp_dir: temp_dir,
        })
    }

    fn scope<'a>(&self, branch: &'a str) -> StorageScope<'a> {
        StorageScope::new(TENANT, REPO, branch, WS)
    }

    /// Create the node on `branch` through the transaction layer.
    async fn create(&self, branch: &str, title: &str) -> Result<()> {
        let ctx = self.storage.begin_context().await?;
        ctx.set_tenant_repo(TENANT, REPO)?;
        ctx.set_branch(branch)?;
        ctx.set_actor("test-user")?;
        ctx.set_auth_context(AuthContext::system())?;
        ctx.set_message("create")?;
        ctx.set_validate_schema(false)?;

        let mut properties = HashMap::new();
        properties.insert(
            "title".to_string(),
            PropertyValue::String(title.to_string()),
        );
        let node = Node {
            id: NODE_ID.to_string(),
            name: "conflicted".to_string(),
            path: "/conflicted".to_string(),
            parent: Some("/".to_string()),
            node_type: "test:Page".to_string(),
            properties,
            ..Default::default()
        };

        ctx.add_node(WS, &node).await?;
        ctx.commit().await
    }

    /// Overwrite `title` on `branch` through the transaction layer.
    async fn set_title(&self, branch: &str, title: &str) -> Result<()> {
        let ctx = self.storage.begin_context().await?;
        ctx.set_tenant_repo(TENANT, REPO)?;
        ctx.set_branch(branch)?;
        ctx.set_actor("test-user")?;
        ctx.set_auth_context(AuthContext::system())?;
        ctx.set_message("edit")?;
        ctx.set_validate_schema(false)?;

        let mut node = ctx
            .get_node(WS, NODE_ID)
            .await?
            .expect("node must exist on the branch being edited");
        node.properties.insert(
            "title".to_string(),
            PropertyValue::String(title.to_string()),
        );

        ctx.put_node(WS, &node).await?;
        ctx.commit().await
    }

    async fn title(&self, branch: &str) -> Option<String> {
        let node = self
            .storage
            .nodes()
            .get(self.scope(branch), NODE_ID, None)
            .await
            .expect("node read must not error")?;
        match node.properties.get("title") {
            Some(PropertyValue::String(s)) => Some(s.clone()),
            _ => None,
        }
    }

    /// Build the conflicted state: one node, edited on both branches, with the
    /// SOURCE edit last so the pre-fix "newest wins" behaviour would pick it.
    async fn diverge(&self) -> Result<()> {
        self.create("main", "base").await?;
        self.storage
            .branches()
            .create_branch(
                TENANT,
                REPO,
                "feature",
                "test-user",
                None,
                Some("main".to_string()),
                false,
                false,
            )
            .await?;

        self.set_title("main", "ours-value").await?;
        self.set_title("feature", "theirs-value").await?;

        // Merging must report the conflict, or the resolution API is not the
        // thing under test.
        let attempt = self
            .storage
            .branches_impl()
            .merge_branches(
                TENANT,
                REPO,
                "main",
                "feature",
                MergeStrategy::ThreeWay,
                "attempt",
                "test-user",
            )
            .await?;
        assert!(
            !attempt.conflicts.is_empty(),
            "the two branches must actually conflict for this test to mean anything"
        );

        Ok(())
    }

    async fn resolve(&self, resolution: ConflictResolution) -> Result<()> {
        let result = self
            .storage
            .branches_impl()
            .resolve_merge_with_resolutions(
                TENANT,
                REPO,
                "main",
                "feature",
                vec![resolution],
                "resolved",
                "test-user",
            )
            .await?;
        assert!(result.success, "resolution must report success");
        Ok(())
    }
}

fn resolution(kind: ResolutionType, properties: serde_json::Value) -> ConflictResolution {
    ConflictResolution {
        node_id: NODE_ID.to_string(),
        resolution_type: kind,
        resolved_properties: properties,
        translation_locale: None,
    }
}

/// KEEP OURS keeps the TARGET's value even though the source edited it later.
///
/// This is the case the old code got wrong every time: the source edit is the
/// newest revision, so doing nothing silently produced `theirs-value`.
#[tokio::test]
async fn keep_ours_keeps_the_target_value_over_a_later_source_edit() -> Result<()> {
    let env = Env::new().await?;
    env.diverge().await?;

    env.resolve(resolution(
        ResolutionType::KeepOurs,
        serde_json::Value::Null,
    ))
    .await?;

    assert_eq!(
        env.title("main").await.as_deref(),
        Some("ours-value"),
        "keep-ours must leave the target's value in place"
    );
    Ok(())
}

/// KEEP THEIRS writes the SOURCE's value into the target.
#[tokio::test]
async fn keep_theirs_writes_the_source_value_into_the_target() -> Result<()> {
    let env = Env::new().await?;
    env.diverge().await?;

    env.resolve(resolution(
        ResolutionType::KeepTheirs,
        serde_json::Value::Null,
    ))
    .await?;

    assert_eq!(
        env.title("main").await.as_deref(),
        Some("theirs-value"),
        "keep-theirs must write the source's value into the target"
    );
    Ok(())
}

/// A KEEP_OURS / KEEP_THEIRS resolution carries a null payload — that is what
/// SQL `RESOLVE CONFLICTS (..., KEEP_OURS)` sends. Deserializing it into a
/// property map made the whole call fail with `invalid type: null, expected a
/// map` before any write was attempted.
#[tokio::test]
async fn a_null_payload_is_not_an_error_for_keep_ours_or_keep_theirs() -> Result<()> {
    let env = Env::new().await?;
    env.diverge().await?;

    env.storage
        .branches_impl()
        .resolve_merge_with_resolutions(
            TENANT,
            REPO,
            "main",
            "feature",
            vec![resolution(
                ResolutionType::KeepOurs,
                serde_json::Value::Null,
            )],
            "resolved",
            "test-user",
        )
        .await
        .expect("a null payload must be accepted for a side-choosing resolution");

    Ok(())
}

/// A manual resolution lays its properties over the node — the SQL
/// `USE_VALUE(...)` path. It used to parse the value and throw it away.
#[tokio::test]
async fn use_value_writes_the_supplied_value() -> Result<()> {
    let env = Env::new().await?;
    env.diverge().await?;

    env.resolve(resolution(
        ResolutionType::Manual,
        serde_json::json!({ "title": "hand-written" }),
    ))
    .await?;

    assert_eq!(
        env.title("main").await.as_deref(),
        Some("hand-written"),
        "a manual resolution must write the value it was given"
    );
    Ok(())
}

/// The resolved value must be findable, not just readable. The merge copies the
/// LOSING side's property-index entries into the target at a revision a reader
/// finds first, so a resolution that writes only the node blob leaves
/// `properties->>'title'` answering with the value the user rejected.
#[tokio::test]
async fn the_resolved_value_replaces_the_losing_side_in_the_property_index() -> Result<()> {
    let env = Env::new().await?;
    env.diverge().await?;

    env.resolve(resolution(
        ResolutionType::KeepOurs,
        serde_json::Value::Null,
    ))
    .await?;

    let by_rejected_value: Vec<Node> = env
        .storage
        .nodes()
        .find_by_property(
            env.scope("main"),
            "title",
            &PropertyValue::String("theirs-value".to_string()),
        )
        .await?;
    assert!(
        by_rejected_value.is_empty(),
        "the rejected value must no longer match in the property index"
    );

    let by_kept_value: Vec<Node> = env
        .storage
        .nodes()
        .find_by_property(
            env.scope("main"),
            "title",
            &PropertyValue::String("ours-value".to_string()),
        )
        .await?;
    assert_eq!(
        by_kept_value.len(),
        1,
        "the kept value must match in the property index"
    );

    Ok(())
}
