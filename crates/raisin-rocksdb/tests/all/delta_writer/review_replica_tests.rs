//! Review findings on the replica side: a cluster node is both an origin and
//! a replica, so a replicated op older than a LOCAL skip-written version must
//! not mask what that version kept from below it.

use super::env::{node, Env, WS};
use super::node_types::register_type;
use super::replica_tests::{applicator, upsert};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::{PropertyValue, RaisinReference};
use raisin_storage::{ListOptions, NodeRepository, ReferenceIndexRepository, Storage};
use std::time::Duration;

pub(super) fn reference_to(target: &str) -> PropertyValue {
    PropertyValue::Reference(RaisinReference {
        id: target.to_string(),
        workspace: WS.to_string(),
        path: format!("/{target}"),
    })
}

pub(super) async fn referrers(env: &Env, target: &str) -> Result<Vec<String>> {
    Ok(env
        .storage
        .reference_index()
        .find_referencing_nodes_at(env.scope("main"), WS, target, false, None)
        .await?
        .into_iter()
        .map(|(id, _)| id)
        .collect())
}

/// Local X at P (p=A, q=1, a reference, ordered label L); a local skip write
/// at S changes only q; then a peer's op at R (P < R < S) arrives with p=C
/// and no reference. HEAD is S: p=A, q=2, the reference, the label.
#[tokio::test]
async fn local_skip_write_then_older_replicated_op_keeps_head_entries() -> Result<()> {
    let env = Env::new(true).await?;
    let replica = applicator(&env);
    env.add("main", node("t", "/t", &[])).await?;
    let mut x = node("x", "/x", &[("p", "A"), ("q", "1")]);
    x.properties.insert("link".into(), reference_to("t"));
    env.add("main", x.clone()).await?;
    let p_rev = env.newest_revision("main", "x");
    let label = env
        .storage
        .nodes()
        .get(env.scope("main"), "x", None)
        .await?
        .expect("x")
        .order_key;
    assert!(!label.is_empty());
    tokio::time::sleep(Duration::from_millis(5)).await;

    x.properties
        .insert("q".into(), PropertyValue::String("2".into()));
    env.put("main", x).await?;
    let s_rev = env.newest_revision("main", "x");
    let r = HLC::new(p_rev.timestamp_ms, p_rev.counter + 1);
    assert!(p_rev < r && r < s_rev, "{p_rev} < {r} < {s_rev}");

    upsert(
        &replica,
        &node("x", "/x", &[("p", "C"), ("q", "1")]),
        &label,
        r,
    )
    .await;

    assert_eq!(env.indexed("main", "p", "A", None).await?, ["x"], "p=A");
    assert!(env.indexed("main", "p", "C", None).await?.is_empty());
    assert_eq!(env.indexed("main", "q", "2", None).await?, ["x"]);
    assert!(env.indexed("main", "q", "1", None).await?.is_empty());
    assert_eq!(referrers(&env, "t").await?, ["x"], "reference masked");
    let children: Vec<String> = env
        .storage
        .nodes()
        .list_children(env.scope("main"), "/", ListOptions::for_api())
        .await?
        .into_iter()
        .map(|n| n.id)
        .collect();
    assert!(children.contains(&"x".to_string()), "{children:?}");
    // R answers as R wrote it.
    assert_eq!(env.indexed("main", "p", "C", Some(&r)).await?, ["x"]);
    Ok(())
}

/// The replica writes no UNIQUE claim; a local update of a replicated node
/// must still put its claims, or a duplicate value is accepted afterwards.
#[tokio::test]
async fn unique_claim_reput_after_replicated_create() -> Result<()> {
    let env = Env::new(true).await?;
    register_type(&env.storage, "main", "test:Unique", Some("sku"), None).await?;
    let replica = applicator(&env);
    let mut x = node("x", "/x", &[("sku", "S-1"), ("title", "one")]);
    x.node_type = "test:Unique".to_string();
    let now = HLC::new(chrono::Utc::now().timestamp_millis() as u64, 0);
    upsert(&replica, &x, "a0", now).await;
    tokio::time::sleep(Duration::from_millis(5)).await;

    // A local edit that leaves `sku` alone.
    x.properties
        .insert("title".into(), PropertyValue::String("two".into()));
    env.put("main", x).await?;

    let mut y = node("y", "/y", &[("sku", "S-1")]);
    y.node_type = "test:Unique".to_string();
    let ctx = env.tx("main").await?;
    assert!(
        ctx.add_node(WS, &y).await.is_err(),
        "duplicate unique value accepted"
    );
    Ok(())
}
