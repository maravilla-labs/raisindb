// SPDX-License-Identifier: BSL-1.1
//
//! Merge resolution writes through the same funnels every other write uses.
//!
//! Merge apply used to be a mirrored write path of its own: it wrote no
//! ORDERED_CHILDREN entry or tombstone, marked vacated paths with a `\0` byte
//! no reader recognised, and tombstoned only the TARGET's old index values.
//! Because `copy_branch_indexes` replays the source side into the target after
//! the resolution, each gap surfaced as data the user had resolved away:
//!
//! - a path vacated by a delete or rename resolution kept resolving;
//! - a node kept on one side was listed under the other side's parent (or
//!   not at all);
//! - `keep-ours` still matched a value only the source ever had.
//!
//! Writes go through the TRANSACTION context: conflict detection walks
//! `RevisionMeta.changed_nodes`, which only the transaction layer records.

use raisin_context::{ConflictResolution, MergeStrategy, RepositoryConfig, ResolutionType};
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_error::Result;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_rocksdb::{cf, keys, RocksDBStorage};
use raisin_storage::scope::StorageScope;
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::{
    BranchRepository, ListOptions, NodeRepository, PropertyIndexRepository, RegistryRepository,
    RepositoryManagementRepository, Storage,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tempfile::TempDir;

pub(crate) const TENANT: &str = "merge-tenant";
pub(crate) const REPO: &str = "merge-repo";
pub(crate) const WS: &str = "content";

pub(crate) struct Env {
    pub(crate) storage: Arc<RocksDBStorage>,
    _temp_dir: TempDir,
}

pub(crate) fn node(id: &str, path: &str, props: &[(&str, &str)]) -> Node {
    let name = path.rsplit('/').next().unwrap().to_string();
    let parent = match path.rsplitn(2, '/').nth(1) {
        Some(p) if !p.is_empty() => p.rsplit('/').next().map(str::to_string),
        _ => Some("/".to_string()),
    };
    Node {
        id: id.to_string(),
        name,
        path: path.to_string(),
        parent,
        node_type: "test:Page".to_string(),
        properties: props
            .iter()
            .map(|(k, v)| (k.to_string(), PropertyValue::String(v.to_string())))
            .collect(),
        ..Default::default()
    }
}

impl Env {
    pub(crate) async fn new() -> Result<Self> {
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
                    description: None,
                    tags: HashMap::new(),
                    localized_names: Default::default(),
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

    pub(crate) fn scope<'a>(&self, branch: &'a str) -> StorageScope<'a> {
        StorageScope::new(TENANT, REPO, branch, WS)
    }

    pub(crate) async fn tx(&self, branch: &str) -> Result<Box<dyn TransactionalContext>> {
        let ctx = self.storage.begin_context().await?;
        ctx.set_tenant_repo(TENANT, REPO)?;
        ctx.set_branch(branch)?;
        ctx.set_actor("test-user")?;
        ctx.set_auth_context(AuthContext::system())?;
        ctx.set_message("edit")?;
        ctx.set_validate_schema(false)?;
        Ok(ctx)
    }

    pub(crate) async fn add(&self, branch: &str, n: Node) -> Result<()> {
        let ctx = self.tx(branch).await?;
        ctx.add_node(WS, &n).await?;
        ctx.commit().await
    }

    pub(crate) async fn put(&self, branch: &str, n: Node) -> Result<()> {
        let ctx = self.tx(branch).await?;
        ctx.put_node(WS, &n).await?;
        ctx.commit().await
    }

    pub(crate) async fn delete(&self, branch: &str, id: &str) -> Result<()> {
        let ctx = self.tx(branch).await?;
        ctx.delete_node(WS, id).await?;
        ctx.commit().await
    }

    pub(crate) async fn fork(&self) -> Result<()> {
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
        Ok(())
    }

    pub(crate) async fn conflict_and_resolve(&self, id: &str, kind: ResolutionType) -> Result<()> {
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
            "the branches must conflict for the resolution to be under test"
        );
        let result = self
            .storage
            .branches_impl()
            .resolve_merge_with_resolutions(
                TENANT,
                REPO,
                "main",
                "feature",
                vec![ConflictResolution {
                    node_id: id.to_string(),
                    resolution_type: kind,
                    resolved_properties: serde_json::Value::Null,
                    translation_locale: None,
                }],
                "resolved",
                "test-user",
            )
            .await?;
        assert!(result.success);
        Ok(())
    }

    pub(crate) async fn id_at(&self, path: &str) -> Option<String> {
        self.storage
            .nodes()
            .get_node_id_by_path(self.scope("main"), path, None)
            .await
            .unwrap()
    }

    /// Live children of `parent_id` on main, straight from ORDERED_CHILDREN:
    /// newest entry per (label, child) decides. Read raw, so a reader that
    /// confirms liveness cannot mask an entry the writer got wrong.
    fn indexed_children(&self, parent_id: &str) -> HashSet<String> {
        let prefix = keys::ordered_children_prefix(TENANT, REPO, "main", WS, parent_id);
        let db = self.storage.db();
        let cf = db.cf_handle(cf::ORDERED_CHILDREN).unwrap();
        let mut decided = HashSet::new();
        let mut live = HashSet::new();
        for item in db.prefix_iterator_cf(cf, &prefix) {
            let (key, value) = item.unwrap();
            if !key.starts_with(&prefix) {
                break;
            }
            let suffix = &key[prefix.len()..];
            let Some(label_end) = suffix.iter().position(|b| *b == 0) else {
                continue;
            };
            let child_start = label_end + 1 + 16 + 1;
            if suffix.len() <= child_start {
                continue;
            }
            let label = suffix[..label_end].to_vec();
            let child = String::from_utf8_lossy(&suffix[child_start..]).into_owned();
            if decided.insert((label, child.clone())) && !keys::is_tombstone_value(&value) {
                live.insert(child);
            }
        }
        live
    }

    /// Index candidates on main for `prop == value` — the raw index answer,
    /// with no node re-check that could mask a stale entry.
    async fn indexed(&self, prop: &str, value: &str) -> Result<Vec<String>> {
        self.storage
            .property_index()
            .find_by_property(
                self.scope("main"),
                prop,
                &PropertyValue::String(value.to_string()),
                false,
                None,
            )
            .await
    }

    async fn listed(&self, parent_path: &str) -> Vec<String> {
        self.storage
            .nodes()
            .list_children(self.scope("main"), parent_path, ListOptions::for_api())
            .await
            .unwrap()
            .into_iter()
            .map(|n| n.id)
            .collect()
    }
}

#[tokio::test]
async fn merge_keep_ours_source_only_value_not_matched() -> Result<()> {
    let env = Env::new().await?;
    env.add("main", node("c", "/c", &[("title", "base")]))
        .await?;
    env.fork().await?;
    env.put("main", node("c", "/c", &[("title", "ours")]))
        .await?;
    env.put(
        "feature",
        node("c", "/c", &[("title", "theirs"), ("extra", "src-only")]),
    )
    .await?;

    env.conflict_and_resolve("c", ResolutionType::KeepOurs)
        .await?;

    assert!(
        env.indexed("extra", "src-only").await?.is_empty(),
        "a value only the source had must not match after keep-ours"
    );
    assert!(env.indexed("title", "theirs").await?.is_empty());
    assert_eq!(env.indexed("title", "ours").await?, vec!["c".to_string()]);
    Ok(())
}

#[tokio::test]
async fn merge_resolution_vacates_path() -> Result<()> {
    // A delete resolution vacates the node's path.
    let env = Env::new().await?;
    env.add("main", node("c", "/c", &[("title", "base")]))
        .await?;
    env.fork().await?;
    env.put("main", node("c", "/c", &[("title", "ours")]))
        .await?;
    env.put("feature", node("c", "/c", &[("title", "theirs")]))
        .await?;
    env.conflict_and_resolve("c", ResolutionType::Manual)
        .await?;
    assert_eq!(env.id_at("/c").await, None, "a deleted node's path is free");

    // A rename resolution vacates the old path and takes the new one.
    let env = Env::new().await?;
    env.add("main", node("c", "/c", &[("title", "base")]))
        .await?;
    env.fork().await?;
    env.put("main", node("c", "/c", &[("title", "ours")]))
        .await?;
    env.put("feature", node("c", "/renamed", &[("title", "theirs")]))
        .await?;
    env.conflict_and_resolve("c", ResolutionType::KeepTheirs)
        .await?;
    assert_eq!(env.id_at("/c").await, None, "the old path is vacated");
    assert_eq!(env.id_at("/renamed").await.as_deref(), Some("c"));
    Ok(())
}

/// Two parents and one child that the branches disagree about.
async fn tree_env() -> Result<Env> {
    let env = Env::new().await?;
    env.add("main", node("folder", "/folder", &[])).await?;
    env.add("main", node("other", "/other", &[])).await?;
    env.add("main", node("c", "/folder/c", &[("title", "base")]))
        .await?;
    env.fork().await?;
    Ok(env)
}

#[tokio::test]
async fn merge_resolution_keeps_child_listing_consistent() -> Result<()> {
    // MOVE: the source moved the child; keep-ours keeps it where the target has it.
    let env = tree_env().await?;
    env.put("main", node("c", "/folder/c", &[("title", "ours")]))
        .await?;
    env.put("feature", node("c", "/other/c", &[("title", "theirs")]))
        .await?;
    env.conflict_and_resolve("c", ResolutionType::KeepOurs)
        .await?;
    assert!(
        env.indexed_children("folder").contains("c"),
        "move: kept parent"
    );
    assert!(
        !env.indexed_children("other").contains("c"),
        "move: source parent"
    );
    assert_eq!(env.listed("/folder").await, vec!["c".to_string()]);
    assert!(env.listed("/other").await.is_empty());

    // DELETE: a delete resolution unlists the child.
    let env = tree_env().await?;
    env.put("main", node("c", "/folder/c", &[("title", "ours")]))
        .await?;
    env.put("feature", node("c", "/folder/c", &[("title", "theirs")]))
        .await?;
    env.conflict_and_resolve("c", ResolutionType::Manual)
        .await?;
    assert!(!env.indexed_children("folder").contains("c"), "delete");
    assert!(env.listed("/folder").await.is_empty());

    // CREATE: the target deleted the child, the source edited it, and
    // keep-theirs brings it back — listed under its parent.
    let env = tree_env().await?;
    env.delete("main", "c").await?;
    env.put("feature", node("c", "/folder/c", &[("title", "theirs")]))
        .await?;
    env.conflict_and_resolve("c", ResolutionType::KeepTheirs)
        .await?;
    assert!(env.indexed_children("folder").contains("c"), "re-create");
    assert_eq!(env.listed("/folder").await, vec!["c".to_string()]);
    Ok(())
}

#[tokio::test]
async fn merge_keep_ours_source_only_geometry_not_matched() -> Result<()> {
    use raisin_models::nodes::properties::GeoJson;
    use raisin_rocksdb::repositories::spatial_index::SpatialIndexRepository;
    use raisin_storage::spatial::SpatialPreFilter;

    let env = Env::new().await?;
    env.add("main", node("c", "/c", &[("title", "base")]))
        .await?;
    env.fork().await?;
    env.put("main", node("c", "/c", &[("title", "ours")]))
        .await?;
    let mut theirs = node("c", "/c", &[("title", "theirs")]);
    theirs.properties.insert(
        "loc".to_string(),
        PropertyValue::Geometry(GeoJson::point(10.0, 20.0)),
    );
    env.put("feature", theirs).await?;

    env.conflict_and_resolve("c", ResolutionType::KeepOurs)
        .await?;

    let near: Vec<String> = SpatialIndexRepository::new(env.storage.db().clone())
        .find_within_radius(
            TENANT,
            REPO,
            "main",
            WS,
            "loc",
            10.0,
            20.0,
            1_000.0,
            &raisin_hlc::HLC::new(u64::MAX / 2, 0),
            100,
            raisin_rocksdb::spatial::INDEX_PRECISIONS,
            &SpatialPreFilter::default(),
        )?
        .into_iter()
        .map(|hit| hit.node_id)
        .collect();
    assert!(
        near.is_empty(),
        "a geometry only the source had must not match after keep-ours: {near:?}"
    );
    Ok(())
}

/// A merge resolution that places a child under a parent advances the
/// parent's last-child metadata, so the next local append mints a label
/// AFTER it — not one computed from the stale metadata, which can sort before
/// the merged-in child.
#[tokio::test]
async fn append_after_merge_inserted_child_sorts_last() -> Result<()> {
    let env = Env::new().await?;
    env.add("main", node("folder", "/folder", &[])).await?;
    env.add("main", node("other", "/other", &[])).await?;
    env.add("main", node("a", "/other/a", &[])).await?;
    env.add("main", node("c", "/folder/c", &[("title", "base")]))
        .await?;
    env.fork().await?;
    env.put("main", node("c", "/folder/c", &[("title", "ours")]))
        .await?;
    // The source appends two children and then `c` under `other`, so `c`'s
    // label sits well past anything the target's own metadata knows of.
    env.add("feature", node("x", "/other/x", &[])).await?;
    env.add("feature", node("y", "/other/y", &[])).await?;
    env.put("feature", node("c", "/other/c", &[("title", "theirs")]))
        .await?;
    env.conflict_and_resolve("c", ResolutionType::KeepTheirs)
        .await?;

    env.add("main", node("e", "/other/e", &[])).await?;

    let entries = env
        .storage
        .nodes()
        .list_ordered_children_page(env.scope("main"), "other", None, None, false, None)
        .await?;
    let label = |id: &str| {
        entries
            .iter()
            .find(|e| e.child_id == id)
            .map(|e| e.order_label.clone())
            .unwrap_or_else(|| panic!("{id} is listed under /other"))
    };
    assert!(
        label("e") > label("c"),
        "the appended child sorts after the merged-in one: e={} c={}",
        label("e"),
        label("c")
    );
    Ok(())
}
