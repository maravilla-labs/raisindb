//! Plan Phase 8 step 3: the replication apply path writes COMPOUND and UNIQUE
//! entries from the cached definitions, so a replica's compound index stays
//! `Ready` and index-served; with the definitions cold it marks the index
//! `NotBuilt` and requests a local build — never a silent skip.

use super::compound_env::item;
use super::env::{Env, REPO, TENANT, WS};
use super::replica_tests::{applicator, upsert};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_rocksdb::indexing::compound::cold;

fn later(offset_ms: u64) -> HLC {
    HLC::new(chrono::Utc::now().timestamp_millis() as u64 + offset_ms, 0)
}

#[tokio::test]
async fn replica_compound_query_correct_without_rebuild() -> Result<()> {
    let env = Env::new(false).await?;
    env.with_items("main").await?;
    // A local write resolves (and caches) the type's definitions.
    env.add("main", item("w", "warm", &[])).await?;
    let replica = applicator(&env);

    let r1 = later(1_000);
    upsert(&replica, &item("x", "b", &[("code", "K-1")]), "a1", r1).await;
    assert!(
        env.compound_ready("main"),
        "a warm replicated write must not mark NotBuilt"
    );
    assert_eq!(env.compound("main", "b", None).await?, ["x"]);

    let r2 = later(2_000);
    upsert(&replica, &item("x", "c", &[("code", "K-1")]), "a1", r2).await;
    assert!(env.compound_ready("main"));
    assert!(
        env.compound("main", "b", None).await?.is_empty(),
        "stale tuple live"
    );
    assert_eq!(env.compound("main", "c", None).await?, ["x"]);
    assert_eq!(env.compound("main", "b", Some(&r1)).await?, ["x"]);

    // The replicated node holds its UNIQUE claim on the replica too.
    let mut dup = item("y", "c", &[("code", "K-1")]);
    dup.workspace = Some(WS.to_string());
    let ctx = env.tx("main").await?;
    assert!(
        ctx.add_node(WS, &dup).await.is_err(),
        "a replicated node's unique value was claimable locally"
    );
    Ok(())
}

/// Ops applied out of order: the older op lands below a stored successor and
/// must leave the successor's tuple — and only it — live at HEAD.
#[tokio::test]
async fn replica_out_of_order_compound_keeps_head() -> Result<()> {
    let env = Env::new(false).await?;
    env.with_items("main").await?;
    env.add("main", item("w", "warm", &[])).await?;
    let replica = applicator(&env);
    let (r1, r2, r3) = (later(1_000), later(2_000), later(3_000));
    upsert(&replica, &item("x", "a", &[]), "a1", r1).await;
    upsert(&replica, &item("x", "c", &[]), "a1", r3).await;
    upsert(&replica, &item("x", "b", &[]), "a1", r2).await;
    assert_eq!(env.compound("main", "c", None).await?, ["x"]);
    assert!(env.compound("main", "a", None).await?.is_empty());
    assert!(env.compound("main", "b", None).await?.is_empty());
    assert_eq!(env.compound("main", "b", Some(&r2)).await?, ["x"]);
    assert_eq!(env.compound("main", "a", Some(&r1)).await?, ["x"]);
    Ok(())
}

#[tokio::test]
async fn replica_cold_definition_cache_marks_not_built() -> Result<()> {
    let env = Env::new(false).await?;
    env.with_items("main").await?;
    assert!(env.compound_ready("main"));
    let replica = applicator(&env);

    // A cold cache — this node has just started (or ingested a checkpoint):
    // nothing has resolved `test:Item` here yet.
    raisin_rocksdb::indexing::compound::defs::invalidate_branch(
        env.storage.db(),
        TENANT,
        REPO,
        "main",
    );
    upsert(&replica, &item("x", "b", &[]), "a1", later(1_000)).await;
    assert!(
        !env.compound_ready("main"),
        "a cold write must fail the index closed"
    );
    assert!(cold::is_requested(
        env.storage.db(),
        TENANT,
        REPO,
        "main",
        WS
    ));
    assert!(
        env.compound("main", "b", None).await?.is_empty(),
        "nothing indexed without definitions"
    );

    // The requested build (here run directly) warms the cache and re-earns
    // Ready; the next replicated write is maintained inline.
    env.build_compound("main").await?;
    assert!(env.compound_ready("main"));
    assert_eq!(env.compound("main", "b", None).await?, ["x"]);
    upsert(&replica, &item("x", "c", &[]), "a1", later(2_000)).await;
    assert!(env.compound_ready("main"));
    assert_eq!(env.compound("main", "c", None).await?, ["x"]);
    assert!(env.compound("main", "b", None).await?.is_empty());
    Ok(())
}
