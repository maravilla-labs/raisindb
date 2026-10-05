//! Review findings on the skip gate's state and the verify job.

use super::env::{node, repair_options, Env, REPO, TENANT};
use raisin_context::MergeStrategy;
use raisin_error::Result;
use raisin_models::nodes::properties::PropertyValue;
use raisin_rocksdb::management::async_indexing::repair::{
    invalidate_property_index_rebuild, load_state, property_index_rebuilt, run_repair, CommitHook,
    RepairKind, RepairOptions,
};
use raisin_rocksdb::management::history_gc::{run_history_gc, GcOptions, HistoryRetention};
use raisin_storage::{BranchRepository, Storage};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

async fn verify_missing(env: &Env) -> Result<u64> {
    let reports = run_repair(
        &env.storage,
        TENANT,
        REPO,
        Some("main"),
        RepairKind::PropertyIndexVerify,
        RepairOptions {
            sample_every: 1,
            ..repair_options()
        },
    )
    .await?;
    assert!(reports[0].completed);
    Ok(reports[0].property_index.missing)
}

fn rebuilt(env: &Env, branch: &str) -> bool {
    property_index_rebuilt(env.storage.db(), TENANT, REPO, branch, "local")
}

/// `slug` unchanged across six updates lives only at the create revision;
/// history GC deletes that NODES version but keeps the entry. The verify
/// must not call it missing.
#[tokio::test]
async fn verify_after_history_gc_finds_no_hole() -> Result<()> {
    let env = Env::new(true).await?;
    env.add("main", node("n", "/n", &[("slug", "s"), ("title", "t0")]))
        .await?;
    for i in 1..=6 {
        let title = format!("t{i}");
        env.put("main", node("n", "/n", &[("slug", "s"), ("title", &title)]))
            .await?;
    }
    let options = GcOptions {
        retention_override: Some(HistoryRetention {
            keep_days: None,
            keep_revisions: Some(3),
        }),
        tenant: Some(TENANT.to_string()),
        repo: Some(REPO.to_string()),
        min_age: Duration::ZERO,
        collect_orphaned_blobs: false,
        bound_job_results: false,
        sweep_unreferenced_blobs: false,
        compact: false,
        ..GcOptions::default()
    };
    run_history_gc(&env.storage, &options)?;
    assert_eq!(env.indexed("main", "slug", "s", None).await?, ["n"]);
    assert_eq!(verify_missing(&env).await?, 0);
    assert!(rebuilt(&env, "main"), "a false miss reset the gate");
    Ok(())
}

/// Every production write stamps `$mixins: []`; stored, it decodes as an
/// empty `Vector`. The writer (in-memory form) and the verify (decoded form)
/// must derive the same entry.
#[tokio::test]
async fn verify_after_update_with_stamped_mixins_finds_no_hole() -> Result<()> {
    let env = Env::new(true).await?;
    let stamped = |title: &str| {
        let mut n = node("n", "/n", &[("title", title)]);
        n.properties.insert(
            raisin_models::nodes::RESERVED_MIXINS_KEY.into(),
            PropertyValue::Array(Vec::new()),
        );
        n
    };
    env.add("main", stamped("a")).await?;
    env.put("main", stamped("b")).await?;
    assert_eq!(verify_missing(&env).await?, 0);
    assert!(rebuilt(&env, "main"));
    Ok(())
}

async fn rebuild(
    env: &Env,
    options: RepairOptions,
) -> Result<raisin_rocksdb::management::async_indexing::repair::RepairReport> {
    Ok(run_repair(
        &env.storage,
        TENANT,
        REPO,
        Some("main"),
        RepairKind::PropertyIndex,
        options,
    )
    .await?
    .remove(0))
}

/// An invalidation (checkpoint ingest, verify miss) while a rebuild is in
/// flight, and while one is crashed waiting to resume, must not be lost.
#[tokio::test]
async fn invalidation_during_rebuild_is_not_lost() -> Result<()> {
    let env = Env::new(false).await?;
    let db = env.storage.db().clone();
    let cf_prop = db.cf_handle(raisin_rocksdb::cf::PROPERTY_INDEX).unwrap();
    for i in 0..30 {
        let id = format!("n{i:02}");
        env.add("main", node(&id, &format!("/{id}"), &[("k", "v")]))
            .await?;
        // A hole per node, so the rebuild commits several batches.
        let r = env.newest_revision("main", &id);
        db.delete_cf(
            cf_prop,
            raisin_rocksdb::keys::property_index_key_versioned(
                TENANT,
                REPO,
                "main",
                super::env::WS,
                "k",
                "v",
                &r,
                &id,
                false,
            ),
        )
        .unwrap();
    }
    // In flight: invalidated at the first (intermediate) batch commit.
    let fired = Arc::new(AtomicBool::new(false));
    let hook_fired = fired.clone();
    let hook_db = db.clone();
    let hook = CommitHook(Arc::new(move || {
        if !hook_fired.swap(true, Ordering::SeqCst) {
            invalidate_property_index_rebuild(&hook_db, TENANT, REPO, "main", "local").unwrap();
        }
    }));
    let report = rebuild(
        &env,
        RepairOptions {
            batch_bytes: 256,
            before_commit: Some(hook),
            ..repair_options()
        },
    )
    .await?;
    assert!(fired.load(Ordering::SeqCst));
    assert!(
        !report.completed && !rebuilt(&env, "main"),
        "invalidated run marked done"
    );
    assert!(rebuild(&env, repair_options()).await?.completed);
    assert!(rebuilt(&env, "main"));

    // Crashed mid-run, then invalidated: the next run restarts from scratch.
    for i in 0..30 {
        let id = format!("n{i:02}");
        let r = env.newest_revision("main", &id);
        db.delete_cf(
            cf_prop,
            raisin_rocksdb::keys::property_index_key_versioned(
                TENANT,
                REPO,
                "main",
                super::env::WS,
                "k",
                "v",
                &r,
                &id,
                false,
            ),
        )
        .unwrap();
    }
    let crashed = rebuild(
        &env,
        RepairOptions {
            batch_bytes: 256,
            stop_after_batches: Some(1),
            ..repair_options()
        },
    )
    .await?;
    assert!(!crashed.completed);
    let state = load_state(
        env.storage.db(),
        TENANT,
        REPO,
        "main",
        "property_index",
        "local",
    )?;
    assert_eq!(state.expect("state").status, "running");
    invalidate_property_index_rebuild(env.storage.db(), TENANT, REPO, "main", "local")?;
    let next = rebuild(&env, repair_options()).await?;
    assert!(
        !next.resumed && next.completed,
        "resumed past the invalidation"
    );
    assert!(rebuilt(&env, "main"));
    Ok(())
}

/// A merge replays the source's entries into the target: unless the source
/// was rebuilt too, the target goes back to full puts.
#[tokio::test]
async fn merge_from_unrebuilt_source_invalidates_target() -> Result<()> {
    let env = Env::new(true).await?;
    env.add("main", node("p", "/p", &[("title", "a")])).await?;
    env.fork("feature").await?;
    env.put("feature", node("p", "/p", &[("title", "b")]))
        .await?;
    let merged = env
        .storage
        .branches_impl()
        .merge_branches(
            TENANT,
            REPO,
            "main",
            "feature",
            MergeStrategy::ThreeWay,
            "m",
            "u",
        )
        .await?;
    assert!(merged.success, "{merged:?}");
    assert!(
        !rebuilt(&env, "main"),
        "unrebuilt source merged into a done target"
    );

    env.rebuild("main").await?;
    env.rebuild("feature").await?;
    env.put("feature", node("p", "/p", &[("title", "c")]))
        .await?;
    let merged = env
        .storage
        .branches_impl()
        .merge_branches(
            TENANT,
            REPO,
            "main",
            "feature",
            MergeStrategy::ThreeWay,
            "m2",
            "u",
        )
        .await?;
    assert!(merged.success, "{merged:?}");
    assert!(
        rebuilt(&env, "main"),
        "rebuilt source must keep the target done"
    );
    Ok(())
}

/// A branch deleted and re-created under the same name starts unrebuilt.
#[tokio::test]
async fn recreated_branch_does_not_inherit_rebuild_state() -> Result<()> {
    let env = Env::new(true).await?;
    env.fork("feature").await?;
    env.rebuild("feature").await?;
    env.storage
        .branches()
        .delete_branch(TENANT, REPO, "feature")
        .await?;
    env.fork("feature").await?;
    assert!(!rebuilt(&env, "feature"));
    Ok(())
}
