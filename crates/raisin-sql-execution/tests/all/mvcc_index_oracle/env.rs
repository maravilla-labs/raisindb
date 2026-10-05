//! The system under test: one RocksDB storage, its query engines, and the
//! write funnels the driver uses.
//!
//! Nothing in here is consulted by the reference model. The model learns what
//! the database SHOULD contain from the op log alone; this module only knows
//! how to ask the database to do things and how to ask it what it contains.

use futures::StreamExt;
use raisin_context::RepositoryConfig;
use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::schema::{
    CompoundColumnType, CompoundIndexColumn, CompoundIndexDefinition,
};
use raisin_models::nodes::NodeType;
use raisin_rocksdb::{RocksDBConfig, RocksDBStorage};
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, ElementTypeRepository, NodeTypeRepository,
    RegistryRepository, RepoScope, RepositoryManagementRepository, Storage, StorageScope,
    WorkspaceRepository,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

pub const TENANT: &str = "oracle";
pub const REPO: &str = "repo";
pub const MAIN: &str = "main";
pub const WS: &str = "content";
pub const LOCALE: &str = "fr";

/// The three node types the generator draws from. `VOLATILE` is
/// `versionable: false`: its updates overwrite in place and mint no revision.
pub const PAGE: &str = "oracle:Page";
pub const DOC: &str = "oracle:Doc";
pub const VOLATILE: &str = "oracle:Volatile";

pub type Store = RocksDBStorage;

pub struct Env {
    pub storage: Arc<Store>,
    pub config: RepositoryConfig,
    _dir: TempDir,
}

pub fn repo_config() -> RepositoryConfig {
    RepositoryConfig {
        default_language: "en".to_string(),
        supported_languages: vec!["en".to_string(), LOCALE.to_string()],
        locale_fallback_chains: HashMap::new(),
        default_branch: MAIN.to_string(),
        description: None,
        tags: HashMap::new(),
        localized_names: Default::default(),
    }
}

/// `(__parent_path, __created_at)` — declared by every node type (type-owned)
/// AND by the workspace (plan Phase 13e: workspace-owned, stored as
/// `@folder_time`), under the same authored name on purpose.
fn folder_time() -> CompoundIndexDefinition {
    CompoundIndexDefinition {
        name: "folder_time".to_string(),
        columns: vec![
            CompoundIndexColumn {
                property: "__parent_path".to_string(),
                column_type: CompoundColumnType::String,
                ascending: None,
            },
            CompoundIndexColumn {
                property: "__created_at".to_string(),
                column_type: CompoundColumnType::Timestamp,
                ascending: None,
            },
        ],
        has_order_column: true,
        owner: None,
    }
}

fn node_type(name: &str, versionable: bool) -> NodeType {
    let mut nt: NodeType =
        serde_json::from_value(serde_json::json!({ "name": name })).expect("node type literal");
    nt.id = Some(name.to_string());
    nt.strict = Some(false);
    nt.allowed_children = vec!["*".to_string()];
    nt.indexable = Some(true);
    nt.versionable = Some(versionable);
    nt.compound_indexes = Some(vec![folder_time()]);
    nt
}

impl Env {
    /// A fresh repository. `replication_node` turns on operation capture under
    /// that cluster node id (stage 3's origins).
    pub async fn new(replication_node: Option<&str>) -> Self {
        let dir = TempDir::new().expect("temp dir");
        let storage = match replication_node {
            None => RocksDBStorage::new(dir.path()).expect("storage"),
            Some(node) => {
                let mut config = RocksDBConfig::default();
                config.path = dir.path().to_path_buf();
                config.replication_enabled = true;
                config.cluster_node_id = Some(node.to_string());
                RocksDBStorage::with_config(config).expect("replicated storage")
            }
        };
        let storage = Arc::new(storage);
        let config = repo_config();
        storage
            .registry()
            .register_tenant(TENANT, HashMap::new())
            .await
            .expect("tenant");
        storage
            .repository_management()
            .create_repository(TENANT, REPO, config.clone())
            .await
            .expect("repository");
        storage
            .branches()
            .create_branch(TENANT, REPO, MAIN, "oracle", None, None, false, false)
            .await
            .expect("main branch");
        storage
            .workspaces()
            .put(RepoScope::new(TENANT, REPO), {
                let mut ws = raisin_models::workspace::Workspace::new(WS.to_string());
                ws.compound_indexes = Some(vec![folder_time()]);
                ws
            })
            .await
            .expect("workspace");
        for (name, versionable) in [(PAGE, true), (DOC, true), (VOLATILE, false)] {
            storage
                .node_types()
                .upsert(
                    BranchScope::new(TENANT, REPO, MAIN),
                    node_type(name, versionable),
                    CommitMetadata::system("oracle seed"),
                )
                .await
                .expect("node type");
        }
        storage
            .element_types()
            .create(
                BranchScope::new(TENANT, REPO, MAIN),
                serde_json::from_value(serde_json::json!({
                    "name": "oracle:Card",
                    "fields": [
                        { "$type": "TextField", "name": "label" },
                        { "$type": "ReferenceField", "name": "target" },
                    ],
                }))
                .expect("element type literal"),
                CommitMetadata::system("oracle seed"),
            )
            .await
            .expect("element type");
        // A declared compound index is not a built one: the planner declines
        // anything not `Ready`. Building it over the empty workspace makes it
        // `Ready`, and the write funnels maintain it from here on.
        raisin_rocksdb::management::async_indexing::rebuild_indexes(
            &storage,
            TENANT,
            REPO,
            MAIN,
            WS,
            raisin_storage::IndexType::Compound,
        )
        .await
        .expect("compound build");
        // The localized name index, built over the empty branch: `Ready` from
        // here, so `main` lookups take the index and its inline writers are
        // what `checks/localized.rs` tests (plan Phase 12).
        raisin_rocksdb::management::async_indexing::repair::run_repair(
            &storage,
            TENANT,
            REPO,
            Some(MAIN),
            raisin_rocksdb::management::async_indexing::repair::RepairKind::LocalizedNames,
            raisin_rocksdb::management::async_indexing::repair::RepairOptions {
                check_headroom: false,
                max_bytes_per_sec: 0,
                ..Default::default()
            },
        )
        .await
        .expect("localized name build");
        let env = Self {
            storage,
            config,
            _dir: dir,
        };
        env.unlock_skip_unchanged(MAIN).await;
        env
    }

    /// With `index.skip_unchanged` on — the default since plan Phase 7b; run
    /// with `RAISIN_INDEX_SKIP_UNCHANGED=0` to test full puts — run the
    /// `property_index` rebuild on `branch`, the per-node precondition that
    /// lets the delta writer skip there. A no-op with the flag off.
    pub async fn unlock_skip_unchanged(&self, branch: &str) {
        use raisin_rocksdb::management::async_indexing::repair::{
            run_repair, RepairKind, RepairOptions,
        };
        if !self.storage.nodes_impl().index_skip_unchanged() {
            return;
        }
        run_repair(
            &self.storage,
            TENANT,
            REPO,
            Some(branch),
            RepairKind::PropertyIndex,
            RepairOptions {
                check_headroom: false,
                max_bytes_per_sec: 0,
                ..RepairOptions::default()
            },
        )
        .await
        .expect("property_index rebuild");
        let node_id = self
            .storage
            .config()
            .cluster_node_id
            .clone()
            .unwrap_or_else(|| "local".to_string());
        assert!(
            raisin_rocksdb::management::async_indexing::repair::property_index_rebuilt(
                self.storage.db(),
                TENANT,
                REPO,
                branch,
                &node_id,
            ),
            "the rebuild must unlock skip-unchanged on {branch}"
        );
    }

    pub fn scope<'a>(&self, branch: &'a str) -> StorageScope<'a> {
        StorageScope::new(TENANT, REPO, branch, WS)
    }

    pub fn engine(&self, branch: &str) -> QueryEngine<Store> {
        let mut catalog = StaticCatalog::default_nodes_schema();
        catalog.register_workspace(WS.to_string());
        QueryEngine::new(self.storage.clone(), TENANT, REPO, branch)
            .with_catalog(Arc::new(catalog))
            .with_repository_config(self.config.clone())
            .with_auth(AuthContext::system())
    }

    pub async fn tx(&self, branch: &str) -> Box<dyn TransactionalContext> {
        let ctx = self.storage.begin_context().await.expect("begin");
        ctx.set_tenant_repo(TENANT, REPO).expect("tenant");
        ctx.set_branch(branch).expect("branch");
        ctx.set_actor("oracle").expect("actor");
        ctx.set_auth_context(AuthContext::system()).expect("auth");
        ctx.set_message("oracle op").expect("message");
        ctx.set_validate_schema(false).expect("schema");
        ctx
    }

    pub async fn head(&self, branch: &str) -> HLC {
        self.storage
            .branches()
            .get_branch(TENANT, REPO, branch)
            .await
            .expect("branch read")
            .expect("branch exists")
            .head
    }
}

/// Run `sql` and return its rows as JSON objects, or the error text.
pub async fn query(engine: &QueryEngine<Store>, sql: &str) -> Result<Vec<Value>, String> {
    let mut stream = engine.execute(sql).await.map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    while let Some(row) = stream.next().await {
        let row = row.map_err(|e| e.to_string())?;
        out.push(serde_json::to_value(&row.columns).map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// A string column of every row (`null` and missing become `""`).
pub fn column(rows: &[Value], name: &str) -> Vec<String> {
    rows.iter()
        .map(|r| {
            let v = r
                .get(name)
                .or_else(|| {
                    r.as_object().and_then(|o| {
                        o.iter()
                            .find(|(k, _)| k.rsplit('.').next() == Some(name))
                            .map(|(_, v)| v)
                    })
                })
                .cloned()
                .unwrap_or(Value::Null);
            match v {
                Value::String(s) => s,
                Value::Null => String::new(),
                other => other.to_string(),
            }
        })
        .collect()
}

/// Escape a literal for a single-quoted SQL string.
pub fn lit(s: &str) -> String {
    s.replace('\'', "''")
}

/// `__revision` as the SQL layer spells it.
pub fn at_rev(rev: &HLC) -> String {
    format!("__revision = '{rev}'")
}
