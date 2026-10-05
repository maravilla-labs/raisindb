//! A repository on a storage with `history_gc_collapse_runs` on, plus raw
//! readers of the collapsible column families: every group decided newest-
//! first at a revision, read straight off RocksDB so no reader can mask what
//! collapse did.

use raisin_core::services::workspace_service::WorkspaceService;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_rocksdb::management::async_indexing::repair::{
    run_repair, RepairKind, RepairOptions, RepairReport,
};
use raisin_rocksdb::{cf, keys, RocksDBConfig, RocksDBStorage};
use raisin_storage::scope::StorageScope;
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::{
    BranchRepository, RegistryRepository, RepositoryManagementRepository, Storage,
};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use tempfile::TempDir;

pub(super) const TENANT: &str = "collapse-tenant";
pub(super) const REPO: &str = "collapse-repo";
pub(super) const WS: &str = "content";

/// The CFs collapse touches, with whether the revision is the key's tail
/// (`true`) or sits before a trailing node id.
pub(super) const CFS: &[(&str, bool)] = &[
    (cf::PROPERTY_INDEX, false),
    (cf::REFERENCE_INDEX, true),
    (cf::ORDERED_CHILDREN, false),
    (cf::UNIQUE_INDEX, true),
    (cf::COMPOUND_INDEX, false),
];

pub(super) struct Env {
    pub(super) storage: Arc<RocksDBStorage>,
    _temp_dir: TempDir,
}

pub(super) fn node(id: &str, path: &str, title: &str) -> Node {
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
        properties: HashMap::from([(
            "title".to_string(),
            PropertyValue::String(title.to_string()),
        )]),
        ..Default::default()
    }
}

/// Repair options for tests: no disk check, no throttle, no age floor.
pub(super) fn options() -> RepairOptions {
    RepairOptions {
        check_headroom: false,
        max_bytes_per_sec: 0,
        collapse_min_age: std::time::Duration::ZERO,
        ..RepairOptions::default()
    }
}

impl Env {
    pub(super) async fn new() -> Result<Self> {
        Self::with_flag(true).await
    }

    pub(super) async fn with_flag(collapse: bool) -> Result<Self> {
        Self::with_config(|c| c.with_history_gc_collapse_runs(collapse)).await
    }

    /// A storage whose configuration `tweak` adjusts.
    pub(super) async fn with_config(
        tweak: impl FnOnce(RocksDBConfig) -> RocksDBConfig,
    ) -> Result<Self> {
        let temp_dir =
            tempfile::tempdir().map_err(|e| raisin_error::Error::Backend(e.to_string()))?;
        let config = tweak(RocksDBConfig::development().with_path(temp_dir.path()));
        let storage = Arc::new(RocksDBStorage::with_config(config)?);
        storage
            .registry()
            .register_tenant(TENANT, HashMap::new())
            .await?;
        storage
            .repository_management()
            .create_repository(
                TENANT,
                REPO,
                raisin_context::RepositoryConfig {
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

    pub(super) fn scope<'a>(&self, branch: &'a str) -> StorageScope<'a> {
        StorageScope::new(TENANT, REPO, branch, WS)
    }

    pub(super) async fn tx(&self, branch: &str) -> Result<Box<dyn TransactionalContext>> {
        tx_on(&self.storage, branch).await
    }

    pub(super) async fn add(&self, branch: &str, n: Node) -> Result<HLC> {
        let ctx = self.tx(branch).await?;
        ctx.add_node(WS, &n).await?;
        ctx.commit().await?;
        self.head(branch).await
    }

    pub(super) async fn put(&self, branch: &str, n: Node) -> Result<HLC> {
        put_on(&self.storage, branch, n).await
    }

    pub(super) async fn delete(&self, branch: &str, id: &str) -> Result<HLC> {
        let ctx = self.tx(branch).await?;
        ctx.delete_node(WS, id).await?;
        ctx.commit().await?;
        self.head(branch).await
    }

    pub(super) async fn head(&self, branch: &str) -> Result<HLC> {
        self.storage.branches().get_head(TENANT, REPO, branch).await
    }

    pub(super) async fn fork(&self, name: &str, from: &str) -> Result<()> {
        self.storage
            .branches()
            .create_branch(
                TENANT,
                REPO,
                name,
                "test-user",
                None,
                Some(from.to_string()),
                false,
                false,
            )
            .await?;
        Ok(())
    }

    pub(super) async fn repair(
        &self,
        branch: Option<&str>,
        kind: RepairKind,
        opts: RepairOptions,
    ) -> Result<Vec<RepairReport>> {
        run_repair(&self.storage, TENANT, REPO, branch, kind, opts).await
    }

    /// The repairs collapse requires before it touches ORDERED_CHILDREN.
    pub(super) async fn prerequisites(&self, branch: Option<&str>) -> Result<()> {
        for kind in [RepairKind::OrderedChildren, RepairKind::NodePath] {
            let reports = self.repair(branch, kind, options()).await?;
            assert!(reports.iter().all(|r| r.completed), "{reports:?}");
        }
        Ok(())
    }

    pub(super) async fn collapse(
        &self,
        branch: Option<&str>,
        opts: RepairOptions,
    ) -> Result<Vec<RepairReport>> {
        self.repair(branch, RepairKind::CollapseRuns, opts).await
    }

    /// Every versioned key of `cf_name` on `branch`, with its value.
    pub(super) fn raw(&self, cf_name: &str, branch: &str) -> Vec<(Vec<u8>, Vec<u8>)> {
        let prefix = keys::branch_prefix(TENANT, REPO, branch);
        let db = self.storage.db();
        let cf = db.cf_handle(cf_name).unwrap();
        let mut it = db.raw_iterator_cf(cf);
        it.seek(&prefix);
        let mut out = Vec::new();
        while it.valid() {
            let key = it.key().unwrap();
            if !key.starts_with(&prefix) {
                break;
            }
            out.push((key.to_vec(), it.value().unwrap().to_vec()));
            it.next();
        }
        out
    }

    /// Live groups of `cf_name` on `branch` as of `at`: newest entry per group
    /// at or below `at` decides; `group -> value`.
    pub(super) fn decided(
        &self,
        cf_name: &str,
        branch: &str,
        at: &HLC,
    ) -> BTreeMap<Vec<u8>, Vec<u8>> {
        let tail = CFS.iter().find(|(c, _)| *c == cf_name).unwrap().1;
        let mut newest: BTreeMap<Vec<u8>, (HLC, Vec<u8>)> = BTreeMap::new();
        for (key, value) in self.raw(cf_name, branch) {
            let Some((group, rev)) = split(&key, tail) else {
                continue;
            };
            if rev > *at {
                continue;
            }
            match newest.get(&group) {
                Some((seen, _)) if *seen >= rev => {}
                _ => {
                    newest.insert(group, (rev, value));
                }
            }
        }
        newest
            .into_iter()
            .filter(|(_, (_, v))| !keys::is_tombstone_value(v))
            .map(|(g, (_, v))| (g, v))
            .collect()
    }

    /// [`Self::decided`] for every collapsible CF.
    pub(super) fn decided_all(&self, branch: &str, at: &HLC) -> Vec<BTreeMap<Vec<u8>, Vec<u8>>> {
        CFS.iter()
            .map(|(c, _)| self.decided(c, branch, at))
            .collect()
    }
}

/// A transaction on `branch` of the test repository.
pub(super) async fn tx_on(
    storage: &RocksDBStorage,
    branch: &str,
) -> Result<Box<dyn TransactionalContext>> {
    let ctx = storage.begin_context().await?;
    ctx.set_tenant_repo(TENANT, REPO)?;
    ctx.set_branch(branch)?;
    ctx.set_actor("test-user")?;
    ctx.set_auth_context(AuthContext::system())?;
    ctx.set_message("edit")?;
    ctx.set_validate_schema(false)?;
    Ok(ctx)
}

/// `put_node` and commit; returns the HEAD after it. A free function so a
/// test can run it from another thread.
pub(super) async fn put_on(storage: &RocksDBStorage, branch: &str, n: Node) -> Result<HLC> {
    let ctx = tx_on(storage, branch).await?;
    ctx.put_node(WS, &n).await?;
    ctx.commit().await?;
    storage.branches().get_head(TENANT, REPO, branch).await
}

/// `(group, revision)` of a versioned key: the key with its 16-byte revision
/// cut out, located from the end (the revision may contain null bytes).
pub(super) fn split(key: &[u8], tail: bool) -> Option<(Vec<u8>, HLC)> {
    let end = if tail {
        key.len()
    } else {
        key.iter().rposition(|b| *b == 0)?
    };
    let start = end.checked_sub(16)?;
    if start == 0 || key[start - 1] != 0 {
        return None;
    }
    let rev = HLC::decode_descending(&key[start..end]).ok()?;
    let mut group = key[..start].to_vec();
    group.extend_from_slice(&key[end..]);
    Some((group, rev))
}
