//! Plan Phase 13e review: a LIMIT pushed into a workspace-index listing must
//! not cut the index read short when anything after the read can drop a row
//! — a residual filter (a keyset cursor, a node type with no index of its
//! own) or row-level security. The cut used to be `max(10 * LIMIT, 100)`
//! entries, so a page whose rows lie past it came back short or empty.
//!
//! `/a` holds 300 children — far more than the cut — so every page asked
//! for here lies beyond it.

use super::compound_index_hierarchy::{explain, setup, strings, Engine, Owner, BRANCH, NOTE_TYPE};
use super::compound_index_hierarchy::{NODE_TYPE, TENANT, WS};
use chrono::{DateTime, Duration, TimeZone, Utc};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::permissions::{Operation, Permission, ResolvedPermissions};
use raisin_storage::{CreateNodeOptions, NodeRepository, Storage, StorageScope};
use std::collections::HashMap;

const CHILDREN: i64 = 300;

fn at(secs: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap() + Duration::seconds(secs)
}

/// `/a/c{age}` created at `at(age)`; the five oldest are `test:Note`s owned
/// by alice, the rest `test:Message`s owned by bob.
fn child(age: i64) -> Node {
    let (node_type, owner) = if age < 5 {
        (NOTE_TYPE, "alice")
    } else {
        (NODE_TYPE, "bob")
    };
    let name = format!("c{age:03}");
    Node {
        id: name.clone(),
        path: format!("/a/{name}"),
        name,
        parent: Some("a".to_string()),
        node_type: node_type.to_string(),
        properties: HashMap::from([(
            "owner".to_string(),
            PropertyValue::String(owner.to_string()),
        )]),
        created_at: Some(at(age)),
        ..Default::default()
    }
}

/// The hierarchy setup (workspace-owned `@folder_time`, built) plus
/// [`CHILDREN`] children of `/a`.
async fn listing(
    repo: &str,
) -> (
    Engine,
    std::sync::Arc<raisin_rocksdb::RocksDBStorage>,
    tempfile::TempDir,
) {
    let (engine, storage, tmp) = setup(repo, Owner::Workspace, true).await;
    for age in 0..CHILDREN {
        storage
            .nodes()
            .create(
                StorageScope::new(TENANT, repo, BRANCH, WS),
                child(age),
                CreateNodeOptions {
                    validate_parent_allows_child: false,
                    validate_workspace_allows_type: false,
                    ..Default::default()
                },
            )
            .await
            .expect("create");
    }
    (engine, storage, tmp)
}

fn expected(ages: impl Iterator<Item = i64>) -> Vec<String> {
    ages.map(|age| format!("c{age:03}")).collect()
}

async fn index_served(engine: &Engine, sql: &str) {
    let plan = explain(engine, &format!("EXPLAIN {sql}")).await;
    assert!(
        plan.contains("CompoundIndexScan") && plan.contains("@folder_time"),
        "not served by the workspace index:\n{plan}"
    );
}

/// A keyset page far down the folder: every entry the cut read would return
/// is newer than the cursor.
#[tokio::test]
async fn a_keyset_page_past_the_index_read_cap_is_complete() {
    let (engine, _storage, _tmp) = listing("r_cwlim_keyset").await;
    let sql = format!(
        "SELECT id FROM 'ws' WHERE CHILD_OF('/a') AND created_at < '{}'::TIMESTAMPTZ \
         ORDER BY created_at DESC LIMIT 20",
        at(50).to_rfc3339()
    );
    index_served(&engine, &sql).await;
    assert_eq!(
        strings(&engine, &sql, "id").await,
        expected((30..50).rev()),
        "pagination ended early"
    );
}

/// A typed listing whose type has no index of its own: `node_type` stays a
/// residual over the workspace index, and the notes are the OLDEST children.
#[tokio::test]
async fn a_typed_listing_past_the_index_read_cap_is_complete() {
    let (engine, _storage, _tmp) = listing("r_cwlim_typed").await;
    let sql = format!(
        "SELECT id FROM 'ws' WHERE CHILD_OF('/a') AND node_type = '{NOTE_TYPE}' \
         ORDER BY created_at DESC LIMIT 5"
    );
    index_served(&engine, &sql).await;
    assert_eq!(strings(&engine, &sql, "id").await, expected((0..5).rev()));
}

/// A caller whose row-level security admits only the five oldest children.
#[tokio::test]
async fn an_rls_restricted_listing_past_the_index_read_cap_is_complete() {
    let (engine, _storage, _tmp) = listing("r_cwlim_rls").await;
    let sql = "SELECT id FROM 'ws' WHERE CHILD_OF('/a') ORDER BY created_at DESC LIMIT 5";
    index_served(&engine, sql).await;
    let alice = AuthContext::for_user("alice").with_permissions(ResolvedPermissions {
        user_id: "alice".to_string(),
        email: Some("alice@test.com".to_string()),
        direct_roles: vec![],
        group_roles: vec![],
        effective_roles: vec![],
        groups: vec![],
        permissions: vec![Permission::new("/**", vec![Operation::Read])
            .with_condition("node.owner == auth.user_id".to_string())],
        is_system_admin: false,
        resolved_at: Some(std::time::Instant::now()),
    });
    let engine = engine.with_auth(alice);
    assert_eq!(strings(&engine, sql, "id").await, expected((0..5).rev()));
}
