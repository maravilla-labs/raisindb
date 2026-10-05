//! Plan Phase 7b: the per-node commit lock. Two writers of one node that
//! overlap commit out of revision order; each commit re-validates its staged
//! index delta against the versions stored at that moment, under a mutex
//! every write funnel (and the replication applicator) takes.
//!
//! Each race test runs twice: with the lock, the index at HEAD and at every
//! revision must equal what a full rebuild derives from the node's versions;
//! with the lock switched off for that database
//! (`node_lock::test_hooks::disable_node_locks`), the same interleaving must
//! reproduce the divergence — the proof that the test exercises the lock.
//! The interleaving is forced, not hoped for: the first committer pauses
//! between its re-validation and its write (`pause_commit_of`).

use super::env::{node, Env, WS};
use super::replica_tests::{applicator, upsert};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_rocksdb::indexing::node_lock::test_hooks::{disable_node_locks, pause_commit_of};
use raisin_storage::{NodeRepository, Storage};
use std::collections::BTreeSet;
use std::future::Future;
use std::time::Duration;

fn titled(title: &str, status: &str) -> Node {
    node("x", "/x", &[("title", title), ("status", status)])
}

/// How long a contender is given to finish while the first committer is
/// paused: with the lock it must still be waiting, without it it must be done.
const CONTENDER_WINDOW: Duration = Duration::from_millis(400);

/// Run `contender` while the paused commit waits; return whether it finished
/// inside [`CONTENDER_WINDOW`], then let the paused commit write and finish
/// `contender`.
async fn race<F: Future<Output = Result<()>>>(
    pause: &raisin_rocksdb::indexing::node_lock::test_hooks::CommitPause,
    contender: F,
) -> Result<bool> {
    pause.reached().await;
    let mut contender = Box::pin(contender);
    let finished_early = match tokio::time::timeout(CONTENDER_WINDOW, &mut contender).await {
        Ok(result) => {
            result?;
            true
        }
        Err(_) => false,
    };
    pause.release();
    if !finished_early {
        contender.await?;
    }
    Ok(finished_early)
}

/// Every `(property, value)` whose PROPERTY_INDEX answer for `id` differs
/// from what a full rebuild derives from the node's stored versions, at every
/// version's revision and at HEAD.
async fn divergence(env: &Env, id: &str) -> Result<Vec<String>> {
    let history = env
        .storage
        .nodes_impl()
        .get_history(super::env::TENANT, super::env::REPO, "main", WS, id, None)
        .await?;
    let mut candidates = BTreeSet::new();
    for (_, version) in &history {
        for (name, value) in version.iter().flat_map(|n| n.properties.iter()) {
            if let PropertyValue::String(value) = value {
                candidates.insert((name.clone(), value.clone()));
            }
        }
    }
    let version_at = |at: Option<&HLC>| -> Option<&Node> {
        history
            .iter()
            .find(|(rev, _)| at.is_none_or(|at| rev <= at))
            .and_then(|(_, version)| version.as_ref())
    };
    let mut points: Vec<Option<HLC>> = history.iter().map(|(rev, _)| Some(*rev)).collect();
    points.push(None);
    let mut out = Vec::new();
    for at in &points {
        let version = version_at(at.as_ref());
        for (name, value) in &candidates {
            let expected = version.is_some_and(|n| {
                n.properties.get(name) == Some(&PropertyValue::String(value.clone()))
            });
            let actual = env
                .indexed("main", name, value, at.as_ref())
                .await?
                .iter()
                .any(|n| n == id);
            if expected != actual {
                let at = at.map(|r| r.to_string()).unwrap_or_else(|| "HEAD".into());
                out.push(format!(
                    "{name}={value} at {at}: index {actual}, versions {expected}"
                ));
            }
        }
    }
    Ok(out)
}

/// Two local transactions stage against the same stored version P; B (the
/// HIGHER revision) re-validates first and pauses, then A (the lower) tries
/// to commit. With the lock A waits, and its re-validation sees B: a
/// corrective out-of-order write (B's skipped `title = x` re-asserted, A's
/// `title = y` ended at B's revision). Without it A commits inside B's window
/// and B's skip-written version loses `title = x` under A's tombstone.
#[tokio::test]
async fn overlapping_local_transactions_out_of_revision_order_match_a_rebuild() -> Result<()> {
    for locks in [true, false] {
        let env = Env::new(true).await?;
        disable_node_locks(env.storage.db(), !locks);
        env.add("main", titled("x", "a")).await?;
        let tx_a = env.tx("main").await?;
        tx_a.put_node(WS, &titled("y", "a")).await?;
        let tx_b = env.tx("main").await?;
        tx_b.put_node(WS, &titled("x", "b")).await?;

        let pause = pause_commit_of(env.storage.db(), "x");
        let (b, a_finished_early) = tokio::join!(tx_b.commit(), race(&pause, tx_a.commit()));
        b?;
        assert_eq!(a_finished_early?, !locks, "A must wait for B's lock");

        let history = env
            .storage
            .nodes_impl()
            .get_history(super::env::TENANT, super::env::REPO, "main", WS, "x", None)
            .await?;
        assert_eq!(history.len(), 3, "P, A and B are stored");
        let wrong = divergence(&env, "x").await?;
        if locks {
            assert!(
                wrong.is_empty(),
                "index diverges from a rebuild: {wrong:#?}"
            );
            assert_eq!(env.indexed("main", "title", "x", None).await?, ["x"]);
        } else {
            assert!(
                !wrong.is_empty(),
                "without the lock the race must reproduce, or this test proves nothing"
            );
        }
        disable_node_locks(env.storage.db(), false);
    }
    Ok(())
}

/// A local commit (S, skip-written against P) re-validates and pauses; a
/// replicated op for the same node at R (P < R < S) arrives meanwhile. With
/// the lock the apply waits, then sees S as its successor (out of order: R's
/// values ended at S, S's entries re-asserted). Without it the apply sees no
/// successor, tombstones P's `title = x` at R, and S — which kept `title = x`
/// from P — loses it at HEAD.
#[tokio::test]
async fn local_commit_racing_an_out_of_order_replicated_write_matches_a_rebuild() -> Result<()> {
    for locks in [true, false] {
        let env = Env::new(true).await?;
        disable_node_locks(env.storage.db(), !locks);
        let replica = applicator(&env);
        env.add("main", titled("x", "a")).await?;
        let p_rev = env.newest_revision("main", "x");
        let label = env
            .storage
            .nodes()
            .get(env.scope("main"), "x", None)
            .await?
            .expect("x")
            .order_key;
        tokio::time::sleep(Duration::from_millis(5)).await;
        let tx_s = env.tx("main").await?;
        tx_s.put_node(WS, &titled("x", "b")).await?;
        let r = HLC::new(p_rev.timestamp_ms, p_rev.counter + 1);

        let pause = pause_commit_of(env.storage.db(), "x");
        let replicated = async {
            upsert(&replica, &titled("y", "a"), &label, r).await;
            Ok(())
        };
        let (s, applied_early) = tokio::join!(tx_s.commit(), race(&pause, replicated));
        s?;
        assert_eq!(
            applied_early?, !locks,
            "the apply must wait for the local commit"
        );
        let s_rev = env.newest_revision("main", "x");
        assert!(p_rev < r && r < s_rev, "{p_rev} < {r} < {s_rev}");

        let wrong = divergence(&env, "x").await?;
        if locks {
            assert!(
                wrong.is_empty(),
                "index diverges from a rebuild: {wrong:#?}"
            );
            assert_eq!(env.indexed("main", "title", "x", None).await?, ["x"]);
            assert!(env.indexed("main", "title", "y", None).await?.is_empty());
        } else {
            assert!(
                !wrong.is_empty(),
                "without the lock the race must reproduce, or this test proves nothing"
            );
        }
        disable_node_locks(env.storage.db(), false);
    }
    Ok(())
}

/// Transactions writing the same nodes in opposite orders, committed
/// concurrently, never deadlock: a commit locks its nodes in sorted order,
/// whatever order it wrote them in. One of each pair is paused holding its
/// locks while the other queues, so the two always overlap.
#[tokio::test]
async fn multi_node_transactions_in_opposite_orders_do_not_deadlock() -> Result<()> {
    let env = Env::new(true).await?;
    for id in ["a", "b", "c"] {
        env.add("main", node(id, &format!("/{id}"), &[("v", "0")]))
            .await?;
    }
    for round in 0..12 {
        let value = round.to_string();
        let forward = env.tx("main").await?;
        let backward = env.tx("main").await?;
        for id in ["a", "b", "c"] {
            forward
                .put_node(WS, &node(id, &format!("/{id}"), &[("v", &value)]))
                .await?;
        }
        for id in ["c", "b", "a"] {
            backward
                .put_node(WS, &node(id, &format!("/{id}"), &[("w", &value)]))
                .await?;
        }
        let first = if round % 2 == 0 { "a" } else { "c" };
        let pause = pause_commit_of(env.storage.db(), first);
        let both = async {
            let (f, b) = tokio::join!(forward.commit(), race(&pause, backward.commit()));
            f?;
            b.map(|_| ())
        };
        tokio::time::timeout(Duration::from_secs(20), both)
            .await
            .expect("opposite-order transactions deadlocked")?;
    }
    // The paused (forward) commit wrote first, the queued one last.
    let last = env.indexed("main", "w", "11", None).await?;
    for id in ["a", "b", "c"] {
        assert!(
            last.contains(&id.to_string()),
            "{id} carries the last write"
        );
    }
    Ok(())
}
