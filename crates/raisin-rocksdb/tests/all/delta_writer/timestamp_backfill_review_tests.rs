//! Plan Phase 13g review: the `timestamp_backfill` repair must not cut a
//! branch's revision history, revert an edit it raced, hand a UNIQUE claim
//! to a legacy duplicate, or leave a branch unrepaired that a later legacy
//! version (or a checkpoint ingest during the run) made owe it again.

use super::builtin_index_tests::born;
use super::env::{node, Env, REPO, TENANT};
use super::node_types::register_type;
use super::replica_tests::{applicator, upsert};
use super::timestamp_backfill_tests::{legacy, rev, time_of, versions};
use raisin_context::MergeStrategy;
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_replication::OpType;
use raisin_rocksdb::indexing::node_lock::test_hooks::pause_commit_of;
use raisin_rocksdb::management::async_indexing::repair::{
    load_state, mark_repairs_pending_on, pending_timestamp_backfill_branches, repair_node_id,
    request_compound_builds_after_backfill, RepairKind,
};
use raisin_rocksdb::OpLogRepository;
use raisin_storage::{
    BranchRepository, NodeRepository, RevisionRepository, Storage, UpdateNodeOptions,
};
use std::time::Duration;

fn no_validation() -> UpdateNodeOptions {
    UpdateNodeOptions {
        validate_schema: false,
        ..Default::default()
    }
}

fn title(node: &Node) -> Option<&PropertyValue> {
    node.properties.get("title")
}

async fn head(env: &Env, branch: &str) -> Result<HLC> {
    env.storage.branches().get_head(TENANT, REPO, branch).await
}

/// The finding: every backfill revision moved the HEAD without a revision
/// record, so the next commit's parent had none, and the merge base of a
/// branch pair fell back to `HLC(0,0)` — the merge then carried nothing. A
/// backfill now rewrites in place (the HEAD stays), so both branches keep
/// their fork point and a merge carries the feature's commits.
#[tokio::test]
async fn the_backfill_on_both_branches_keeps_the_merge_base() -> Result<()> {
    let env = Env::new(false).await?;
    legacy(&env, "a", rev(&env), |_| {}).await;
    env.add("main", node("base", "/base", &[])).await?;
    let fork_point = head(&env, "main").await?;
    env.fork("feature").await?;
    env.add("feature", node("f", "/f", &[])).await?;

    assert_eq!(env.backfill("main").await?.timestamps.backfilled, 1);
    env.backfill("feature").await?;
    env.add("main", node("m", "/m", &[])).await?;

    let divergence = env
        .storage
        .branches()
        .calculate_divergence(TENANT, REPO, "feature", "main")
        .await?;
    assert_eq!(divergence.common_ancestor, fork_point, "the fork point");
    let merged = env
        .storage
        .branches_impl()
        .merge_branches(
            TENANT,
            REPO,
            "main",
            "feature",
            MergeStrategy::ThreeWay,
            "merge",
            "test-user",
        )
        .await?;
    assert!(merged.conflicts.is_empty(), "{:?}", merged.conflicts);
    assert!(
        env.storage
            .nodes()
            .get(env.scope("main"), "f", None)
            .await?
            .is_some(),
        "the feature's commit is merged"
    );
    Ok(())
}

/// The same defect on the funnel's edit path (publishing, `update_property`,
/// `NodeRepository::update`): every revision it moves the HEAD to now has a
/// record whose parent is the HEAD it replaced and which lists the node.
#[tokio::test]
async fn a_repository_update_records_its_revision_and_keeps_the_merge_base() -> Result<()> {
    let env = Env::new(false).await?;
    env.add("main", node("x", "/x", &[("title", "1")])).await?;
    let fork_point = head(&env, "main").await?;
    env.fork("feature").await?;
    env.storage
        .nodes()
        .update(
            env.scope("main"),
            node("x", "/x", &[("title", "2")]),
            no_validation(),
        )
        .await?;
    let updated = head(&env, "main").await?;
    assert!(updated > fork_point, "the update moved the HEAD");
    let meta = env
        .storage
        .revisions()
        .get_revision_meta(TENANT, REPO, &updated)
        .await?
        .expect("a revision record for the update's revision");
    assert_eq!(meta.parent, Some(fork_point));
    let changed: Vec<_> = meta
        .changed_nodes
        .iter()
        .map(|c| c.node_id.as_str())
        .collect();
    assert_eq!(changed, ["x"]);
    env.add("main", node("y", "/y", &[])).await?;
    let divergence = env
        .storage
        .branches()
        .calculate_divergence(TENANT, REPO, "feature", "main")
        .await?;
    assert_eq!(divergence.common_ancestor, fork_point);
    Ok(())
}

/// The finding: a backfill at a FRESH revision put the content it read back
/// over an edit with a lower revision — a peer's edit this node had not
/// applied yet, then replicated everywhere. In place at the read version's
/// revision, the edit (always above it) wins on the origin and the replica,
/// whatever the arrival order.
#[tokio::test]
async fn a_late_edit_wins_over_the_backfill_on_every_node() -> Result<()> {
    let origin = Env::new_with(false, true).await?;
    let replica = Env::new(false).await?;
    let r1 = rev(&origin);
    legacy(&origin, "a", r1, |_| {}).await;
    legacy(&replica, "a", r1, |_| {}).await;
    // A peer's edit, after the legacy version: the replica has applied it,
    // the origin has not yet.
    let edited = node("a", "/a", &[("title", "edited")]);
    let r_edit = rev(&origin);
    upsert(&applicator(&replica), &edited, "aa", r_edit).await;

    assert_eq!(origin.backfill("main").await?.timestamps.backfilled, 1);
    assert_eq!(versions(&origin, "main", "a"), [r1], "in place");
    let op = OpLogRepository::new(origin.storage.db().clone())
        .get_all_operations(TENANT, REPO)?
        .into_values()
        .flatten()
        .find(|op| {
            matches!(&op.op_type, OpType::ApplyRevision { node_changes, .. }
            if node_changes.iter().any(|c| c.node.id == "a"))
        })
        .expect("the backfill's ApplyRevision");
    upsert(&applicator(&origin), &edited, "aa", r_edit).await;
    applicator(&replica)
        .apply_operation(&op)
        .await
        .expect("apply the backfill");

    for env in [&origin, &replica] {
        let a = env.stored("a").await?;
        assert_eq!(
            title(&a),
            Some(&PropertyValue::String("edited".into())),
            "the edit is the newest version"
        );
    }
    let backfilled = replica
        .storage
        .nodes()
        .get(replica.scope("main"), "a", Some(&r1))
        .await?
        .expect("the version at r1");
    assert_eq!(
        backfilled.created_at,
        Some(time_of(&r1)),
        "r1 carries the fill"
    );
    Ok(())
}

/// The finding: the funnel's read-modify-write window. An in-place
/// (`versionable: false`) edit commits after the backfill read the node: the
/// backfill's conditional commit finds the node changed and writes nothing
/// (it used to overwrite the edit at the same revision); the next run fills
/// the timestamps over the edit.
#[tokio::test]
async fn a_backfill_does_not_overwrite_an_edit_committed_after_its_read() -> Result<()> {
    let env = Env::new(false).await?;
    register_type(&env.storage, "main", "test:Health", None, Some(false)).await?;
    legacy(&env, "h", rev(&env), |n| n.node_type = "test:Health".into()).await;
    let mut edited = node("h", "/h", &[("title", "new")]);
    edited.node_type = "test:Health".into();

    let pause = pause_commit_of(env.storage.db(), "h");
    let edit = env
        .storage
        .nodes()
        .update(env.scope("main"), edited, no_validation());
    let backfill = async {
        pause.reached().await;
        let mut run = Box::pin(env.backfill("main"));
        let early = tokio::time::timeout(Duration::from_millis(400), &mut run).await;
        pause.release();
        match early {
            Ok(report) => Ok::<_, raisin_error::Error>((true, report?)),
            Err(_) => Ok((false, run.await?)),
        }
    };
    let (edit, backfill) = tokio::join!(edit, backfill);
    edit?;
    let (finished_early, report) = backfill?;
    assert!(!finished_early, "the backfill waits for the edit's commit");
    let t = &report.timestamps;
    assert_eq!((t.backfilled, t.unchanged, t.failed), (0, 1, 0));
    assert_eq!(
        title(&env.stored("h").await?),
        Some(&PropertyValue::String("new".into())),
        "the edit survives"
    );

    assert_eq!(env.backfill("main").await?.timestamps.backfilled, 1);
    let h = env.stored("h").await?;
    assert_eq!(title(&h), Some(&PropertyValue::String("new".into())));
    assert!(h.created_at.is_some());
    Ok(())
}

/// The finding: the backfill re-put every UNIQUE claim of the version it
/// wrote, so a legacy duplicate took the value from the node holding it, and
/// every later edit of the owner failed as a violation.
#[tokio::test]
async fn legacy_unique_duplicates_keep_their_owner_after_the_backfill() -> Result<()> {
    let env = Env::new(false).await?;
    register_type(&env.storage, "main", "test:User", Some("email"), None).await?;
    let user = |n: &mut Node| {
        n.node_type = "test:User".into();
        n.properties
            .insert("email".into(), PropertyValue::String("x@y".into()));
    };
    // `b` first, so `a` (scanned first) holds the newest claim.
    legacy(&env, "b", rev(&env), user).await;
    legacy(&env, "a", rev(&env), user).await;

    assert_eq!(env.backfill("main").await?.timestamps.backfilled, 2);
    let mut a = env.stored("a").await?;
    a.properties
        .insert("title".into(), PropertyValue::String("renamed".into()));
    env.storage
        .nodes()
        .update(env.scope("main"), a, no_validation())
        .await
        .expect("the owner of the value can still be edited");
    Ok(())
}

/// The finding: a branch whose backfill was `done` was never owed it again,
/// so a legacy version written after it (replicated, imported) kept the
/// built-in index refused for good. A refused `compound_builds` link now
/// makes the branch pending.
#[tokio::test]
async fn a_refused_build_makes_the_branch_owe_the_backfill_again() -> Result<()> {
    let env = Env::new(false).await?;
    legacy(&env, "a", rev(&env), |_| {}).await;
    assert_eq!(env.backfill("main").await?.timestamps.backfilled, 1);
    let before = pending_timestamp_backfill_branches(&env.storage)?;
    legacy(&env, "late", rev(&env), |_| {}).await;
    assert_eq!(env.compound_builds("main").await?.compound.refused, 1);
    let after = pending_timestamp_backfill_branches(&env.storage)?;
    assert!(before.is_empty(), "done and nothing refused: {before:?}");
    assert_eq!(
        after,
        [(TENANT.to_string(), REPO.to_string(), "main".to_string())]
    );
    Ok(())
}

/// The finding: a checkpoint ingest's `queued` mark during a run was
/// overwritten by the run's `done`, and the legacy versions it brought in
/// below the run's cursor were never filled. The mark now wins: the run
/// scans the branch again.
#[tokio::test]
async fn a_pending_mark_during_a_run_rescans_the_branch() -> Result<()> {
    let env = Env::new(false).await?;
    for id in ["a", "b", "c"] {
        legacy(&env, id, rev(&env), |_| {}).await;
    }
    let node_id = repair_node_id(&env.storage);
    let pause = pause_commit_of(env.storage.db(), "b");
    let run = env.backfill("main");
    let ingest = async {
        pause.reached().await;
        // A version the run has already passed, and the ingest's mark.
        legacy(&env, "a1", rev(&env), |_| {}).await;
        mark_repairs_pending_on(
            env.storage.db(),
            TENANT,
            REPO,
            "main",
            &node_id,
            &[RepairKind::TimestampBackfill],
        )?;
        pause.release();
        Ok::<_, raisin_error::Error>(())
    };
    let (report, ingest) = tokio::join!(run, ingest);
    ingest?;
    let report = report?;
    assert!(report.completed);
    assert_eq!(report.timestamps.backfilled, 4, "a, b, c, then a1");
    assert!(env.stored("a1").await?.created_at.is_some());
    let state = load_state(
        env.storage.db(),
        TENANT,
        REPO,
        "main",
        RepairKind::TimestampBackfill.slug(),
        &node_id,
    )?
    .expect("state");
    assert_eq!(state.status, "done");
    Ok(())
}

/// The findings: a backfill that wrote nothing re-requested the link (and
/// past a FAILED link's fingerprint); one that did write could not reach an
/// index nothing recorded as owed (a per-index job's quiet refusal). Now: no
/// write, no request; a write, a link whatever the record owes.
#[tokio::test]
async fn the_link_is_requested_exactly_when_the_backfill_wrote() -> Result<()> {
    let env = Env::new(false).await?;
    env.add("main", born("w", "/w", 5)).await?;
    assert!(env.compound_builds("main").await?.completed);
    let report = env.backfill("main").await?;
    assert_eq!(report.timestamps.backfilled, 0);
    assert_eq!(
        env.queued_compound_links("main").await,
        0,
        "nothing written"
    );
    assert_eq!(
        request_compound_builds_after_backfill(&env.storage, TENANT, REPO, "main").await?,
        1,
        "queued although the record owes nothing"
    );
    Ok(())
}
