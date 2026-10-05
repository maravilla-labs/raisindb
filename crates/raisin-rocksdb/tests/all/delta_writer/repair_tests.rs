//! The `property_index` rebuild that gates skip-unchanged, and the sampling
//! `property_index_verify` job.

use super::env::{node, repair_options, Env, REPO, TENANT, WS};
use raisin_error::Result;
use raisin_rocksdb::management::async_indexing::repair::{
    invalidate_property_index_rebuild, load_state, property_index_rebuilt, run_repair, RepairKind,
    RepairOptions,
};
use raisin_rocksdb::{cf, keys};

async fn run(env: &Env, kind: RepairKind, options: RepairOptions) -> Result<(u64, u64, bool)> {
    let reports = run_repair(&env.storage, TENANT, REPO, Some("main"), kind, options).await?;
    let report = &reports[0];
    Ok((
        report.writes.written,
        report.property_index.missing,
        report.completed,
    ))
}

/// A node with a shared `kind = doc` entry (membership is re-stamped from
/// the NodeType on a local write, so the hole is made in an ordinary entry).
fn typed(id: &str) -> raisin_models::nodes::Node {
    node(id, &format!("/{id}"), &[("title", id), ("kind", "doc")])
}

/// An entry an older writer never wrote: the verify finds it, fails
/// the branch back to full puts and the rebuild puts it where it belongs.
#[tokio::test]
async fn verify_finds_a_hole_and_the_rebuild_fills_it() -> Result<()> {
    let env = Env::new(true).await?;
    for id in ["n1", "n2", "n3"] {
        env.add("main", typed(id)).await?;
    }
    let every = RepairOptions {
        sample_every: 1,
        ..repair_options()
    };
    assert_eq!(
        run(&env, RepairKind::PropertyIndexVerify, every.clone())
            .await?
            .1,
        0
    );

    // Remove one of n2's entries, as a writer that missed it left it.
    let r = env.newest_revision("main", "n2");
    let db = env.storage.db();
    let cf_prop = db.cf_handle(cf::PROPERTY_INDEX).unwrap();
    db.delete_cf(
        cf_prop,
        keys::property_index_key_versioned(
            TENANT, REPO, "main", WS, "kind", "doc", &r, "n2", false,
        ),
    )
    .unwrap();
    assert_eq!(env.indexed("main", "kind", "doc", None).await?.len(), 2);

    let (_, missing, completed) = run(&env, RepairKind::PropertyIndexVerify, every.clone()).await?;
    assert!(completed);
    assert_eq!(missing, 1);
    assert!(
        !property_index_rebuilt(db, TENANT, REPO, "main", "local"),
        "a verify miss must put the branch back on full puts"
    );

    let (written, _, completed) = run(&env, RepairKind::PropertyIndex, repair_options()).await?;
    assert!(completed);
    assert_eq!(written, 1, "only the missing entry is written");
    assert!(property_index_rebuilt(db, TENANT, REPO, "main", "local"));
    assert_eq!(env.indexed("main", "kind", "doc", None).await?.len(), 3);
    let (again, _, _) = run(&env, RepairKind::PropertyIndex, repair_options()).await?;
    assert_eq!(again, 0, "idempotent");
    assert_eq!(
        run(&env, RepairKind::PropertyIndexVerify, every).await?.1,
        0
    );
    Ok(())
}

/// The gate: the flag alone does not skip; this node's completed rebuild
/// does; invalidation (a checkpoint ingest, a verify miss) undoes it.
#[tokio::test]
async fn skip_needs_this_nodes_rebuild() -> Result<()> {
    let env = Env::new(false).await?;
    env.storage.nodes_impl().set_index_skip_unchanged(true);
    env.add("main", node("n", "/n", &[("a", "1"), ("b", "1")]))
        .await?;
    let update = |b: &'static str| node("n", "/n", &[("a", "1"), ("b", b)]);

    env.put("main", update("2")).await?;
    let full = env.property_keys_at("main", "n", &env.newest_revision("main", "n"));
    assert!(full > 4, "no rebuild yet: full put ({full})");

    env.rebuild("main").await?;
    env.put("main", update("3")).await?;
    assert_eq!(
        env.property_keys_at("main", "n", &env.newest_revision("main", "n")),
        4
    );

    invalidate_property_index_rebuild(env.storage.db(), TENANT, REPO, "main", "local")?;
    env.put("main", update("4")).await?;
    assert_eq!(
        env.property_keys_at("main", "n", &env.newest_revision("main", "n")),
        full
    );
    // A record under ANOTHER node id never counts.
    assert!(!property_index_rebuilt(
        env.storage.db(),
        TENANT,
        REPO,
        "main",
        "peer"
    ));
    Ok(())
}

/// Crash-mid-rebuild resume (operational gate): stop after one batch, resume
/// from the cursor, and end with what an uninterrupted run writes.
#[tokio::test]
async fn property_index_rebuild_resumes_after_crash() -> Result<()> {
    let env = Env::new(false).await?;
    for i in 0..40 {
        env.add("main", typed(&format!("n{i:02}"))).await?;
    }
    // Drop every `kind` entry, so the rebuild has 40 writes to make.
    let db = env.storage.db();
    let cf_prop = db.cf_handle(cf::PROPERTY_INDEX).unwrap();
    for i in 0..40 {
        let id = format!("n{i:02}");
        let r = env.newest_revision("main", &id);
        let key = keys::property_index_key_versioned(
            TENANT, REPO, "main", WS, "kind", "doc", &r, &id, false,
        );
        db.delete_cf(cf_prop, key).unwrap();
    }
    // What an uninterrupted run writes: the 40 holes, plus whatever the
    // workspace's own bootstrap nodes miss.
    let dry = RepairOptions {
        dry_run: true,
        ..repair_options()
    };
    let (expected, _, _) = run(&env, RepairKind::PropertyIndex, dry).await?;
    assert!(expected >= 40, "{expected}");
    let small = RepairOptions {
        batch_bytes: 512,
        stop_after_batches: Some(2),
        ..repair_options()
    };
    let (first, _, completed) = run(&env, RepairKind::PropertyIndex, small).await?;
    assert!(!completed && first > 0 && first < expected, "{first}");
    let state = load_state(db, TENANT, REPO, "main", "property_index", "local")?.unwrap();
    assert_eq!(state.status, "running");
    assert!(state.cursor.is_some());

    let reports = run_repair(
        &env.storage,
        TENANT,
        REPO,
        Some("main"),
        RepairKind::PropertyIndex,
        repair_options(),
    )
    .await?;
    assert!(reports[0].resumed && reports[0].completed);
    assert_eq!(
        first + reports[0].writes.written,
        expected,
        "resumed, not restarted"
    );
    assert_eq!(env.indexed("main", "kind", "doc", None).await?.len(), 40);
    Ok(())
}
