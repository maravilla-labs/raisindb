//! A storage with `index.skip_unchanged` on and its branches rebuilt by the
//! Phase 7 `property_index` repair, so the delta writer actually skips.

use raisin_context::{ConflictResolution, MergeStrategy, RepositoryConfig, ResolutionType};
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_rocksdb::management::async_indexing::repair::{
    property_index_rebuilt, run_repair, RepairKind, RepairOptions, RepairReport,
};
use raisin_rocksdb::{cf, keys, RocksDBStorage};
use raisin_storage::scope::StorageScope;
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::{
    BranchRepository, PropertyIndexRepository, RegistryRepository, RepositoryManagementRepository,
    Storage,
};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

pub(super) const TENANT: &str = "delta-tenant";
pub(super) const REPO: &str = "delta-repo";
pub(super) const WS: &str = "content";

pub(super) struct Env {
    pub(super) storage: Arc<RocksDBStorage>,
    _temp_dir: TempDir,
}

pub(super) fn node(id: &str, path: &str, props: &[(&str, &str)]) -> Node {
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

pub(super) fn repair_options() -> RepairOptions {
    RepairOptions {
        check_headroom: false,
        max_bytes_per_sec: 0,
        ..RepairOptions::default()
    }
}

impl Env {
    /// A fresh repository with `main` and `WS`; `skip` turns
    /// `index.skip_unchanged` on and rebuilds `main` (so it takes effect).
    pub(super) async fn new(skip: bool) -> Result<Self> {
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
        storage.nodes_impl().set_index_skip_unchanged(skip);
        let env = Self {
            storage,
            _temp_dir: temp_dir,
        };
        if skip {
            env.rebuild("main").await?;
        }
        Ok(env)
    }

    /// Run the `property_index` rebuild on `branch` and assert it unlocked
    /// skip-unchanged there.
    pub(super) async fn rebuild(&self, branch: &str) -> Result<Vec<RepairReport>> {
        let reports = run_repair(
            &self.storage,
            TENANT,
            REPO,
            Some(branch),
            RepairKind::PropertyIndex,
            repair_options(),
        )
        .await?;
        assert!(property_index_rebuilt(
            self.storage.db(),
            TENANT,
            REPO,
            branch,
            "local"
        ));
        Ok(reports)
    }

    pub(super) fn scope<'a>(&self, branch: &'a str) -> StorageScope<'a> {
        StorageScope::new(TENANT, REPO, branch, WS)
    }

    pub(super) async fn tx(&self, branch: &str) -> Result<Box<dyn TransactionalContext>> {
        let ctx = self.storage.begin_context().await?;
        ctx.set_tenant_repo(TENANT, REPO)?;
        ctx.set_branch(branch)?;
        ctx.set_actor("test-user")?;
        ctx.set_auth_context(AuthContext::system())?;
        ctx.set_message("edit")?;
        ctx.set_validate_schema(false)?;
        Ok(ctx)
    }

    pub(super) async fn add(&self, branch: &str, n: Node) -> Result<()> {
        let ctx = self.tx(branch).await?;
        ctx.add_node(WS, &n).await?;
        ctx.commit().await
    }

    pub(super) async fn put(&self, branch: &str, n: Node) -> Result<()> {
        let ctx = self.tx(branch).await?;
        ctx.put_node(WS, &n).await?;
        ctx.commit().await
    }

    pub(super) async fn fork(&self, name: &str) -> Result<()> {
        self.storage
            .branches()
            .create_branch(
                TENANT,
                REPO,
                name,
                "test-user",
                None,
                Some("main".to_string()),
                false,
                false,
            )
            .await?;
        Ok(())
    }

    pub(super) async fn conflict_and_resolve(
        &self,
        source: &str,
        id: &str,
        kind: ResolutionType,
    ) -> Result<()> {
        let attempt = self
            .storage
            .branches_impl()
            .merge_branches(
                TENANT,
                REPO,
                "main",
                source,
                MergeStrategy::ThreeWay,
                "attempt",
                "test-user",
            )
            .await?;
        assert!(!attempt.conflicts.is_empty(), "the branches must conflict");
        let result = self
            .storage
            .branches_impl()
            .resolve_merge_with_resolutions(
                TENANT,
                REPO,
                "main",
                source,
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

    /// The raw index answer for `prop == value` (no node re-check).
    pub(super) async fn indexed(
        &self,
        branch: &str,
        prop: &str,
        value: &str,
        at: Option<&HLC>,
    ) -> Result<Vec<String>> {
        self.storage
            .property_index()
            .find_by_property(
                self.scope(branch),
                prop,
                &PropertyValue::String(value.to_string()),
                false,
                at,
            )
            .await
    }

    /// The revision of `id`'s newest stored version.
    pub(super) fn newest_revision(&self, branch: &str, id: &str) -> HLC {
        let prefix = keys::node_key_prefix(TENANT, REPO, branch, WS, id);
        let db = self.storage.db();
        let cf = db.cf_handle(cf::NODES).unwrap();
        let (key, _) = db
            .iterator_cf(
                cf,
                rocksdb::IteratorMode::From(&prefix, rocksdb::Direction::Forward),
            )
            .next()
            .expect("a version")
            .unwrap();
        assert!(key.starts_with(&prefix));
        keys::extract_revision_from_key(&key).unwrap()
    }

    /// Every key of `cf_name` stored for `id` AT `revision`, for the
    /// index families whose key ends `{~rev}\0{node_id}` (PROPERTY_INDEX) —
    /// the writes one node write staged there.
    pub(super) fn property_keys_at(&self, branch: &str, id: &str, revision: &HLC) -> usize {
        let prefix = keys::KeyBuilder::new()
            .push(TENANT)
            .push(REPO)
            .push(branch)
            .push(WS)
            .build_prefix();
        let suffix = {
            let mut s = revision.encode_descending().to_vec();
            s.push(0);
            s.extend_from_slice(id.as_bytes());
            s
        };
        let db = self.storage.db();
        let cf = db.cf_handle(cf::PROPERTY_INDEX).unwrap();
        db.iterator_cf(
            cf,
            rocksdb::IteratorMode::From(&prefix, rocksdb::Direction::Forward),
        )
        .map(|item| item.unwrap().0)
        .take_while(|key| key.starts_with(&prefix))
        .filter(|key| key.ends_with(&suffix))
        .count()
    }
}
