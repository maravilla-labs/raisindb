//! Row-level security for the SEARCH table functions: `FULLTEXT_SEARCH` and
//! `HYBRID_SEARCH`.
//!
//! Neither index is permission-aware. Tantivy answers "which nodes contain
//! these terms" and HNSW answers "which vectors are nearest" — neither answers
//! "which of them may this caller read". Both table functions fetch each hit
//! with `storage.nodes().get(...)` and emit the node's COMPLETE property bag,
//! so without a filter every SQL caller could read the full contents of nodes
//! they have no read permission for. That is a read-path authorization bypass,
//! not a ranking quirk — the same one that got `CYPHER(...)` withdrawn from SQL
//! (see the comment in `physical_plan/table_function.rs`).
//!
//! The policy is exactly the scan executors' policy, through the same shared
//! helper (`scan_executors::helpers::rls_filter_node_graph`):
//!
//! * an identified caller sees only nodes a permission allows, with that
//!   permission's FIELD filter applied to the property bag;
//! * an identified caller with no matching permission sees NOTHING (deny by
//!   default — [`fulltext_search_denies_when_no_permission_matches`]);
//! * `auth_context == None` is the system/internal caller and is unfiltered,
//!   the convention every scan executor and `GRAPH_TABLE` already use.
//!
//! Both table functions are covered because they are two separate fetch loops
//! over two separate index legs; a fix applied to one and not the other leaves
//! the hole open. `HYBRID_SEARCH` here runs its fulltext leg only (no HNSW
//! engine is attached), which is enough to drive its fusion + fetch path.

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

const TENANT: &str = "t_search_rls";
const REPO: &str = "r_search_rls";
const BRANCH: &str = "main";
const WS: &str = "default";
const DOC_TYPE: &str = "raisin:Document";

/// A term present in every seeded document and nowhere else, so the fulltext
/// leg returns all four and any shrinkage is RLS and not the query.
const TERM: &str = "zebracorn";

struct Fixture {
    storage: Arc<raisin_rocksdb::RocksDBStorage>,
    /// Writes the documents.
    writer: Arc<TantivyIndexingEngine>,
    /// Reads them back. A SECOND engine over the same directory, opened after
    /// the writer committed: its index cache is empty, so its first lookup
    /// loads the index from disk and its reader sees every committed document
    /// immediately. Reusing the writer's engine instead makes the test depend
    /// on `ReloadPolicy::OnCommitWithDelay` firing, which is a background
    /// reload with no completion signal — a stale reader then looks exactly
    /// like RLS having dropped the rows.
    reader: Arc<TantivyIndexingEngine>,
    _db: TempDir,
    _index: TempDir,
}

fn scope() -> StorageScope<'static> {
    StorageScope::new(TENANT, REPO, BRANCH, WS)
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
    catalog.register_workspace(WS.to_string());
    Arc::new(catalog)
}

/// A non-admin user that may only read nodes whose `owner` property is theirs.
fn owner_scoped_user(user: &str) -> AuthContext {
    permissions_for(
        user,
        vec![Permission::new("/**", vec![Operation::Read])
            .with_condition("node.owner == auth.user_id".to_string())],
    )
}

/// A non-admin user holding NO permission at all: every node must be denied.
fn user_without_permissions(user: &str) -> AuthContext {
    permissions_for(user, vec![])
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
    register_workspace(&fixture.storage, WS).await;
    fixture.seed().await;
    // Reopened only now, so it loads the committed index from disk rather than
    // waiting on a background reader reload that has no completion signal.
    fixture.reader = open();
    fixture.assert_index_visible().await;
    fixture
}

impl Fixture {
    /// Four documents sharing one search term. alice owns two, bob owns two.
    /// Each carries a `classified` property so field-level filtering is
    /// observable in the emitted property bag.
    async fn seed(&self) {
        let mut to_index: Vec<(Node, NodeIndexPlan)> = Vec::new();
        for (id, owner) in [
            ("alpha", "alice"),
            ("bravo", "bob"),
            ("charlie", "alice"),
            ("delta", "bob"),
        ] {
            let mut props = HashMap::new();
            props.insert(
                "owner".to_string(),
                PropertyValue::String(owner.to_string()),
            );
            props.insert(
                "content".to_string(),
                PropertyValue::String(format!("the {TERM} report for {id}")),
            );
            props.insert(
                "classified".to_string(),
                PropertyValue::String(format!("{id}-secret-payload")),
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
                    scope(),
                    node.clone(),
                    CreateNodeOptions {
                        validate_parent_allows_child: false,
                        validate_workspace_allows_type: false,
                        ..Default::default()
                    },
                )
                .await
                .unwrap_or_else(|e| panic!("create {id}: {e}"));

            let stored = self
                .storage
                .nodes()
                .get(scope(), id, None)
                .await
                .unwrap_or_else(|e| panic!("read back {id}: {e}"))
                .unwrap_or_else(|| panic!("read back {id}: missing"));

            // `legacy_index_all_strings` indexes every top-level String
            // property, which is what puts TERM in the index.
            to_index.push((
                stored,
                NodeIndexPlan {
                    node_type: DOC_TYPE.to_string(),
                    legacy_index_all_strings: true,
                    ..Default::default()
                },
            ));
        }

        // ONE writer, ONE commit. The per-node path (`do_index_node`) opens and
        // drops a fresh 50 MB IndexWriter per call, and four of those in a row
        // intermittently lost the LAST document permanently — no amount of
        // reopening the reader brought it back. The batch path is also what a
        // bulk reindex uses in production.
        let context = BatchIndexContext {
            tenant_id: TENANT.to_string(),
            repo_id: REPO.to_string(),
            branch: BRANCH.to_string(),
            workspace_id: WS.to_string(),
            default_language: "en".to_string(),
            supported_languages: vec!["en".to_string()],
        };
        self.writer
            .do_batch_index(&context, to_index, vec![])
            .expect("batch index");
    }

    /// Every "N rows" assertion below is only meaningful if all four documents
    /// are actually in the index, so establish that once, up front. A failure
    /// here is a broken fixture, not a broken filter.
    async fn assert_index_visible(&self) {
        assert_eq!(
            self.names(None, &fulltext_sql()).await,
            vec!["alpha", "bravo", "charlie", "delta"],
            "fixture: the four seeded documents are not all in the index",
        );
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
            // No auth context at all: the system/internal caller path.
            None => engine,
        }
    }

    async fn names(&self, auth: Option<AuthContext>, sql: &str) -> Vec<String> {
        let mut out: Vec<String> = rows(&self.query_engine(auth), sql)
            .await
            .iter()
            .filter_map(|r| column(r, "name"))
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

/// Columns are emitted qualified (`fulltext_search.name`), so match on suffix.
fn column(row: &Row, name: &str) -> Option<String> {
    row.columns.iter().find_map(|(key, value)| {
        let matches = key == name || key.ends_with(&format!(".{name}"));
        match (matches, value) {
            (true, PropertyValue::String(s)) => Some(s.clone()),
            _ => None,
        }
    })
}

/// Everything the row would hand a caller, rendered. The leak assertions scan
/// this rather than one named column: whatever shape the property bag reaches
/// the client in, an excluded value must not appear ANYWHERE in the row.
fn rendered(row: &Row) -> String {
    format!("{:?}", row.columns)
}

/// The scope is now REQUIRED and part of the query text, so every call here says
/// which corpus it searches. `'ALL READABLE'` is the spelling that means what
/// the two-argument form used to mean silently.
fn fulltext_sql() -> String {
    format!("SELECT * FROM FULLTEXT_SEARCH('{TERM}', 'en', workspaces => 'ALL READABLE')")
}

fn hybrid_sql() -> String {
    format!("SELECT * FROM HYBRID_SEARCH('{TERM}', 10, workspaces => 'ALL READABLE')")
}

// ---------------------------------------------------------------------------
// FULLTEXT_SEARCH
// ---------------------------------------------------------------------------

/// Baseline: without an identity nothing is filtered, so the index really does
/// return all four. Every "N rows" assertion below is measured against this.
#[tokio::test]
async fn fulltext_search_system_caller_sees_everything() {
    let f = fixture().await;
    assert_eq!(
        f.names(None, &fulltext_sql()).await,
        vec!["alpha", "bravo", "charlie", "delta"],
    );
}

#[tokio::test]
async fn fulltext_search_filters_rows_the_caller_may_not_read() {
    let f = fixture().await;
    assert_eq!(
        f.names(Some(owner_scoped_user("alice")), &fulltext_sql())
            .await,
        vec!["alpha", "charlie"],
        "bravo/delta are bob's; they must not reach an alice-scoped caller",
    );
}

/// Deny by default. A caller whose permission set matches nothing gets no rows
/// — not the unfiltered index.
#[tokio::test]
async fn fulltext_search_denies_when_no_permission_matches() {
    let f = fixture().await;
    assert!(f
        .names(Some(user_without_permissions("mallory")), &fulltext_sql())
        .await
        .is_empty());
}

/// The row carries the node's COMPLETE property bag, so the granting
/// permission's field filter has to apply to it as well. Row-level access is
/// not field-level access.
#[tokio::test]
async fn fulltext_search_applies_the_permissions_field_filter() {
    let f = fixture().await;
    let auth = permissions_for(
        "alice",
        vec![Permission::new("/**", vec![Operation::Read])
            .with_condition("node.owner == auth.user_id".to_string())
            .with_except_fields(vec!["classified".to_string()])],
    );

    let rows = rows(&f.query_engine(Some(auth)), &fulltext_sql()).await;
    assert_eq!(rows.len(), 2);
    for row in &rows {
        let rendered = rendered(row);
        assert!(
            !rendered.contains("secret-payload"),
            "excluded field leaked into the emitted row: {rendered}",
        );
        assert!(
            rendered.contains(TERM),
            "allowed field was dropped: {rendered}",
        );
    }
}

// ---------------------------------------------------------------------------
// HYBRID_SEARCH — a second, separate fetch loop over a second index leg.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn hybrid_search_system_caller_sees_everything() {
    let f = fixture().await;
    assert_eq!(
        f.names(None, &hybrid_sql()).await,
        vec!["alpha", "bravo", "charlie", "delta"],
    );
}

#[tokio::test]
async fn hybrid_search_filters_rows_the_caller_may_not_read() {
    let f = fixture().await;
    assert_eq!(
        f.names(Some(owner_scoped_user("alice")), &hybrid_sql())
            .await,
        vec!["alpha", "charlie"],
    );
}

#[tokio::test]
async fn hybrid_search_denies_when_no_permission_matches() {
    let f = fixture().await;
    assert!(f
        .names(Some(user_without_permissions("mallory")), &hybrid_sql())
        .await
        .is_empty());
}

#[tokio::test]
async fn hybrid_search_applies_the_permissions_field_filter() {
    let f = fixture().await;
    let auth = permissions_for(
        "alice",
        vec![Permission::new("/**", vec![Operation::Read])
            .with_condition("node.owner == auth.user_id".to_string())
            .with_except_fields(vec!["classified".to_string()])],
    );

    let rows = rows(&f.query_engine(Some(auth)), &hybrid_sql()).await;
    assert_eq!(rows.len(), 2);
    for row in &rows {
        let rendered = rendered(row);
        assert!(
            !rendered.contains("secret-payload"),
            "excluded field leaked into the emitted row: {rendered}",
        );
        assert!(
            rendered.contains(TERM),
            "allowed field was dropped: {rendered}",
        );
    }
}

// ---------------------------------------------------------------------------
// The scope argument, and the properties that must never drift
// ---------------------------------------------------------------------------

/// THE system-caller regression, and the reason it is written first.
///
/// `auth == None`, `is_system` and `is_system_admin` must each resolve
/// `'ALL READABLE'` to the whole catalog. Miss any one and every internal
/// search silently narrows: indexing jobs, MCP tools, agents running as system.
#[tokio::test]
async fn every_unfilterable_caller_resolves_all_readable_to_everything() {
    let f = fixture().await;
    let expected = vec!["alpha", "bravo", "charlie", "delta"];

    // No identity at all.
    assert_eq!(f.names(None, &hybrid_sql()).await, expected);

    // is_system.
    assert_eq!(
        f.names(Some(AuthContext::system()), &hybrid_sql()).await,
        expected,
        "a system caller must see the whole catalog"
    );

    // is_system_admin.
    let admin = AuthContext::for_user("root").with_permissions(ResolvedPermissions {
        user_id: "root".to_string(),
        email: None,
        direct_roles: vec![],
        group_roles: vec![],
        effective_roles: vec![],
        groups: vec![],
        permissions: vec![],
        is_system_admin: true,
        resolved_at: Some(std::time::Instant::now()),
    });
    assert_eq!(
        f.names(Some(admin), &hybrid_sql()).await,
        expected,
        "a system admin must see the whole catalog"
    );
}

/// `Empty` is not `All`. A caller who may read nothing gets zero rows -- not the
/// unfiltered index, which is what collapsing an empty set to "no filter" would
/// produce.
#[tokio::test]
async fn a_caller_who_may_read_nothing_gets_zero_rows() {
    let f = fixture().await;
    assert!(f
        .names(Some(user_without_permissions("mallory")), &hybrid_sql())
        .await
        .is_empty());
}

/// The two-argument form is now a hard error naming every migration, rather than
/// an undocumented repo-wide search.
#[tokio::test]
async fn the_implicit_scope_is_refused_and_the_error_names_the_fix() {
    let f = fixture().await;
    let err = f
        .query_engine(None)
        .execute(&format!("SELECT * FROM HYBRID_SEARCH('{TERM}', 10)"))
        .await
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    for fragment in ["workspaces =>", "ALL READABLE"] {
        assert!(err.contains(fragment), "error omits {fragment}: {err}");
    }
}

/// A named workspace that does not exist and one the caller may not read must
/// produce BYTE-IDENTICAL errors, or the function is an existence oracle.
#[tokio::test]
async fn scope_errors_are_not_an_existence_oracle() {
    let f = fixture().await;

    // "payroll" does not exist at all.
    let nonexistent = f
        .query_engine(Some(owner_scoped_user("alice")))
        .execute(&format!(
            "SELECT * FROM HYBRID_SEARCH('{TERM}', 10, workspaces => 'payroll')"
        ))
        .await
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();

    // "default" exists but this caller has no grant matching it.
    let unreadable_user = permissions_for(
        "carol",
        vec![Permission::new("/**", vec![Operation::Read]).with_workspace("elsewhere".to_string())],
    );
    let unreadable = f
        .query_engine(Some(unreadable_user))
        .execute(&format!(
            "SELECT * FROM HYBRID_SEARCH('{TERM}', 10, workspaces => 'default')"
        ))
        .await
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();

    assert!(nonexistent.contains("payroll"), "{nonexistent}");
    assert!(unreadable.contains("default"), "{unreadable}");
    assert_eq!(
        nonexistent.replace("payroll", "<ws>"),
        unreadable.replace("default", "<ws>"),
        "the two reasons must be indistinguishable to the caller",
    );
}

/// `WHERE` over a table function used to be DISCARDED by the plan builder --
/// `build_table_source` returned the plan without using `filter` and no `Filter`
/// node was ever created. The published book ships exactly this pattern.
#[tokio::test]
async fn where_over_a_table_function_actually_runs() {
    let f = fixture().await;
    let sql = format!(
        "SELECT * FROM HYBRID_SEARCH('{TERM}', 10, workspaces => 'ALL READABLE') \
         WHERE name = 'alpha'"
    );
    assert_eq!(f.names(None, &sql).await, vec!["alpha"]);

    let none = format!(
        "SELECT * FROM FULLTEXT_SEARCH('{TERM}', 'en', workspaces => 'ALL READABLE') \
         WHERE node_type = 'no:SuchType'"
    );
    assert!(f.names(None, &none).await.is_empty());
}

/// `limit` means rows DELIVERED, not candidates budgeted. Truncation used to
/// happen before the permission filter, so a restricted caller asking for 2 got
/// however many of the top 2 they happened to be allowed to see.
#[tokio::test]
async fn limit_is_honoured_after_permission_filtering() {
    let f = fixture().await;
    // alice owns exactly two of the four documents. Asking for two must yield
    // two, even though the unfiltered top-2 contains one of bob's.
    assert_eq!(
        f.names(
            Some(owner_scoped_user("alice")),
            &format!("SELECT * FROM HYBRID_SEARCH('{TERM}', 2, workspaces => 'ALL READABLE')")
        )
        .await,
        vec!["alpha", "charlie"],
    );
}

/// `KNN` used to analyse, plan, and then die at runtime with "Unsupported table
/// function: KNN" while the keyword help shipped a worked example. With no
/// vector index attached it must fail with a clear message -- never silently
/// answer with full-text results.
#[tokio::test]
async fn knn_is_implemented_and_says_what_it_needs() {
    let f = fixture().await;
    let err = f
        .query_engine(None)
        .execute(&format!(
            "SELECT * FROM KNN('{TERM}', 10, workspaces => 'ALL READABLE')"
        ))
        .await
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert!(
        !err.contains("Unsupported table function"),
        "KNN must be implemented: {err}"
    );
    assert!(err.contains("vector index"), "{err}");
}

/// `vector_weight => 0` must skip the vector leg ENTIRELY, including embedding
/// provider resolution -- otherwise a tenant with no embedder cannot run a
/// deliberately keyword-only hybrid query.
#[tokio::test]
async fn vector_weight_zero_needs_no_embedding_configuration() {
    let f = fixture().await;
    assert_eq!(
        f.names(
            None,
            &format!(
                "SELECT * FROM HYBRID_SEARCH('{TERM}', 10, \
                 workspaces => 'ALL READABLE', vector_weight => 0)"
            )
        )
        .await,
        vec!["alpha", "bravo", "charlie", "delta"],
    );
}

/// `FULLTEXT_SEARCH` and `HYBRID_SEARCH` return the SAME columns now. A
/// retriever that reranks by recency could not previously get `updated_at` out
/// of the hybrid function.
#[tokio::test]
async fn both_functions_return_the_same_column_set() {
    let f = fixture().await;
    let ft = rows(&f.query_engine(None), &fulltext_sql()).await;
    let hy = rows(&f.query_engine(None), &hybrid_sql()).await;

    let names = |row: &Row| -> Vec<String> {
        row.columns
            .keys()
            .map(|k| k.rsplit('.').next().unwrap_or(k).to_string())
            .collect()
    };
    assert_eq!(names(&ft[0]), names(&hy[0]));
    for wanted in [
        "node_id",
        "workspace_id",
        "name",
        "path",
        "node_type",
        "score",
        "fulltext_rank",
        "vector_rank",
        "vector_distance",
        "chunk_index",
        "revision",
        "created_at",
        "updated_at",
        "properties",
    ] {
        assert!(
            names(&hy[0]).iter().any(|n| n == wanted),
            "missing column {wanted} in {:?}",
            names(&hy[0])
        );
    }
}
