//! Partial updates of one node, written concurrently, lose nothing.
//!
//! `UPDATE ... SET properties = properties || {...}` used to evaluate the
//! merge against a row read BEFORE the write and store the whole property
//! map: a write of another property that committed in between was silently
//! overwritten with its old value (reproduced: 2 of 10 parallel writes
//! survived). The statement is now a partial update applied at commit to the
//! node as stored then (`TransactionalContext::patch_node_properties`), so
//! per property the later write wins and nothing else is touched. No locks
//! are taken by callers, no version is checked and nothing is retried.

use futures::StreamExt;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql_execution::{QueryEngine, StaticCatalog};
use raisin_storage::node_patch::{NullMode, PatchTarget, PropertyPatch};
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::{
    BranchRepository, NodeRepository, NodeTypeRepository, RepoScope, Storage, StorageScope,
    WorkspaceRepository,
};
use std::sync::Arc;
use tempfile::TempDir;

const TENANT: &str = "t_partial";
const REPO: &str = "r_partial";
const BRANCH: &str = "main";
const WS: &str = "items";
const WRITERS: usize = 24;

type Rocks = raisin_rocksdb::RocksDBStorage;

async fn setup() -> (Arc<Rocks>, TempDir) {
    let dir = TempDir::new().expect("temp dir");
    let storage = Rocks::new(dir.path()).expect("storage");
    let _ = storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "test-user", None, None, false, false)
        .await;
    storage
        .workspaces()
        .put(
            RepoScope::new(TENANT, REPO),
            raisin_models::workspace::Workspace::new(WS.to_string()),
        )
        .await
        .expect("workspace");
    storage
        .node_types()
        .create(
            raisin_storage::BranchScope::new(TENANT, REPO, BRANCH),
            serde_json::from_value(serde_json::json!({ "name": "test:Job" })).expect("type"),
            raisin_storage::CommitMetadata {
                message: "test".to_string(),
                actor: "test".to_string(),
                is_system: true,
            },
        )
        .await
        .expect("node type");
    (Arc::new(storage), dir)
}

fn engine(storage: &Arc<Rocks>) -> QueryEngine<Rocks> {
    let mut catalog = StaticCatalog::default_nodes_schema();
    catalog.register_workspace(WS.to_string());
    QueryEngine::new(
        storage.clone(),
        TENANT.to_string(),
        REPO.to_string(),
        BRANCH.to_string(),
    )
    .with_catalog(Arc::new(catalog))
    .with_auth(raisin_models::auth::AuthContext::system())
}

async fn run(engine: &QueryEngine<Rocks>, sql: &str) -> usize {
    let mut stream = engine
        .execute(sql)
        .await
        .unwrap_or_else(|e| panic!("SQL failed [{sql}]: {e}"));
    let mut n = 0;
    while let Some(row) = stream.next().await {
        row.unwrap_or_else(|e| panic!("row error [{sql}]: {e}"));
        n += 1;
    }
    n
}

async fn stored(storage: &Rocks, path: &str) -> raisin_models::nodes::Node {
    storage
        .nodes()
        .get_by_path(StorageScope::new(TENANT, REPO, BRANCH, WS), path, None)
        .await
        .expect("read")
        .expect("node exists")
}

async fn create(storage: &Arc<Rocks>, path: &str) {
    run(
        &engine(storage),
        &format!(
            "INSERT INTO items (path, node_type, properties) VALUES \
             ('{path}', 'test:Job', '{{\"keep\":\"kept\",\"status\":\"new\"}}'::JSONB)"
        ),
    )
    .await;
}

/// User properties (engine stamps like `$supertypes` left out).
fn user_props(node: &raisin_models::nodes::Node) -> usize {
    node.properties
        .keys()
        .filter(|k| !k.starts_with('$'))
        .count()
}

fn text(node: &raisin_models::nodes::Node, key: &str) -> Option<String> {
    match node.properties.get(key) {
        Some(PropertyValue::String(s)) => Some(s.clone()),
        other => other.map(|v| format!("{v:?}")),
    }
}

/// Many parallel `properties || {...}` writes of DIFFERENT properties: every
/// one survives, and the property index matches every one of them.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn sql_merge_updates_of_different_properties_are_all_kept() {
    let (storage, _dir) = setup().await;
    create(&storage, "/job").await;

    let mut tasks = Vec::new();
    for i in 0..WRITERS {
        let storage = storage.clone();
        tasks.push(tokio::spawn(async move {
            run(
                &engine(&storage),
                &format!(
                    "UPDATE items SET properties = properties || '{{\"p{i}\":\"v{i}\"}}'::JSONB \
                     WHERE path = '/job'"
                ),
            )
            .await
        }));
    }
    for t in tasks {
        t.await.expect("join");
    }

    let node = stored(&storage, "/job").await;
    for i in 0..WRITERS {
        assert_eq!(
            text(&node, &format!("p{i}")).as_deref(),
            Some(format!("v{i}").as_str()),
            "property p{i} was lost: {:?}",
            node.properties.keys().collect::<Vec<_>>()
        );
    }
    assert_eq!(text(&node, "keep").as_deref(), Some("kept"));
    assert_eq!(text(&node, "status").as_deref(), Some("new"));

    // The indexes agree with the record.
    let e = engine(&storage);
    for i in [0, WRITERS / 2, WRITERS - 1] {
        let n = run(
            &e,
            &format!("SELECT path FROM items WHERE properties->>'p{i}'::String = 'v{i}'"),
        )
        .await;
        assert_eq!(n, 1, "property index misses p{i}");
    }
}

/// Parallel writes of the SAME property: one value wins (a value that was
/// written), and no other property is touched.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn sql_merge_updates_of_the_same_property_last_write_wins() {
    let (storage, _dir) = setup().await;
    create(&storage, "/job").await;

    let mut tasks = Vec::new();
    for i in 0..WRITERS {
        let storage = storage.clone();
        tasks.push(tokio::spawn(async move {
            run(
                &engine(&storage),
                &format!(
                    "UPDATE items SET properties = properties || '{{\"status\":\"s{i}\"}}'::JSONB \
                     WHERE path = '/job'"
                ),
            )
            .await
        }));
    }
    for t in tasks {
        t.await.expect("join");
    }

    let node = stored(&storage, "/job").await;
    let status = text(&node, "status").expect("status");
    assert!(
        (0..WRITERS).any(|i| status == format!("s{i}")),
        "status is {status}"
    );
    assert_eq!(text(&node, "keep").as_deref(), Some("kept"));
    assert_eq!(user_props(&node), 2, "{:?}", node.properties);
    // Exactly the winning value is indexed; the losers are not.
    let e = engine(&storage);
    let mut matched = 0;
    for i in 0..WRITERS {
        matched += run(
            &e,
            &format!("SELECT path FROM items WHERE properties->>'status'::String = 's{i}'"),
        )
        .await;
    }
    assert_eq!(matched, 1, "exactly one status value is indexed");
}

/// The SDK path (`TransactionalContext::patch_node_properties`, what
/// `nodes.update` / `nodes.patch` use), interleaved with SQL merges: nothing
/// is lost, and a `null` in remove mode deletes only that property.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn transactional_patches_and_sql_merges_interleave_without_loss() {
    let (storage, _dir) = setup().await;
    create(&storage, "/job").await;

    let mut tasks = Vec::new();
    for i in 0..WRITERS {
        let storage = storage.clone();
        tasks.push(tokio::spawn(async move {
            if i % 2 == 0 {
                let txn = storage.begin_context().await.expect("txn");
                txn.set_tenant_repo(TENANT, REPO).unwrap();
                txn.set_branch(BRANCH).unwrap();
                txn.set_actor("test").unwrap();
                txn.set_auth_context(raisin_models::auth::AuthContext::system())
                    .unwrap();
                let patch = serde_json::json!({ format!("p{i}"): format!("v{i}") });
                txn.patch_node_properties(
                    WS,
                    &PatchTarget::Path("/job".into()),
                    PropertyPatch::from_json(patch.as_object().unwrap(), NullMode::Remove),
                )
                .await
                .expect("patch");
                txn.commit().await.expect("commit");
            } else {
                run(
                    &engine(&storage),
                    &format!(
                        "UPDATE items SET properties = properties || '{{\"p{i}\":\"v{i}\"}}'::JSONB \
                         WHERE path = '/job'"
                    ),
                )
                .await;
            }
        }));
    }
    for t in tasks {
        t.await.expect("join");
    }

    let node = stored(&storage, "/job").await;
    for i in 0..WRITERS {
        assert_eq!(
            text(&node, &format!("p{i}")).as_deref(),
            Some(format!("v{i}").as_str()),
            "property p{i} was lost"
        );
    }

    // Remove one property; the others stay.
    let txn = storage.begin_context().await.expect("txn");
    txn.set_tenant_repo(TENANT, REPO).unwrap();
    txn.set_branch(BRANCH).unwrap();
    txn.set_auth_context(raisin_models::auth::AuthContext::system())
        .unwrap();
    txn.patch_node_properties(
        WS,
        &PatchTarget::Path("/job".into()),
        PropertyPatch::from_json(
            serde_json::json!({"status": null}).as_object().unwrap(),
            NullMode::Remove,
        ),
    )
    .await
    .expect("patch");
    txn.commit().await.expect("commit");
    let node = stored(&storage, "/job").await;
    assert!(!node.properties.contains_key("status"));
    assert_eq!(text(&node, "keep").as_deref(), Some("kept"));
    assert_eq!(user_props(&node), 1 + WRITERS);
}

/// `SET properties = {...}` without `properties ||` still REPLACES the map:
/// the explicit way to replace it.
#[tokio::test]
async fn sql_full_assignment_still_replaces_the_map() {
    let (storage, _dir) = setup().await;
    create(&storage, "/job").await;
    let e = engine(&storage);
    run(
        &e,
        "UPDATE items SET properties = '{\"only\":\"this\"}'::JSONB WHERE path = '/job'",
    )
    .await;
    let node = stored(&storage, "/job").await;
    assert_eq!(user_props(&node), 1, "{:?}", node.properties);
    assert_eq!(text(&node, "only").as_deref(), Some("this"));

    // And a merge on a missing node fails like before.
    let err = match e
        .execute("UPDATE items SET properties = properties || '{\"a\":\"b\"}'::JSONB WHERE path = '/nope'")
        .await
    {
        Err(err) => err.to_string(),
        Ok(mut s) => {
            let mut msg = None;
            while let Some(r) = s.next().await {
                if let Err(err) = r {
                    msg = Some(err.to_string());
                }
            }
            msg.expect("update of a missing node fails")
        }
    };
    assert!(err.contains("not found"), "{err}");
}
