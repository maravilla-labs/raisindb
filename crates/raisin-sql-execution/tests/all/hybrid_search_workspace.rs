//! `HYBRID_SEARCH` must fetch each hit from the workspace the hit came from.
//!
//! Both index legs report the workspace of every hit
//! (`FullTextSearchResult::workspace_id`, `SearchResult::workspace_id`), but the
//! fusion step used to key its ranking maps on `node_id` alone and then fetch
//! every surviving hit from a hardcoded literal:
//!
//! ```ignore
//! let workspace_id = "default";
//! storage.nodes().get(StorageScope::new(&tenant, &repo, &branch, workspace_id), ...)
//! ```
//!
//! In a repo whose workspaces are named anything else the `get` returns `None`
//! for every hit, the `if let Some(node)` is false, and HYBRID_SEARCH answers
//! HTTP 200 with ZERO rows — rankings computed, `truncate(limit)` already
//! applied, no backfill and no error. `FULLTEXT_SEARCH` never had the bug: it
//! fetches with `&result.workspace_id`.
//!
//! The workspace is also the scope row-level security is evaluated in
//! (`PermissionScope::new(workspace_id, branch)`), so the literal meant a hit
//! was checked against the wrong workspace's permissions as well.
//!
//! These tests deliberately use NO workspace named "default", so a regression
//! to any hardcoded literal fails them. The vector leg is not attached (no HNSW
//! engine), which is enough to drive fusion + the fetch loop; the fulltext leg
//! alone decides the ranking.

use futures::StreamExt;
use raisin_indexer::{BatchIndexContext, TantivyIndexingEngine};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::permissions::{Operation, Permission, ResolvedPermissions};
use raisin_sql_execution::{QueryEngine, Row, StaticCatalog};
use raisin_storage::fulltext::NodeIndexPlan;
use raisin_storage::{BranchRepository, CreateNodeOptions, NodeRepository, Storage, StorageScope};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "t_hybrid_ws";
const REPO: &str = "r_hybrid_ws";
const BRANCH: &str = "main";
const DOC_TYPE: &str = "raisin:Document";

/// Two workspaces, NEITHER of them "default".
const WS_LIBRARY: &str = "library";
const WS_STORIES: &str = "stories";

/// Present in every seeded document and nowhere else.
const TERM: &str = "zebracorn";

struct Fixture {
    storage: Arc<raisin_rocksdb::RocksDBStorage>,
    writer: Arc<TantivyIndexingEngine>,
    /// A SECOND engine over the same directory, opened after the writer
    /// committed, so its first lookup loads the index from disk instead of
    /// waiting on a background reader reload with no completion signal.
    reader: Arc<TantivyIndexingEngine>,
    _db: TempDir,
    _index: TempDir,
}

fn scope(ws: &str) -> StorageScope<'_> {
    StorageScope::new(TENANT, REPO, BRANCH, ws)
}

/// Register a workspace in STORAGE, not only in the SQL catalog.
///
/// `workspaces => ...` resolves against `storage.workspaces().list(...)`, which
/// is what a real deployment has and what makes `'ALL READABLE'` mean something
/// auditable. A fixture that only registers the name with the planner leaves the
/// search universe empty.
async fn register_workspace(storage: &raisin_rocksdb::RocksDBStorage, name: &str) {
    use raisin_storage::{RepoScope, WorkspaceRepository};
    storage
        .workspaces()
        .put(
            RepoScope::new(TENANT, REPO),
            raisin_models::workspace::Workspace {
                name: name.to_string(),
                description: None,
                allowed_node_types: vec![DOC_TYPE.to_string()],
                allowed_root_node_types: vec![DOC_TYPE.to_string()],
                depends_on: Vec::new(),
                initial_structure: None,
                created_at: raisin_models::StorageTimestamp::now(),
                updated_at: None,
                config: Default::default(),
            },
        )
        .await
        .unwrap_or_else(|e| panic!("register workspace {name}: {e}"));
}

fn catalog() -> Arc<StaticCatalog> {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS_LIBRARY.to_string());
    catalog.register_workspace(WS_STORIES.to_string());
    Arc::new(catalog)
}

/// Documents, by workspace. Two per workspace so a rank list has something to
/// order and a workspace filter has something to exclude.
fn seed_plan() -> Vec<(&'static str, &'static str)> {
    vec![
        (WS_LIBRARY, "alpha"),
        (WS_LIBRARY, "bravo"),
        (WS_STORIES, "charlie"),
        (WS_STORIES, "delta"),
    ]
}

async fn fixture() -> Fixture {
    let db = TempDir::new().expect("db temp dir");
    let index = TempDir::new().expect("index temp dir");
    let storage = Arc::new(raisin_rocksdb::RocksDBStorage::new(db.path()).expect("storage"));
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await;

    let index_path = index.path().to_path_buf();
    let open = || {
        Arc::new(
            TantivyIndexingEngine::new(index_path.clone(), 64 * 1024 * 1024)
                .expect("tantivy engine"),
        )
    };

    let writer = open();
    let mut fixture = Fixture {
        storage,
        writer,
        reader: open(),
        _db: db,
        _index: index,
    };
    for ws in [WS_LIBRARY, WS_STORIES] {
        register_workspace(&fixture.storage, ws).await;
    }
    fixture.seed().await;
    fixture.reader = open();
    fixture
}

impl Fixture {
    async fn seed(&self) {
        // One batch per workspace: BatchIndexContext carries the workspace, and
        // it is that value the index stores and later reports on every hit.
        for ws in [WS_LIBRARY, WS_STORIES] {
            let mut to_index: Vec<(Node, NodeIndexPlan)> = Vec::new();

            for (node_ws, id) in seed_plan() {
                if node_ws != ws {
                    continue;
                }
                let mut props = HashMap::new();
                props.insert(
                    "content".to_string(),
                    PropertyValue::String(format!("the {TERM} report for {id}")),
                );

                let node = Node {
                    id: id.to_string(),
                    path: format!("/docs/{id}"),
                    name: id.to_string(),
                    parent: Some("/".to_string()),
                    node_type: DOC_TYPE.to_string(),
                    properties: props,
                    ..Default::default()
                };

                self.storage
                    .nodes()
                    .create(
                        scope(ws),
                        node.clone(),
                        CreateNodeOptions {
                            validate_parent_allows_child: false,
                            validate_workspace_allows_type: false,
                            ..Default::default()
                        },
                    )
                    .await
                    .unwrap_or_else(|e| panic!("create {ws}/{id}: {e}"));

                let stored = self
                    .storage
                    .nodes()
                    .get(scope(ws), id, None)
                    .await
                    .unwrap_or_else(|e| panic!("read back {ws}/{id}: {e}"))
                    .unwrap_or_else(|| panic!("read back {ws}/{id}: missing"));

                to_index.push((
                    stored,
                    NodeIndexPlan {
                        node_type: DOC_TYPE.to_string(),
                        legacy_index_all_strings: true,
                        ..Default::default()
                    },
                ));
            }

            let context = BatchIndexContext {
                tenant_id: TENANT.to_string(),
                repo_id: REPO.to_string(),
                branch: BRANCH.to_string(),
                workspace_id: ws.to_string(),
                default_language: "en".to_string(),
                supported_languages: vec!["en".to_string()],
            };
            self.writer
                .do_batch_index(&context, to_index, vec![])
                .unwrap_or_else(|e| panic!("batch index {ws}: {e}"));
        }
    }

    fn query_engine(
        &self,
        auth: Option<AuthContext>,
    ) -> QueryEngine<raisin_rocksdb::RocksDBStorage> {
        let engine = QueryEngine::new(
            self.storage.clone(),
            TENANT.to_string(),
            REPO.to_string(),
            BRANCH.to_string(),
        )
        .with_catalog(catalog())
        .with_indexing_engine(self.reader.clone());
        match auth {
            Some(auth) => engine.with_auth(auth),
            None => engine,
        }
    }

    async fn hits(&self, auth: Option<AuthContext>, sql: &str) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = rows(&self.query_engine(auth), sql)
            .await
            .iter()
            .map(|r| {
                (
                    column(r, "workspace_id").unwrap_or_default(),
                    column(r, "name").unwrap_or_default(),
                )
            })
            .collect();
        out.sort();
        out
    }
}

async fn rows(engine: &QueryEngine<raisin_rocksdb::RocksDBStorage>, sql: &str) -> Vec<Row> {
    let mut stream = engine
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("query failed: {e}\nSQL: {sql}"));
    let mut out = Vec::new();
    while let Some(row) = stream.next().await {
        out.push(row.unwrap_or_else(|e| panic!("row error: {e}\nSQL: {sql}")));
    }
    out
}

/// Columns are emitted qualified (`hybrid_search.name`), so match on suffix.
fn column(row: &Row, name: &str) -> Option<String> {
    row.columns.iter().find_map(|(key, value)| {
        let matches = key == name || key.ends_with(&format!(".{name}"));
        match (matches, value) {
            (true, PropertyValue::String(s)) => Some(s.clone()),
            _ => None,
        }
    })
}

fn permissions_for(user: &str, permissions: Vec<Permission>) -> AuthContext {
    AuthContext::for_user(user).with_permissions(ResolvedPermissions {
        user_id: user.to_string(),
        email: Some(format!("{user}@test.com")),
        direct_roles: vec![],
        group_roles: vec![],
        effective_roles: vec![],
        groups: vec![],
        permissions,
        is_system_admin: false,
        resolved_at: Some(std::time::Instant::now()),
    })
}

// ---------------------------------------------------------------------------

/// The fixture is only meaningful if the documents really are in the index, and
/// `FULLTEXT_SEARCH` is the leg that was always correct — so it establishes the
/// baseline the hybrid path has to match.
#[tokio::test]
async fn fulltext_search_finds_both_non_default_workspaces() {
    let f = fixture().await;
    assert_eq!(
        f.hits(
            None,
            &format!("SELECT * FROM FULLTEXT_SEARCH('{TERM}', 'en', workspaces => 'ALL READABLE')")
        )
        .await,
        vec![
            ("library".to_string(), "alpha".to_string()),
            ("library".to_string(), "bravo".to_string()),
            ("stories".to_string(), "charlie".to_string()),
            ("stories".to_string(), "delta".to_string()),
        ],
    );
}

/// The regression. Before the fix this returned ZERO rows — every hit was
/// fetched from a workspace named "default", which does not exist here.
#[tokio::test]
async fn hybrid_search_returns_hits_from_non_default_workspaces() {
    let f = fixture().await;
    assert_eq!(
        f.hits(
            None,
            &format!("SELECT * FROM HYBRID_SEARCH('{TERM}', 10, workspaces => 'ALL READABLE')")
        )
        .await,
        vec![
            ("library".to_string(), "alpha".to_string()),
            ("library".to_string(), "bravo".to_string()),
            ("stories".to_string(), "charlie".to_string()),
            ("stories".to_string(), "delta".to_string()),
        ],
        "HYBRID_SEARCH must fetch each hit from ITS OWN workspace",
    );
}

/// The emitted `workspace_id` column must be the hit's real workspace, not a
/// constant — a caller uses it to address the node it just found.
#[tokio::test]
async fn hybrid_search_reports_the_real_workspace_per_row() {
    let f = fixture().await;
    let reported: Vec<String> = f
        .hits(
            None,
            &format!("SELECT * FROM HYBRID_SEARCH('{TERM}', 10, workspaces => 'ALL READABLE')"),
        )
        .await
        .into_iter()
        .map(|(ws, _)| ws)
        .collect();
    assert!(
        reported.iter().any(|ws| ws == "library") && reported.iter().any(|ws| ws == "stories"),
        "both workspaces must appear in workspace_id, got {reported:?}",
    );
    assert!(
        !reported.iter().any(|ws| ws == "default"),
        "no hit may report the old hardcoded literal, got {reported:?}",
    );
}

/// The optional third argument scopes BOTH legs. Without it there is no way to
/// ask for one workspace: the plan's `workspace` field is always `None` for a
/// table function (`from_clause::analyze_table_function`).
#[tokio::test]
async fn hybrid_search_workspace_argument_scopes_the_search() {
    let f = fixture().await;
    assert_eq!(
        f.hits(
            None,
            &format!("SELECT * FROM HYBRID_SEARCH('{TERM}', 10, 'library')")
        )
        .await,
        vec![
            ("library".to_string(), "alpha".to_string()),
            ("library".to_string(), "bravo".to_string()),
        ],
        "a named workspace must exclude every other workspace's hits",
    );
}

/// Row-level security is evaluated in `PermissionScope::new(workspace_id, …)`,
/// so the workspace a hit is checked in must be the workspace it came from. A
/// permission granted in `library` must not admit a `stories` node, and must
/// still admit its own — the hardcoded literal made both of those wrong at once.
#[tokio::test]
async fn hybrid_search_applies_rls_in_the_hits_own_workspace() {
    let f = fixture().await;
    let library_only = permissions_for(
        "reader",
        vec![Permission::new("/**", vec![Operation::Read]).with_workspace(WS_LIBRARY.to_string())],
    );
    assert_eq!(
        f.hits(
            Some(library_only),
            &format!("SELECT * FROM HYBRID_SEARCH('{TERM}', 10, workspaces => 'ALL READABLE')")
        )
        .await,
        vec![
            ("library".to_string(), "alpha".to_string()),
            ("library".to_string(), "bravo".to_string()),
        ],
        "a library-scoped permission must admit library hits and only those",
    );
}
