//! Plan Phase 7b review: the commit-time re-validation for the races the
//! first pass missed or over-reacted to. Each test is named after the
//! failure it pins.

use super::compound_env::{item, ITEM};
use super::env::{node, Env, REPO, TENANT, WS};
use super::node_types::register_type;
use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_rocksdb::indexing::lock_nodes;
use raisin_rocksdb::indexing::node_lock::test_hooks::pause_write_of;
use std::time::Duration;

/// An item at `path` (not the root).
fn item_at(id: &str, path: &str, cat: &str) -> Node {
    let mut n = node(id, path, &[("cat", cat)]);
    n.node_type = ITEM.to_string();
    n
}

fn ping(title: &str) -> Node {
    let mut n = node("ping", "/ping", &[("title", title)]);
    n.node_type = "test:Ping".to_string();
    n
}

/// Two ordinary writers of one node overlap: the second sees the first's
/// NODES version AND its NODE_PATH entry (every record write writes both).
/// That is a record write, corrected from NODES — not an index-only write —
/// so the workspace's compound indexes must stay `Ready` (they used to be
/// marked `NotBuilt`, with a full rebuild queued, on every such race).
#[tokio::test]
async fn racing_writers_of_one_node_keep_the_workspace_compound_index_ready() -> Result<()> {
    let env = Env::new(true).await?;
    env.with_items("main").await?;
    env.add("main", item("x", "a", &[])).await?;
    assert!(env.compound_ready("main"));

    let tx_a = env.tx("main").await?;
    tx_a.put_node(WS, &item("x", "b", &[])).await?;
    let tx_b = env.tx("main").await?;
    tx_b.put_node(WS, &item("x", "c", &[])).await?;
    tx_a.commit().await?;
    tx_b.commit().await?;

    assert!(
        env.compound_ready("main"),
        "a record-write race must not fail the workspace's compound indexes closed"
    );
    assert_eq!(env.compound("main", "c", None).await?, ["x"]);
    assert!(env.compound("main", "b", None).await?.is_empty());
    assert!(env.compound("main", "a", None).await?.is_empty());
    Ok(())
}

/// A move lists descendant D (`cat = draft`), an update of D (staged at a
/// LOWER revision) sets `cat = published` and commits, then the move commits
/// and re-keys D's compound entries without rewriting D's record. The re-key
/// must follow the version stored then, not the listed one: a `draft` tuple
/// live at the move's revision would match D at HEAD forever.
#[tokio::test]
async fn a_descendant_updated_during_a_move_is_rekeyed_from_its_stored_version() -> Result<()> {
    let env = Env::new(true).await?;
    env.with_items("main").await?;
    env.add("main", node("f", "/f", &[])).await?;
    env.add("main", node("g", "/g", &[])).await?;
    env.add("main", item_at("d", "/f/d", "draft")).await?;

    let update = env.tx("main").await?;
    update
        .put_node(WS, &item_at("d", "/f/d", "published"))
        .await?;
    let mover = env.tx("main").await?;
    mover.move_node_tree(WS, "f", "/g/f").await?;
    update.commit().await?;
    mover.commit().await?;

    assert!(
        env.compound("main", "draft", None).await?.is_empty(),
        "the move re-keyed D from the version it listed"
    );
    assert_eq!(env.compound("main", "published", None).await?, ["d"]);
    assert!(env.compound_ready("main"));
    Ok(())
}

/// Two in-place (`versionable=false`) rewrites of one node overlap: both read
/// the content at the reused revision R0 and both end only ITS values. The
/// second commit must end the first's value too, or `title = ok` matches the
/// node at HEAD while its record says `down`.
#[tokio::test]
async fn overlapping_in_place_rewrites_leave_no_phantom_value() -> Result<()> {
    let env = Env::new(true).await?;
    register_type(&env.storage, "main", "test:Ping", None, Some(false)).await?;
    env.add("main", ping("init")).await?;
    let r0 = env.newest_revision("main", "ping");

    let tx_a = env.tx("main").await?;
    tx_a.put_node(WS, &ping("ok")).await?;
    let tx_b = env.tx("main").await?;
    tx_b.put_node(WS, &ping("down")).await?;
    tx_a.commit().await?;
    tx_b.commit().await?;

    assert_eq!(env.newest_revision("main", "ping"), r0, "written in place");
    assert_eq!(env.indexed("main", "title", "down", None).await?, ["ping"]);
    assert!(
        env.indexed("main", "title", "ok", None).await?.is_empty(),
        "the first in-place value must be ended by the second"
    );
    assert!(env.indexed("main", "title", "init", None).await?.is_empty());
    Ok(())
}

/// Two creates of one (deterministic) id overlap: both stage a `NoPrior`
/// full put. The second commit must end the first's values, or `tag = a`
/// matches the node at HEAD while its record says `b`. (Different paths: a
/// create reserves its path, so two creates at ONE path conflict outright.)
#[tokio::test]
async fn racing_creates_of_one_id_leave_no_phantom_value() -> Result<()> {
    let env = Env::new(true).await?;
    let tx_a = env.tx("main").await?;
    tx_a.put_node(WS, &node("n", "/n-a", &[("tag", "a")]))
        .await?;
    let tx_b = env.tx("main").await?;
    tx_b.put_node(WS, &node("n", "/n-b", &[("tag", "b")]))
        .await?;
    tx_a.commit().await?;
    tx_b.commit().await?;

    assert_eq!(env.indexed("main", "tag", "b", None).await?, ["n"]);
    assert!(
        env.indexed("main", "tag", "a", None).await?.is_empty(),
        "the first create's value must be ended by the second"
    );
    Ok(())
}

/// A commit future dropped while its write runs on the blocking pool (a
/// client disconnect, a timeout wrapper) must keep the node locked until the
/// write lands — otherwise another writer re-validates against stored state
/// that lacks the in-flight batch.
#[tokio::test]
async fn a_commit_dropped_mid_write_keeps_its_node_locked_until_the_write_lands() -> Result<()> {
    let env = Env::new(true).await?;
    env.add("main", node("x", "/x", &[("title", "a")])).await?;
    let db = env.storage.db();
    let pause = pause_write_of(db, "x");

    let tx = env.tx("main").await?;
    tx.put_node(WS, &node("x", "/x", &[("title", "b")])).await?;
    {
        let mut commit = Box::pin(tx.commit());
        let waiter = pause.clone();
        let mut reached =
            tokio::task::spawn_blocking(move || waiter.wait_reached(Duration::from_secs(20)));
        tokio::select! {
            result = &mut commit => panic!("the commit finished before its write paused: {result:?}"),
            reached = &mut reached => assert!(reached.expect("join"), "the write never paused"),
        }
        // Dropped mid-write: the blocking task still holds the batch.
    }

    let contender = tokio::time::timeout(
        Duration::from_millis(400),
        lock_nodes(db, TENANT, REPO, "main", ["x"]),
    )
    .await;
    assert!(
        contender.is_err(),
        "the node was released while its write was still in flight"
    );

    pause.release();
    let guard = tokio::time::timeout(
        Duration::from_secs(20),
        lock_nodes(db, TENANT, REPO, "main", ["x"]),
    )
    .await
    .expect("the node is released once the write lands");
    drop(guard);
    assert_eq!(env.indexed("main", "title", "b", None).await?, ["x"]);
    Ok(())
}
