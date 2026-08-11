//! In-process SQL read throughput baseline: scans across buckets + compound-index
//! keyset pagination. Complements `raisin-core`'s throughput_baseline (writes).
//!
//! Run:
//!   BENCH_N=20000 cargo test --release -p raisin-sql-execution --test throughput_sql -- --ignored --nocapture
//!
//! Data shape: N items spread across hierarchical buckets (≤ cap children each),
//! each with `cat` (one of 10 categories) and `seq` (zero-padded so text ordering
//! == numeric ordering). The node type carries a compound index on (cat, seq).
//! Reads go through SQL across ALL buckets (no bucket dependency on the read side).

use futures::StreamExt;
use std::sync::Arc;
use std::time::Instant;

use raisin_core::NodeService;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::schema::{
    CompoundColumnType, CompoundIndexColumn, CompoundIndexDefinition, PropertyType,
    PropertyValueSchema,
};
use raisin_models::nodes::types::NodeType;
use raisin_models::nodes::{properties::PropertyValue, Node};
use raisin_models::permissions::{Operation, Permission, ResolvedPermissions};
use raisin_models::workspace::Workspace;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::{
    BranchRepository, BranchScope, CommitMetadata, NodeTypeRepository, RepoScope,
    RepositoryManagementRepository, Storage, WorkspaceRepository,
};

const TENANT: &str = "default";
const REPO: &str = "default";
const BRANCH: &str = "main";
const WS: &str = "default";
const ITEM_TYPE: &str = "bench:Item";
const CATS: usize = 10;
const BUCKET_CAP: usize = 500;

fn bench_n() -> usize {
    std::env::var("BENCH_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10000)
}

fn make_node_type(
    name: &str,
    properties: Vec<PropertyValueSchema>,
    compound_indexes: Option<Vec<CompoundIndexDefinition>>,
) -> NodeType {
    NodeType {
        id: Some(name.to_string()),
        strict: Some(false),
        name: name.to_string(),
        extends: None,
        mixins: Vec::new(),
        overrides: None,
        description: None,
        icon: None,
        version: Some(1),
        properties: Some(properties),
        allowed_children: Vec::new(),
        required_nodes: Vec::new(),
        initial_structure: None,
        versionable: Some(true),
        publishable: Some(false),
        auditable: Some(false),
        indexable: Some(true),
        index_types: None,
        created_at: Some(chrono::Utc::now()),
        updated_at: None,
        published_at: None,
        published_by: None,
        previous_version: None,
        compound_indexes,
        is_mixin: None,
    }
}

fn prop_schema(name: &str, ty: PropertyType) -> PropertyValueSchema {
    PropertyValueSchema {
        name: Some(name.to_string()),
        property_type: ty,
        required: Some(false),
        unique: Some(false),
        default: None,
        constraints: None,
        structure: None,
        items: None,
        value: None,
        meta: None,
        is_translatable: None,
        allow_additional_properties: None,
        index: None,
        spatial: None,
        encrypted: None,
    }
}

async fn bootstrap() -> (Arc<raisin_rocksdb::RocksDBStorage>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(raisin_rocksdb::RocksDBStorage::new(dir.path()).unwrap());
    storage
        .repository_management()
        .create_repository(TENANT, REPO, raisin_context::RepositoryConfig::default())
        .await
        .unwrap();
    storage
        .workspaces()
        .put(RepoScope::new(TENANT, REPO), Workspace::new(WS.to_string()))
        .await
        .unwrap();
    storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "bench", None, None, false, false)
        .await
        .unwrap();

    // Folder type for buckets + item type with a compound index on (cat, seq).
    let folder = make_node_type("bench:Folder", vec![], None);
    let item = make_node_type(
        ITEM_TYPE,
        vec![
            prop_schema("cat", PropertyType::String),
            prop_schema("seq", PropertyType::String),
        ],
        Some(vec![CompoundIndexDefinition {
            name: "cat_seq".to_string(),
            columns: vec![
                CompoundIndexColumn {
                    property: "cat".to_string(),
                    ascending: None,
                    column_type: CompoundColumnType::String,
                },
                CompoundIndexColumn {
                    property: "seq".to_string(),
                    ascending: Some(true),
                    column_type: CompoundColumnType::String,
                },
            ],
            has_order_column: true,
        }]),
    );
    for nt in [folder, item] {
        storage
            .node_types()
            .upsert(
                BranchScope::new(TENANT, REPO, BRANCH),
                nt,
                CommitMetadata::system("bench types"),
            )
            .await
            .unwrap();
    }
    (storage, dir)
}

fn item(name: &str, cat: &str, seq: usize) -> Node {
    let mut props = std::collections::HashMap::new();
    props.insert("cat".to_string(), PropertyValue::String(cat.to_string()));
    // Zero-pad so text ordering matches numeric ordering (keyset pagination).
    props.insert(
        "seq".to_string(),
        PropertyValue::String(format!("{seq:09}")),
    );
    Node {
        id: String::new(),
        name: name.to_string(),
        path: String::new(),
        node_type: ITEM_TYPE.to_string(),
        properties: props,
        version: 1,
        ..Default::default()
    }
}

fn folder(name: &str) -> Node {
    Node {
        id: String::new(),
        name: name.to_string(),
        path: String::new(),
        node_type: "bench:Folder".to_string(),
        version: 1,
        ..Default::default()
    }
}

fn rls_user(uid: &str) -> AuthContext {
    AuthContext::for_user(uid.to_string())
        .with_permissions(ResolvedPermissions {
            user_id: uid.to_string(),
            email: None,
            direct_roles: vec![],
            group_roles: vec![],
            effective_roles: vec![],
            groups: vec![],
            permissions: vec![Permission::new(
                "**",
                vec![Operation::Read, Operation::Create],
            )],
            is_system_admin: false,
            resolved_at: None,
        })
        .with_local_user_id(uid.to_string())
}

fn engine(
    storage: &Arc<raisin_rocksdb::RocksDBStorage>,
    auth: AuthContext,
) -> QueryEngine<raisin_rocksdb::RocksDBStorage> {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    QueryEngine::new(
        storage.clone(),
        TENANT.to_string(),
        REPO.to_string(),
        BRANCH.to_string(),
    )
    .with_catalog(Arc::new(catalog))
    .with_auth(auth)
}

/// Execute a query, count rows.
async fn count_rows(eng: &QueryEngine<raisin_rocksdb::RocksDBStorage>, sql: &str) -> usize {
    let mut stream = eng
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("query failed [{sql}]: {e}"));
    let mut n = 0;
    while let Some(r) = stream.next().await {
        r.unwrap_or_else(|e| panic!("row error [{sql}]: {e}"));
        n += 1;
    }
    n
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "SQL read throughput baseline; run with --ignored --nocapture"]
async fn throughput_sql() {
    let n = bench_n();
    println!("\n=== RaisinDB in-process SQL read baseline (RocksDB, real QueryEngine) ===");
    println!("BENCH_N = {n}, buckets ≤{BUCKET_CAP}/bucket, {CATS} categories\n");

    // ---- Seed N items across hierarchical buckets ----
    let (storage, _dir) = bootstrap().await;
    let seeder = NodeService::new(storage.clone()).with_auth(AuthContext::system());
    let seed_start = Instant::now();
    let mut cur = usize::MAX;
    for i in 0..n {
        let b = i / BUCKET_CAP;
        if b != cur {
            seeder
                .add_node("/", folder(&format!("bk{b}")))
                .await
                .unwrap();
            cur = b;
        }
        let cat = format!("c{}", i % CATS);
        seeder
            .add_node(&format!("/bk{b}"), item(&format!("i{i}"), &cat, i))
            .await
            .unwrap();
    }
    println!(
        "seeded {} items in {:.2}s ({:.0}/s)\n",
        n,
        seed_start.elapsed().as_secs_f64(),
        n as f64 / seed_start.elapsed().as_secs_f64()
    );

    let sys = engine(&storage, AuthContext::system());
    let usr = engine(&storage, rls_user("u1"));

    // ---- Full scan across ALL buckets (RLS off) ----
    {
        let start = Instant::now();
        let rows = count_rows(&sys, &format!("SELECT id FROM {WS}")).await;
        let e = start.elapsed();
        println!(
            "full scan (RLS off)            {:>7} rows in {:>7.3}s => {:>10.0} rows/sec",
            rows,
            e.as_secs_f64(),
            rows as f64 / e.as_secs_f64()
        );
    }

    // ---- Full scan across ALL buckets (RLS on) ----
    {
        let start = Instant::now();
        let rows = count_rows(&usr, &format!("SELECT id FROM {WS}")).await;
        let e = start.elapsed();
        println!(
            "full scan (RLS on)             {:>7} rows in {:>7.3}s => {:>10.0} rows/sec",
            rows,
            e.as_secs_f64(),
            rows as f64 / e.as_secs_f64()
        );
    }

    // ---- Equality filter on indexed property. BARE form (no ::String cast) so
    //      the planner can route to the property/compound index; the cast form is
    //      a verbatim row-level filter that always full-scans. ----
    for (label, sql) in [
        (
            "filter cat='c3' BARE (index)",
            format!("SELECT id FROM {WS} WHERE properties->>'cat' = 'c3'"),
        ),
        (
            "filter cat='c3' ::String (scan)",
            format!("SELECT id FROM {WS} WHERE properties->>'cat'::String = 'c3'"),
        ),
    ] {
        let start = Instant::now();
        let rows = count_rows(&sys, &sql).await;
        let e = start.elapsed();
        println!(
            "{label:<32} {:>7} rows in {:>7.3}s => {:>10.0} rows/sec",
            rows,
            e.as_secs_f64(),
            rows as f64 / e.as_secs_f64()
        );
    }

    // ---- Paginate a category with ORDER BY + LIMIT/OFFSET, pages of 100 ----
    // NOTE: keyset via a JSON range predicate (`properties->>'seq' > x`) does NOT
    // route to the compound index (only equality does) and returns incomplete
    // pages, so we use LIMIT/OFFSET which paginates correctly. first vs last page
    // latency shows the OFFSET cost.
    {
        let page = 100usize;
        let mut pages = 0usize;
        let mut total = 0usize;
        let mut first_page_us = 0u128;
        let mut last_page_us = 0u128;
        let start = Instant::now();
        let mut offset = 0usize;
        loop {
            let sql = format!(
                "SELECT id FROM {WS} WHERE properties->>'cat' = 'c3' \
                 ORDER BY properties->>'seq' LIMIT {page} OFFSET {offset}"
            );
            let t = Instant::now();
            let got = count_rows(&sys, &sql).await;
            let took = t.elapsed().as_micros();
            if pages == 0 {
                first_page_us = took;
            }
            last_page_us = took;
            total += got;
            pages += 1;
            offset += page;
            if got < page {
                break;
            }
        }
        let e = start.elapsed();
        println!(
            "paginate cat='c3' (/100)       {:>7} rows in {:>4} pages, {:>7.3}s => {:>8.1} pages/sec (first {}µs, last {}µs)",
            total,
            pages,
            e.as_secs_f64(),
            pages as f64 / e.as_secs_f64(),
            first_page_us,
            last_page_us
        );
    }

    println!();
}
