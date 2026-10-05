//! Plan Phase 8 on the origin: the COMPOUND reader decides each `(tuple,
//! node)` by its newest entry at or below the read revision, and the writer
//! derives the old tuple from the baseline (no workspace scan, no in-place
//! overwrite).

use super::compound_env::{item, BY_CAT};
use super::env::{Env, REPO, TENANT, WS};
use crate::perf_counters::{count, count_async};
use raisin_context::ResolutionType;
use raisin_error::Result;
use raisin_storage::{DeleteNodeOptions, NodeRepository, Storage};

/// The index as of every revision a node went through: create `a`, change to
/// `b`, an edit that keeps `b` (skipped with the flag on), back to `a`.
/// The old writer overwrote each superseded entry IN PLACE with a tombstone,
/// so a read at r1 found nothing under `a`.
#[tokio::test]
async fn compound_time_travel_after_update() -> Result<()> {
    for skip in [false, true] {
        let env = Env::new(skip).await?;
        env.with_items("main").await?;
        env.add("main", item("x", "a", &[])).await?;
        let r1 = env.newest_revision("main", "x");
        env.put("main", item("x", "b", &[])).await?;
        let r2 = env.newest_revision("main", "x");
        env.put("main", item("x", "b", &[("title", "t")])).await?;
        let r3 = env.newest_revision("main", "x");
        env.put("main", item("x", "a", &[("title", "t")])).await?;
        let r4 = env.newest_revision("main", "x");

        for (at, a, b) in [
            (r1, vec!["x"], vec![]),
            (r2, vec![], vec!["x"]),
            (r3, vec![], vec!["x"]),
            (r4, vec!["x"], vec![]),
        ] {
            assert_eq!(
                env.compound("main", "a", Some(&at)).await?,
                a,
                "a@{at} skip={skip}"
            );
            assert_eq!(
                env.compound("main", "b", Some(&at)).await?,
                b,
                "b@{at} skip={skip}"
            );
        }
        assert_eq!(env.compound("main", "a", None).await?, ["x"]);
        assert!(env.compound("main", "b", None).await?.is_empty());
        assert!(env.compound_ready("main"));
    }
    Ok(())
}

/// The repository update and delete funnels: the delete tombstones at its
/// own revision, so the node still lists at the revisions before it.
#[tokio::test]
async fn repository_update_and_delete_keep_compound_history() -> Result<()> {
    let env = Env::new(false).await?;
    env.with_items("main").await?;
    env.add("main", item("x", "a", &[])).await?;
    let r1 = env.newest_revision("main", "x");
    let mut x = env
        .storage
        .nodes()
        .get(env.scope("main"), "x", None)
        .await?
        .expect("x");
    x.properties.insert(
        "cat".into(),
        raisin_models::nodes::properties::PropertyValue::String("b".into()),
    );
    env.storage
        .nodes()
        .update(env.scope("main"), x, Default::default())
        .await?;
    let r2 = env.newest_revision("main", "x");
    env.storage
        .nodes()
        .delete(env.scope("main"), "x", DeleteNodeOptions::default())
        .await?;

    assert_eq!(env.compound("main", "a", Some(&r1)).await?, ["x"]);
    assert_eq!(env.compound("main", "b", Some(&r2)).await?, ["x"]);
    assert!(env.compound("main", "a", None).await?.is_empty());
    assert!(env.compound("main", "b", None).await?.is_empty());
    Ok(())
}

/// A delete whose definitions are NOT cached finds the node's live entries by
/// a scan instead — and still tombstones at the delete revision.
#[tokio::test]
async fn cold_delete_scans_and_keeps_history() -> Result<()> {
    let env = Env::new(false).await?;
    env.with_items("main").await?;
    env.add("main", item("x", "a", &[])).await?;
    let r1 = env.newest_revision("main", "x");
    raisin_rocksdb::indexing::compound::defs::invalidate_branch(
        env.storage.db(),
        TENANT,
        REPO,
        "main",
    );
    let ctx = env.tx("main").await?;
    ctx.delete_node(WS, "x").await?;
    ctx.commit().await?;
    assert!(env.compound("main", "a", None).await?.is_empty());
    assert_eq!(env.compound("main", "a", Some(&r1)).await?, ["x"]);
    Ok(())
}

/// One update costs the same whatever the workspace holds: the old tuple is
/// derived from the replaced version. The old tombstoner prefix-scanned every
/// compound entry of the workspace (twice) on every update.
///
/// Measured three ways per workspace size: the whole update through the
/// transaction, the compound writer alone, and — as the proof that the
/// counters see a scan when there is one — the cold delete fallback, which
/// still walks the workspace's compound keyspace.
#[tokio::test]
async fn compound_update_cost_independent_of_workspace_size() -> Result<()> {
    use raisin_rocksdb::indexing::compound::{
        tombstone_compound_for_delete, write_compound_delta, DefsSet,
    };
    use raisin_rocksdb::indexing::{Baseline, IndexCtx};
    let ctx = IndexCtx::new(TENANT, REPO, "main", WS);
    let mut put = Vec::new();
    let mut writer = Vec::new();
    let mut scan = Vec::new();
    for size in [20usize, 400] {
        let env = Env::new(false).await?;
        env.with_items("main").await?;
        let tx = env.tx("main").await?;
        for i in 0..size {
            tx.add_node(WS, &item(&format!("n{i}"), "x", &[])).await?;
        }
        tx.commit().await?;
        let (result, counts) = count_async(env.put("main", item("n0", "y", &[]))).await;
        result?;
        assert_eq!(env.compound("main", "y", None).await?, ["n0"]);
        assert_eq!(env.compound("main", "x", None).await?.len(), size - 1);
        put.push(counts.next_on_memtable);

        let db = env.storage.db();
        let defs = DefsSet::resolve(
            db,
            env.storage.node_types(),
            raisin_storage::BranchScope::new(TENANT, REPO, "main"),
            &["test:Item"],
        )
        .await?;
        let (old, new) = (item("n1", "x", &[]), item("n1", "z", &[]));
        let rev = raisin_hlc::HLC::new(u64::MAX / 2, 0);
        let ((), counts) = count(|| {
            let mut batch = rocksdb::WriteBatch::default();
            write_compound_delta(
                &mut batch,
                db,
                &ctx,
                &defs,
                Baseline::Full(Some(&old)),
                &new,
                &rev,
            )
            .unwrap();
        });
        writer.push(counts.next_on_memtable + counts.seek_on_memtable);

        raisin_rocksdb::indexing::compound::defs::invalidate_branch(db, TENANT, REPO, "main");
        let ((), counts) = count(|| {
            let mut batch = rocksdb::WriteBatch::default();
            tombstone_compound_for_delete(&mut batch, db, &ctx, &old, &rev).unwrap();
        });
        scan.push(counts.next_on_memtable);
    }
    assert!(
        scan[1] >= scan[0] + 300,
        "the counters must see a workspace scan (cold delete fallback): {scan:?}"
    );
    assert_eq!(
        writer[0], writer[1],
        "the compound writer's reads grew with the workspace"
    );
    assert!(
        put[1] <= put[0] + 40,
        "an update over 400 indexed nodes stepped {} keys against {} over 20",
        put[1],
        put[0]
    );
    Ok(())
}

/// Two writers stage against the same stored predecessor; B keeps `cat = a`
/// unchanged (skipped) while A, committed between, moves it to `b`. The
/// commit-time re-check corrects B's compound write as it does the property
/// index — in revision order and out of it.
#[tokio::test]
async fn concurrent_updates_keep_the_compound_tuple() -> Result<()> {
    for b_first in [false, true] {
        let env = Env::new(true).await?;
        env.with_items("main").await?;
        env.add("main", item("x", "a", &[("title", "t0")])).await?;
        let tx_a = env.tx("main").await?;
        tx_a.put_node(WS, &item("x", "b", &[("title", "t0")]))
            .await?;
        let tx_b = env.tx("main").await?;
        tx_b.put_node(WS, &item("x", "a", &[("title", "t1")]))
            .await?;
        if b_first {
            tx_b.commit().await?;
            tx_a.commit().await?;
        } else {
            tx_a.commit().await?;
            tx_b.commit().await?;
        }
        assert_eq!(
            env.compound("main", "a", None).await?,
            ["x"],
            "b_first={b_first}"
        );
        assert!(
            env.compound("main", "b", None).await?.is_empty(),
            "b_first={b_first}"
        );
    }
    Ok(())
}

/// A merge resolution writes the compound entries at M (it used to write
/// none and mark the index `NotBuilt`): the index stays usable and answers
/// with the resolved value only.
#[tokio::test]
async fn merge_resolution_writes_compound_entries() -> Result<()> {
    let env = Env::new(false).await?;
    env.with_items("main").await?;
    env.add("main", item("x", "a", &[])).await?;
    env.fork("feature").await?;
    env.put("main", item("x", "b", &[])).await?;
    env.put("feature", item("x", "c", &[])).await?;
    env.conflict_and_resolve("feature", "x", ResolutionType::KeepTheirs)
        .await?;
    assert!(env.compound_ready("main"), "{BY_CAT} must stay Ready");
    assert_eq!(env.compound("main", "c", None).await?, ["x"]);
    assert!(env.compound("main", "a", None).await?.is_empty());
    assert!(env.compound("main", "b", None).await?.is_empty());
    Ok(())
}
