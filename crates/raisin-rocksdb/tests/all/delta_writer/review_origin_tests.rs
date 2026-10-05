//! Review findings on the origin: writes that commit below, or beside, a
//! version written with skips.

use super::env::{node, Env, WS};
use super::node_types::register_type;
use super::review_replica_tests::{reference_to, referrers};
use raisin_error::Result;
use raisin_models::nodes::properties::PropertyValue;

/// tx1 takes its revision, a later write of `n` (b only) commits with skips,
/// then the flag is turned OFF (the documented rollback) and tx1 writes `n`
/// below it. The full put must still re-assert the successor's entries.
#[tokio::test]
async fn out_of_order_write_after_rollback_keeps_successor_entries() -> Result<()> {
    let env = Env::new(true).await?;
    env.add("main", node("n", "/n", &[("a", "1"), ("b", "1")]))
        .await?;
    env.add("main", node("other", "/other", &[])).await?;
    let tx1 = env.tx("main").await?;
    tx1.put_node(WS, &node("other", "/other", &[("x", "1")]))
        .await?;
    env.put("main", node("n", "/n", &[("a", "1"), ("b", "2")]))
        .await?;
    env.storage.nodes_impl().set_index_skip_unchanged(false);
    tx1.put_node(WS, &node("n", "/n", &[("a", "2"), ("b", "1")]))
        .await?;
    tx1.commit().await?;
    assert!(env.newest_revision("main", "other") < env.newest_revision("main", "n"));

    assert_eq!(env.indexed("main", "a", "1", None).await?, ["n"], "a=1");
    assert!(env.indexed("main", "a", "2", None).await?.is_empty());
    assert_eq!(env.indexed("main", "b", "2", None).await?, ["n"]);
    Ok(())
}

/// The same out-of-order shape for REFERENCE and UNIQUE: the successor
/// (which only changed the title) keeps the reference and the claim; the
/// lower write removes both. HEAD must still answer as the successor.
#[tokio::test]
async fn out_of_order_write_keeps_successor_reference_and_unique_claim() -> Result<()> {
    let env = Env::new(true).await?;
    register_type(&env.storage, "main", "test:Unique", Some("sku"), None).await?;
    env.add("main", node("t", "/t", &[])).await?;
    env.add("main", node("other", "/other", &[])).await?;
    let mut x = node("x", "/x", &[("sku", "S-1"), ("title", "one")]);
    x.node_type = "test:Unique".to_string();
    x.properties.insert("link".into(), reference_to("t"));
    env.add("main", x.clone()).await?;

    let tx1 = env.tx("main").await?;
    tx1.put_node(WS, &node("other", "/other", &[("x", "1")]))
        .await?;
    let mut successor = x.clone();
    successor
        .properties
        .insert("title".into(), PropertyValue::String("two".into()));
    env.put("main", successor).await?;
    let mut lower = x.clone();
    lower.properties.remove("link");
    lower
        .properties
        .insert("sku".into(), PropertyValue::String("S-2".into()));
    tx1.put_node(WS, &lower).await?;
    tx1.commit().await?;

    assert_eq!(referrers(&env, "t").await?, ["x"], "reference masked");
    let mut y = node("y", "/y", &[("sku", "S-1")]);
    y.node_type = "test:Unique".to_string();
    let ctx = env.tx("main").await?;
    assert!(
        ctx.add_node(WS, &y).await.is_err(),
        "the successor's claim was masked"
    );
    Ok(())
}

fn titled(title: &str, status: &str) -> raisin_models::nodes::Node {
    node("x", "/x", &[("title", title), ("status", status)])
}

/// Two writers stage against the same stored predecessor and commit in
/// revision order: B keeps `title = x` (unchanged against P) while A, below
/// it, tombstones it. HEAD is B's version.
#[tokio::test]
async fn concurrent_updates_proving_the_same_predecessor_keep_unchanged_values() -> Result<()> {
    let env = Env::new(true).await?;
    env.add("main", titled("x", "a")).await?;
    let tx_a = env.tx("main").await?;
    tx_a.put_node(WS, &titled("y", "a")).await?;
    let tx_b = env.tx("main").await?;
    tx_b.put_node(WS, &titled("x", "b")).await?;
    tx_a.commit().await?;
    tx_b.commit().await?;

    assert_eq!(env.indexed("main", "title", "x", None).await?, ["x"]);
    assert!(env.indexed("main", "title", "y", None).await?.is_empty());
    assert_eq!(env.indexed("main", "status", "b", None).await?, ["x"]);
    assert!(env.indexed("main", "status", "a", None).await?.is_empty());
    Ok(())
}

/// The same two writers committing OUT of revision order: A (the lower
/// revision) commits last, after B's skip write is stored above it.
#[tokio::test]
async fn out_of_revision_order_commits_keep_head_entries() -> Result<()> {
    let env = Env::new(true).await?;
    env.add("main", titled("x", "a")).await?;
    let tx_a = env.tx("main").await?;
    tx_a.put_node(WS, &titled("y", "a")).await?;
    let tx_b = env.tx("main").await?;
    tx_b.put_node(WS, &titled("x", "b")).await?;
    tx_b.commit().await?;
    tx_a.commit().await?;

    assert_eq!(env.indexed("main", "title", "x", None).await?, ["x"]);
    assert!(env.indexed("main", "title", "y", None).await?.is_empty());
    assert_eq!(env.indexed("main", "status", "b", None).await?, ["x"]);
    assert!(env.indexed("main", "status", "a", None).await?.is_empty());
    Ok(())
}
