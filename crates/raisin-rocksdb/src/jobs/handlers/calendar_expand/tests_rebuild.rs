//! Projection rebuild against a real RocksDB tempdir.

use std::sync::Arc;

use chrono::Utc;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};

use super::guard;
use super::rebuild::rebuild_locked;
use super::tests_fixtures::{exception_node, s, weekly_master, TENANT, WS};
use super::{format_utc, EVENT_TYPE};
use crate::RocksDBStorage;

const REPO: &str = "cal-expand-test";

struct Env {
    _dir: tempfile::TempDir,
    storage: Arc<RocksDBStorage>,
}

async fn setup() -> Env {
    use raisin_storage::{
        BranchRepository, RepoScope, RepositoryManagementRepository, Storage, WorkspaceRepository,
    };
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(RocksDBStorage::new(dir.path()).unwrap());
    storage
        .repository_management()
        .create_repository(TENANT, REPO, raisin_context::RepositoryConfig::default())
        .await
        .unwrap();
    storage
        .branches()
        .create_branch(TENANT, REPO, "main", "test", None, None, false, false)
        .await
        .unwrap();
    raisin_core::nodetype_init::init_repository_nodetypes(storage.clone(), TENANT, REPO, "main")
        .await
        .unwrap();
    for ws in raisin_core::workspace_init::load_global_workspaces() {
        storage
            .workspaces()
            .put(RepoScope::new(TENANT, REPO), ws)
            .await
            .unwrap();
    }
    Env { _dir: dir, storage }
}

async fn write_nodes(env: &Env, nodes: &[Node]) {
    let tx = env.storage.begin_context().await.unwrap();
    tx.set_tenant_repo(TENANT, REPO).unwrap();
    tx.set_branch("main").unwrap();
    tx.set_actor("test").unwrap();
    tx.set_auth_context(raisin_models::auth::AuthContext::system())
        .unwrap();
    tx.set_message("test fixture").unwrap();
    for node in nodes {
        tx.upsert_deep_node(WS, node, "raisin:Folder")
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();
}

async fn delete_node(env: &Env, id: &str) {
    let tx = env.storage.begin_context().await.unwrap();
    tx.set_tenant_repo(TENANT, REPO).unwrap();
    tx.set_branch("main").unwrap();
    tx.set_actor("test").unwrap();
    tx.set_auth_context(raisin_models::auth::AuthContext::system())
        .unwrap();
    tx.set_message("test delete").unwrap();
    tx.delete_node(WS, id).await.unwrap();
    tx.commit().await.unwrap();
}

async fn projected(env: &Env) -> Vec<String> {
    let tx = env.storage.begin_context().await.unwrap();
    tx.set_tenant_repo(TENANT, REPO).unwrap();
    tx.set_branch("main").unwrap();
    let mut paths: Vec<String> = tx
        .scan_nodes(WS)
        .await
        .unwrap()
        .into_iter()
        .filter(|n| n.node_type == EVENT_TYPE && guard::is_derived_occurrence(n))
        .map(|n| n.path)
        .collect();
    paths.sort();
    paths
}

async fn rebuild(env: &Env) -> super::RebuildSummary {
    rebuild_locked(&env.storage, TENANT, REPO, "main", WS)
        .await
        .unwrap()
}

/// A master anchored near "now" so the rolling window covers it whatever day
/// the suite runs on.
fn rolling_master(id: &str, external: &str) -> Node {
    let anchor = Utc::now() + chrono::Duration::days(3);
    let mut node = weekly_master(id, Some(external));
    node.properties.insert(
        "recurrence".into(),
        PropertyValue::Array(vec![s("RRULE:FREQ=DAILY;COUNT=4")]),
    );
    node.properties
        .insert("start_utc".into(), s(&format_utc(anchor)));
    node.properties.insert(
        "end_utc".into(),
        s(&format_utc(anchor + chrono::Duration::hours(1))),
    );
    node
}

#[tokio::test]
async fn rebuild_materializes_prunes_and_is_idempotent() {
    let env = setup().await;
    let master = rolling_master("m-roll", "ext-roll");
    write_nodes(&env, std::slice::from_ref(&master)).await;

    let first = rebuild(&env).await;
    assert_eq!(first.masters, 1);
    assert_eq!(first.written, 4, "one node per instance");
    assert_eq!(projected(&env).await.len(), 4);

    // Second pass finds everything already right and writes NOTHING. Without
    // this the projection would mint four revisions every 15 minutes forever.
    let second = rebuild(&env).await;
    assert_eq!(second.written, 0);
    assert_eq!(second.pruned, 0);
    assert_eq!(second.unchanged, 4);

    // The master goes away; so does its projection.
    delete_node(&env, "m-roll").await;
    let third = rebuild(&env).await;
    assert_eq!(third.pruned, 4);
    assert!(projected(&env).await.is_empty());
}

#[tokio::test]
async fn an_exception_suppresses_its_slot_and_a_cancelled_series_projects_nothing() {
    let env = setup().await;
    let master = rolling_master("m-sup", "ext-sup");
    write_nodes(&env, std::slice::from_ref(&master)).await;
    let all = rebuild(&env).await;
    assert_eq!(all.written, 4);

    // The second instance gets an exception. Its slot must stop being generated
    // — otherwise the day shows twice, once at the original time and once at
    // the moved one.
    let paths = projected(&env).await;
    let slot = paths[1].rsplit('/').next().unwrap().to_string();
    let slot_utc = format!(
        "{}-{}-{}T{}:{}:{}Z",
        &slot[0..4],
        &slot[4..6],
        &slot[6..8],
        &slot[9..11],
        &slot[11..13],
        &slot[13..15]
    );
    write_nodes(
        &env,
        &[exception_node("e-sup", "ext-sup", &slot_utc, "confirmed")],
    )
    .await;

    let after = rebuild(&env).await;
    assert_eq!(after.suppressed, 1);
    assert_eq!(after.pruned, 1, "the superseded slot is removed");
    let left = projected(&env).await;
    assert_eq!(left.len(), 3);
    assert!(!left.iter().any(|p| p.ends_with(&slot)));

    // A cancelled MASTER projects nothing at all.
    let mut cancelled = rolling_master("m-sup", "ext-sup");
    cancelled.properties.insert("status".into(), s("cancelled"));
    write_nodes(&env, &[cancelled]).await;
    let after_cancel = rebuild(&env).await;
    assert_eq!(after_cancel.pruned, 3);
    assert!(projected(&env).await.is_empty());
}

/// A cancelled EXCEPTION leaves a hole: the instance does not happen, so it must
/// not be generated and the exception node itself is the only trace.
#[tokio::test]
async fn a_cancelled_occurrence_does_not_appear() {
    let env = setup().await;
    write_nodes(&env, &[rolling_master("m-canc", "ext-canc")]).await;
    rebuild(&env).await;
    let paths = projected(&env).await;
    let slot = paths[2].rsplit('/').next().unwrap().to_string();
    let slot_utc = format!(
        "{}-{}-{}T{}:{}:{}Z",
        &slot[0..4],
        &slot[4..6],
        &slot[6..8],
        &slot[9..11],
        &slot[11..13],
        &slot[13..15]
    );
    write_nodes(
        &env,
        &[exception_node("e-canc", "ext-canc", &slot_utc, "cancelled")],
    )
    .await;
    let after = rebuild(&env).await;
    assert_eq!(after.suppressed, 1);
    let left = projected(&env).await;
    assert_eq!(left.len(), 3);
    assert!(!left.iter().any(|p| p.ends_with(&slot)));
}
