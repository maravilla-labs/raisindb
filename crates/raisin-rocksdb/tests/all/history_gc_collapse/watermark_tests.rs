//! The watermark: nothing at or above a revision that can still be written
//! is collapsed, and a node that may apply peers' operations never collapses.

use super::env::{node, options, Env, WS};
use super::race_tests::title_entries;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_rocksdb::cf;
use raisin_rocksdb::management::async_indexing::repair::RepairOptions;

fn round(title: &str, r: u32) -> raisin_models::nodes::Node {
    let mut n = node("n", "/n", title);
    n.properties
        .insert("round".to_string(), PropertyValue::String(r.to_string()));
    n
}

/// Revisions of `n`'s `title = v` PROPERTY_INDEX entries.
fn v_entries(env: &Env) -> Vec<HLC> {
    title_entries(env, "v")
        .into_iter()
        .map(|(r, _)| r)
        .collect()
}

fn watermark(report: &raisin_rocksdb::management::async_indexing::repair::RepairReport) -> HLC {
    report
        .collapse
        .watermark
        .as_deref()
        .unwrap()
        .parse()
        .unwrap()
}

#[tokio::test]
async fn late_commit_below_watermark_keeps_the_masking_entry() -> Result<()> {
    let env = Env::new().await?;
    env.add("main", node("n", "/n", "v")).await?; // v@r1
    env.put("main", round("v", 1)).await?; // v@r2
                                           // A session opens a transaction and writes: it allocates r3 and does not
                                           // commit (BEGIN ... UPDATE, then idle).
    let open = env.tx("main").await?;
    open.put_node(WS, &node("m", "/m", "x")).await?;
    let r4 = env.put("main", round("v", 2)).await?; // v@r4
    env.add("main", node("o", "/o", "other")).await?; // HEAD r5
    assert_eq!(v_entries(&env).len(), 3);

    let reports = env
        .collapse(
            Some("main"),
            RepairOptions {
                collapse_cfs: Some(vec![cf::PROPERTY_INDEX.to_string()]),
                ..options()
            },
        )
        .await?;
    let counts = &reports[0].collapse;
    assert!(reports[0].completed, "{counts:?}");
    let at = watermark(&reports[0]);
    assert!(
        at < r4,
        "the watermark {at} must stay below the open transaction's revision"
    );
    // v@r4 masks whatever the open transaction lands at r3: it stays. v@r2,
    // below r3, is a twin of v@r1 and goes.
    let left = v_entries(&env);
    assert!(left.contains(&r4), "{left:?}");
    assert_eq!(left.len(), 2, "{left:?}");

    // Committed, the revision is released: a later run may go further.
    open.commit().await?;
    drop(open);
    let again = env
        .collapse(
            Some("main"),
            RepairOptions {
                collapse_cfs: Some(vec![cf::PROPERTY_INDEX.to_string()]),
                ..options()
            },
        )
        .await?;
    assert!(watermark(&again[0]) > r4);
    assert_eq!(v_entries(&env).len(), 1);
    Ok(())
}

#[tokio::test]
async fn cluster_settings_without_replication_enabled_refuse_collapse() -> Result<()> {
    // A node id and a replication port start the coordinator even with
    // `replication.enabled = false` (capture off): this node applies peers'
    // ops at their original revisions, so it must refuse like a cluster.
    let env = Env::with_config(|c| {
        c.with_history_gc_collapse_runs(true)
            .with_replication_configured(true)
    })
    .await?;
    env.put("main", round("v", 1)).await?;
    env.put("main", round("v", 2)).await?;
    assert!(!env.storage.config().replication_enabled);
    let err = env
        .collapse(Some("main"), options())
        .await
        .expect_err("a replicating node has no watermark");
    assert!(err.to_string().contains("watermark"), "{err}");
    Ok(())
}
