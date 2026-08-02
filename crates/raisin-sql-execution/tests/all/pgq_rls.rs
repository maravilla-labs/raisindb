//! Row-level security for `GRAPH_TABLE` — and, just as importantly, the exact
//! SHAPE of that security.
//!
//! GRAPH_TABLE is the only graph query language reachable from SQL, so it is
//! the one surface where graph RLS can be enforced. The chosen policy is
//! **ENDPOINTS ONLY**: a node is permission-checked when the `COLUMNS(...)`
//! clause RETURNS it. A node that is merely traversed on the way from one
//! endpoint to another is not checked and stays traversable.
//!
//! The load-bearing test here is [`intermediate_hop_may_be_invisible`]. A
//! stricter "prune the whole path" implementation — which is what you get by
//! accident if you filter every node present in a binding — passes every other
//! test in this file. Without that one test the wrong policy ships silently.
//!
//! Graph (all under `/users`, workspace `default`, `FOLLOWS` edges):
//!
//! ```text
//!   alice ──▶ mid ──▶ carol
//!     │              ▲
//!     ├──▶ secret    │
//!     └──────────────┘
//! ```
//!
//! `alice` and `carol` are owned by user "alice"; `mid` and `secret` are owned
//! by "bob". The RLS permission is the canonical differential shape:
//! read only what you own.

use futures::StreamExt;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::{Node, RelationRef};
use raisin_models::permissions::{Operation, Permission, ResolvedPermissions};
use raisin_sql_execution::{QueryEngine, Row, StaticCatalog};
use raisin_storage::{
    BranchRepository, CreateNodeOptions, NodeRepository, RelationRepository, Storage, StorageScope,
};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "t_pgq_rls";
const REPO: &str = "r_pgq_rls";
const BRANCH: &str = "main";
const WS: &str = "default";
const USER_TYPE: &str = "raisin:User";

/// Single hop; both endpoints projected.
const HOP_BOTH_ENDPOINTS: &str = "SELECT * FROM GRAPH_TABLE(MATCH (a:User)-[r:FOLLOWS]->(b:User) \
     COLUMNS (a.id AS src, b.id AS dst))";

/// Two hops; only the two ENDPOINTS are projected, `m` is a stepping stone.
const CHAIN_ENDPOINTS_ONLY: &str =
    "SELECT * FROM GRAPH_TABLE(MATCH (a:User)-[r1:FOLLOWS]->(m:User)-[r2:FOLLOWS]->(c:User) \
     COLUMNS (a.id AS src, c.id AS dst))";

/// Single hop; a QUALIFIED wildcard projects only `a`.
const HOP_QUALIFIED_WILDCARD: &str =
    "SELECT * FROM GRAPH_TABLE(MATCH (a:User)-[r:FOLLOWS]->(b:User) COLUMNS (a.*))";

/// Single hop; a BARE wildcard projects every variable, so both endpoints.
const HOP_BARE_WILDCARD: &str =
    "SELECT * FROM GRAPH_TABLE(MATCH (a:User)-[r:FOLLOWS]->(b:User) COLUMNS (*))";

async fn storage() -> (Arc<raisin_rocksdb::RocksDBStorage>, TempDir) {
    let tmp = TempDir::new().expect("temp dir");
    let storage = raisin_rocksdb::RocksDBStorage::new(tmp.path()).expect("storage");
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await;
    (Arc::new(storage), tmp)
}

fn scope() -> StorageScope<'static> {
    StorageScope::new(TENANT, REPO, BRANCH, WS)
}

fn catalog() -> Arc<StaticCatalog> {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    Arc::new(catalog)
}

/// A non-admin user that may only read nodes whose `owner` property is theirs.
fn owner_scoped_user(user: &str) -> AuthContext {
    AuthContext::for_user(user).with_permissions(ResolvedPermissions {
        user_id: user.to_string(),
        email: Some(format!("{user}@test.com")),
        direct_roles: vec![],
        group_roles: vec![],
        effective_roles: vec![],
        groups: vec![],
        permissions: vec![Permission::new("/**", vec![Operation::Read])
            .with_condition("node.owner == auth.user_id".to_string())],
        is_system_admin: false,
        resolved_at: Some(std::time::Instant::now()),
    })
}

fn engine(
    storage: &Arc<raisin_rocksdb::RocksDBStorage>,
    auth: Option<AuthContext>,
) -> QueryEngine<raisin_rocksdb::RocksDBStorage> {
    let engine = QueryEngine::new(
        storage.clone(),
        TENANT.to_string(),
        REPO.to_string(),
        BRANCH.to_string(),
    )
    .with_catalog(catalog());
    match auth {
        Some(auth) => engine.with_auth(auth),
        // No auth context at all: the system/internal caller path.
        None => engine,
    }
}

async fn seed(storage: &Arc<raisin_rocksdb::RocksDBStorage>) {
    let folder = Node {
        id: "users".to_string(),
        path: "/users".to_string(),
        name: "users".to_string(),
        parent: Some("/".to_string()),
        node_type: "raisin:Folder".to_string(),
        properties: HashMap::new(),
        ..Default::default()
    };
    create(storage, folder).await;

    for (id, owner) in [
        ("alice", "alice"),
        ("mid", "bob"),
        ("secret", "bob"),
        ("carol", "alice"),
    ] {
        let mut props = HashMap::new();
        props.insert(
            "owner".to_string(),
            PropertyValue::String(owner.to_string()),
        );
        props.insert("name".to_string(), PropertyValue::String(id.to_string()));
        create(
            storage,
            Node {
                id: id.to_string(),
                path: format!("/users/{id}"),
                name: id.to_string(),
                parent: Some("users".to_string()),
                node_type: USER_TYPE.to_string(),
                properties: props,
                ..Default::default()
            },
        )
        .await;
    }

    // alice ─▶ mid ─▶ carol, alice ─▶ secret, alice ─▶ carol
    for (from, to) in [
        ("alice", "mid"),
        ("mid", "carol"),
        ("alice", "secret"),
        ("alice", "carol"),
    ] {
        let rel = RelationRef::new(
            to.to_string(),
            WS.to_string(),
            USER_TYPE.to_string(),
            "FOLLOWS".to_string(),
            None,
        );
        storage
            .relations()
            .add_relation(scope(), from, USER_TYPE, rel)
            .await
            .unwrap_or_else(|e| panic!("relation {from}->{to}: {e}"));
    }
}

async fn create(storage: &Arc<raisin_rocksdb::RocksDBStorage>, node: Node) {
    let id = node.id.clone();
    storage
        .nodes()
        .create(
            scope(),
            node,
            CreateNodeOptions {
                validate_parent_allows_child: false,
                validate_workspace_allows_type: false,
                ..Default::default()
            },
        )
        .await
        .unwrap_or_else(|e| panic!("create {id}: {e}"));
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

/// Column values are emitted qualified (`graph_table.src`), so match on suffix.
fn column(row: &Row, name: &str) -> Option<String> {
    row.columns.iter().find_map(|(key, value)| {
        let matches = key == name || key.ends_with(&format!(".{name}"));
        match (matches, value) {
            (true, PropertyValue::String(s)) => Some(s.clone()),
            _ => None,
        }
    })
}

fn pairs(rows: &[Row]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = rows
        .iter()
        .map(|r| {
            (
                column(r, "src").unwrap_or_default(),
                column(r, "dst").unwrap_or_default(),
            )
        })
        .collect();
    out.sort();
    out
}

/// Row identity for the value-bearing (aliased) queries.
fn srcs(rows: &[Row]) -> Vec<String> {
    let mut out: Vec<String> = rows.iter().filter_map(|r| column(r, "src")).collect();
    out.sort();
    out
}

/// A caller with NO auth context is a system/internal caller and must be
/// completely unfiltered — that is how every scan executor behaves, and jobs,
/// replication and function bindings depend on it.
#[tokio::test]
async fn no_auth_context_is_unfiltered() {
    let (storage, _tmp) = storage().await;
    seed(&storage).await;
    let system = engine(&storage, None);

    assert_eq!(
        pairs(&rows(&system, HOP_BOTH_ENDPOINTS).await),
        vec![
            ("alice".into(), "carol".into()),
            ("alice".into(), "mid".into()),
            ("alice".into(), "secret".into()),
            ("mid".into(), "carol".into()),
        ],
        "a system caller must see every edge"
    );
}

/// A PROJECTED endpoint the caller cannot read removes the whole row.
#[tokio::test]
async fn denied_projected_endpoint_removes_the_row() {
    let (storage, _tmp) = storage().await;
    seed(&storage).await;
    let alice = engine(&storage, Some(owner_scoped_user("alice")));

    // alice→mid and alice→secret lose their target; mid→carol loses its source.
    assert_eq!(
        pairs(&rows(&alice, HOP_BOTH_ENDPOINTS).await),
        vec![("alice".into(), "carol".into())],
        "only the edge whose BOTH projected endpoints are readable survives"
    );
}

/// THE policy test. `MATCH (a)-[]->(m)-[]->(c) COLUMNS(a.id, c.id)` returns
/// `a` and `c` only; `m` is a stepping stone. A caller who may read `a` and `c`
/// gets the row even though `m` is invisible to them.
///
/// If this ever fails while the rest of the file passes, the implementation has
/// drifted to PRUNE-THE-WHOLE-PATH — a stricter policy that was explicitly not
/// chosen — most likely by filtering every node in the binding instead of only
/// the variables named by COLUMNS.
#[tokio::test]
async fn intermediate_hop_may_be_invisible() {
    let (storage, _tmp) = storage().await;
    seed(&storage).await;

    let system = engine(&storage, None);
    let baseline = pairs(&rows(&system, CHAIN_ENDPOINTS_ONLY).await);
    assert_eq!(
        baseline,
        vec![("alice".into(), "carol".into())],
        "the two-hop path alice→mid→carol must exist for an unfiltered caller"
    );

    let alice = engine(&storage, Some(owner_scoped_user("alice")));
    assert_eq!(
        pairs(&rows(&alice, CHAIN_ENDPOINTS_ONLY).await),
        baseline,
        "endpoints-only: `mid` is denied to alice but merely traversed, so the \
         row must STILL be returned"
    );
}

/// `COLUMNS(a.*)` projects only `a`; `b` is not returned and not checked.
///
/// Asserted on row COUNT rather than values: a wildcard COLUMNS clause expands
/// at runtime while the table function's SQL schema is derived statically, so
/// the outer `SELECT *` shows one placeholder column. That is a pre-existing
/// wildcard-projection limitation, unrelated to RLS; which ROWS survive is
/// exactly what this test is about.
#[tokio::test]
async fn qualified_wildcard_checks_only_its_own_variable() {
    let (storage, _tmp) = storage().await;
    seed(&storage).await;

    let system = engine(&storage, None);
    assert_eq!(
        rows(&system, HOP_QUALIFIED_WILDCARD).await.len(),
        4,
        "unfiltered baseline: all four edges"
    );

    let alice = engine(&storage, Some(owner_scoped_user("alice")));
    assert_eq!(
        rows(&alice, HOP_QUALIFIED_WILDCARD).await.len(),
        3,
        "every edge OUT of alice survives — mid and secret are targets, and \
         `a.*` does not project the target; only mid→carol is dropped, because \
         its projected `a` is denied"
    );
}

/// `COLUMNS(*)` expands every bound variable at runtime, so every node in the
/// row is projected — and therefore every node is checked.
#[tokio::test]
async fn bare_wildcard_checks_every_projected_variable() {
    let (storage, _tmp) = storage().await;
    seed(&storage).await;

    let system = engine(&storage, None);
    assert_eq!(
        rows(&system, HOP_BARE_WILDCARD).await.len(),
        4,
        "unfiltered baseline: all four edges"
    );

    let alice = engine(&storage, Some(owner_scoped_user("alice")));
    let filtered = rows(&alice, HOP_BARE_WILDCARD).await;
    assert_eq!(
        filtered.len(),
        1,
        "`*` projects both a and b, so only alice→carol survives — strictly \
         fewer rows than the same pattern with `a.*` (3), which is what pins \
         the runtime expansion of a bare wildcard"
    );
}

/// A denied endpoint must not be countable either: an aggregate is still a
/// value computed from rows the caller may not see.
#[tokio::test]
async fn aggregates_do_not_count_denied_endpoints() {
    let (storage, _tmp) = storage().await;
    seed(&storage).await;

    let sql = "SELECT * FROM GRAPH_TABLE(MATCH (a:User)-[r:FOLLOWS]->(b:User) \
               COLUMNS (COUNT(b.id) AS n))";

    let count = |rows: Vec<Row>| -> i64 {
        rows.first()
            .and_then(|r| {
                r.columns.iter().find_map(|(k, v)| {
                    if k == "n" || k.ends_with(".n") {
                        match v {
                            PropertyValue::Integer(i) => Some(*i),
                            PropertyValue::Float(f) => Some(*f as i64),
                            _ => None,
                        }
                    } else {
                        None
                    }
                })
            })
            .unwrap_or(-1)
    };

    let system = engine(&storage, None);
    assert_eq!(count(rows(&system, sql).await), 4, "unfiltered baseline");

    let alice = engine(&storage, Some(owner_scoped_user("alice")));
    // Only `b` is projected here, so only `b` is checked — the same
    // endpoints-only rule, applied to an aggregate. alice→carol and mid→carol
    // both end at a readable node; alice→mid and alice→secret do not.
    assert_eq!(
        count(rows(&alice, sql).await),
        2,
        "rows whose projected endpoint alice may not read are not counted"
    );
}
