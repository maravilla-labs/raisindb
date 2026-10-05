//! Review fixes for plan Phase 13f's built-in `(parent, created_at)` index:
//! a fork below the build floor, a legacy child without `created_at`, the
//! targeted retry of a failed branch, the admin rebuild's split precheck and
//! keyspace lock, the precheck against the build's own output, and the link's
//! re-check of its work.

use super::builtin_index_tests::born;
use super::compound_env::{by_cat, item, ITEM};
use super::env::{node, Env, REPO, TENANT, WS};
use super::node_types::register_type_with;
use super::replica_tests::{applicator, upsert_as_stored};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::workspace::builtin_indexes::children_by_created_at_stored_name;
use raisin_rocksdb::compound_state::CompoundStateStore;
use raisin_rocksdb::indexing::compound::{build, keyspace, workspace_defs};
use raisin_rocksdb::indexing::IndexCtx;
use raisin_rocksdb::management::async_indexing::rebuild_indexes;
use raisin_rocksdb::management::async_indexing::repair::{
    check_headroom_assuming, enqueue_compound_builds_if_owed,
};
use raisin_storage::compound::CompoundStateSource;
use raisin_storage::{BranchRepository, IndexType, Storage};
use std::time::Duration;

fn later(offset_ms: u64) -> HLC {
    HLC::new(chrono::Utc::now().timestamp_millis() as u64 + offset_ms, 0)
}

/// A root-level child written before `created_at` was stamped: replication
/// carries a node as its origin stored it, so a replicated legacy version is
/// exactly such a node on this one.
async fn legacy_child(env: &Env) -> Result<()> {
    env.add("main", born("w", "/w", 5)).await?; // warms test:Page
    let legacy = node("legacy", "/legacy", &[]);
    assert!(legacy.created_at.is_none());
    upsert_as_stored(&applicator(env), &legacy, "a1", later(1_000)).await;
    Ok(())
}

/// A fork taken BELOW the source's build floor (a tag from before the
/// automatic build) used to inherit `Ready`: the copy keeps only entries at
/// or below the fork revision, and the build wrote `x` only at its newest
/// version (above it), so the fork's listing silently left `x` out.
#[tokio::test]
async fn a_fork_below_the_build_floor_does_not_inherit_ready() -> Result<()> {
    let env = Env::new(false).await?;
    env.add("main", born("docs", "/docs", 0)).await?;
    env.add("main", born("x", "/docs/x", 10)).await?;
    let fork_at = env.newest_revision("main", "x");
    let mut x = born("x", "/docs/x", 10);
    x.properties.insert(
        "title".to_string(),
        PropertyValue::String("later".to_string()),
    );
    env.put("main", x).await?;
    env.compound_builds("main").await?;
    assert!(env.builtin_ready("main"));

    env.storage
        .branches()
        .create_branch(
            TENANT,
            REPO,
            "tag",
            "test-user",
            Some(fork_at),
            Some("main".to_string()),
            false,
            false,
        )
        .await?;
    assert!(
        !env.builtin_ready("tag"),
        "the copy below the floor is incomplete: it must not be inherited Ready"
    );
    assert_eq!(env.pending_compound_builds().await?, ["tag"]);
    assert_eq!(env.compound_builds("tag").await?.compound.built, 1);
    assert!(env.builtin_ready("tag"));
    assert_eq!(env.children("tag", "/docs").await?, ["x"]);

    // A fork at HEAD still inherits it.
    env.fork("feature").await?;
    assert!(env.builtin_ready("feature"));
    Ok(())
}

/// A child with no `created_at` (legacy data) has no built-in entry, so a
/// `Ready` index would list the folder without it while the row scan lists
/// it with a NULL. The build refuses instead; the planner keeps scanning.
/// Since plan Phase 13g the refusal is an expected state recorded on the
/// link (no error, no job retries); `timestamp_backfill_tests` resolves it.
#[tokio::test]
async fn a_child_without_created_at_is_not_dropped_from_the_builtin_listing() -> Result<()> {
    let env = Env::new(false).await?;
    legacy_child(&env).await?;
    let link = env.compound_builds("main").await?;
    assert_eq!(
        (
            link.compound.refused,
            link.compound.refused_missing_order_values
        ),
        (1, 1),
        "the build must refuse a node it cannot list"
    );
    assert!(!env.builtin_ready("main"), "the index stays failed closed");
    Ok(())
}

/// A branch whose link failed (or, since plan Phase 13g, was refused) used
/// to be re-linked by every targeted request (each workspace event, each
/// cold drain): a full workspace rescan that ended the same way,
/// indefinitely. Now it waits for the next start — unless the work it owes
/// changed.
#[tokio::test]
async fn a_failed_branch_is_not_relinked_on_every_request() -> Result<()> {
    let env = Env::new(false).await?;
    legacy_child(&env).await?;
    assert_eq!(env.compound_builds("main").await?.compound.refused, 1);
    assert_eq!(
        enqueue_compound_builds_if_owed(&env.storage, TENANT, REPO, "main").await?,
        0,
        "the same owed work failed last time: retried at the next start"
    );
    // The operator opts the workspace out: different work, linked at once.
    env.builtin(false).await?;
    assert_eq!(
        enqueue_compound_builds_if_owed(&env.storage, TENANT, REPO, "main").await?,
        1
    );
    Ok(())
}

/// The admin `REBUILD … compound` prechecked the NodeType and workspace
/// indexes as ONE pass, so since every workspace carries the built-in index,
/// one node the built-in cannot hold refused the NodeType indexes' rebuild
/// too. The passes are prechecked apart now.
#[tokio::test]
async fn an_unindexable_node_only_refuses_the_indexes_that_want_it() -> Result<()> {
    let env = Env::new(false).await?;
    register_type_with(&env.storage, "main", ITEM, None, None, Some(vec![by_cat()])).await?;
    env.add("main", item("i", "a", &[])).await?;
    legacy_child(&env).await?;
    assert!(!env.compound_ready("main"));

    let rebuilt =
        rebuild_indexes(&env.storage, TENANT, REPO, "main", WS, IndexType::Compound).await;
    assert!(rebuilt.is_err(), "the built-in's refusal is still reported");
    assert!(env.compound_ready("main"), "the NodeType index was rebuilt");
    assert_eq!(env.compound("main", "a", None).await?, ["i"]);
    assert!(!env.builtin_ready("main"));
    Ok(())
}

/// The admin rebuild did not take the keyspace lock, so it could clear the
/// keyspace an automatic build was writing (and both stamp `Ready`). It now
/// queues behind any builder of the same index.
#[tokio::test]
async fn the_admin_rebuild_queues_behind_a_build_of_the_same_index() -> Result<()> {
    let env = Env::new(false).await?;
    env.add("main", born("f", "/f", 0)).await?;
    env.add("main", born("c1", "/f/c1", 10)).await?;
    let held = keyspace::lock(
        env.storage.db(),
        TENANT,
        REPO,
        "main",
        WS,
        &children_by_created_at_stored_name(),
    )
    .await;
    let admin = rebuild_indexes(&env.storage, TENANT, REPO, "main", WS, IndexType::Compound);
    tokio::pin!(admin);
    assert!(
        tokio::time::timeout(Duration::from_millis(300), &mut admin)
            .await
            .is_err(),
        "the admin rebuild must wait for the build holding the keyspace"
    );
    drop(held);
    admin.await?;
    assert!(env.builtin_ready("main"));
    assert_eq!(env.children("main", "/f").await?, ["c1"]);
    Ok(())
}

/// The precheck's only disk rule was 2x the CURRENT compound column family,
/// which says nothing about an index never built before — near zero on a
/// database with few compound indexes, so a volume with almost no free space
/// passed. It now also sizes free space against the build's own output.
#[tokio::test]
async fn a_build_refuses_when_the_volume_cannot_take_its_output() -> Result<()> {
    let env = Env::new(false).await?;
    env.add("main", born("f", "/f", 0)).await?;
    env.add("main", born("c1", "/f/c1", 10)).await?;
    let db = env.storage.db();
    let wanted = build::Wanted::every_type(workspace_defs::current(db, TENANT, REPO, WS)?.to_vec());
    let ctx = IndexCtx::new(TENANT, REPO, "main", WS);
    let floor = later(10_000);
    let tiny = 1 << 20;
    check_headroom_assuming(db, raisin_rocksdb::cf::COMPOUND_INDEX, Some(tiny))?;
    assert!(
        build::precheck_assuming(db, &ctx, &wanted, &floor, Some(tiny)).is_err(),
        "1 MiB free must not pass for a build that has to leave the floor free"
    );
    build::precheck_assuming(db, &ctx, &wanted, &floor, Some(u64::MAX / 4))?;
    Ok(())
}

/// A request that arrives while a link of its branch is live queues nothing,
/// and the link used to save `done` from the work list it read at its start:
/// a workspace created meanwhile kept scanning until the next restart. The
/// link now re-reads its work before it finishes.
#[tokio::test]
async fn a_workspace_created_during_a_link_is_built_by_that_link() -> Result<()> {
    let env = Env::new(false).await?;
    env.add("main", born("f", "/f", 0)).await?;
    let builtin = children_by_created_at_stored_name();
    let held = keyspace::lock(env.storage.db(), TENANT, REPO, "main", WS, &builtin).await;
    let link = env.compound_builds("main");
    tokio::pin!(link);
    assert!(
        tokio::time::timeout(Duration::from_millis(200), &mut link)
            .await
            .is_err(),
        "the link waits on the held keyspace"
    );
    let mut zeta = raisin_models::workspace::Workspace::new("zeta".to_string());
    zeta.config.default_branch = "main".to_string();
    raisin_core::services::workspace_service::WorkspaceService::new(env.storage.clone())
        .put(TENANT, REPO, zeta)
        .await?;
    drop(held);
    let report = link.await?;
    assert_eq!(
        report.compound.built, 2,
        "content, then zeta on the re-check"
    );
    let declared = workspace_defs::current(env.storage.db(), TENANT, REPO, "zeta")?;
    let def = declared
        .iter()
        .find(|d| d.name == builtin)
        .expect("zeta carries the built-in");
    assert!(CompoundStateStore::new(env.storage.db().clone())
        .compound_availability(TENANT, REPO, "main", "zeta", def)
        .is_ready());
    assert!(env.pending_compound_builds().await?.is_empty());
    Ok(())
}

/// Writers maintain the built-in keyspace whenever it is declared, record or
/// not; a drop was found only from STATE RECORDS, so an index switched off
/// before any build registered here (before the chain reached it, or after a
/// refused precheck) left its entries in place for good.
#[tokio::test]
async fn an_index_switched_off_before_its_first_build_is_still_dropped() -> Result<()> {
    let env = Env::new(false).await?;
    env.add("main", born("f", "/f", 0)).await?;
    env.add("main", born("c1", "/f/c1", 10)).await?;
    assert!(env.builtin_entries("main") > 0, "the writers maintained it");
    env.builtin(false).await?;
    assert_eq!(env.pending_compound_builds().await?, ["main"]);
    let report = env.compound_builds("main").await?;
    assert_eq!(report.compound.dropped, 1);
    assert_eq!(env.builtin_entries("main"), 0);
    assert!(env.pending_compound_builds().await?.is_empty());
    Ok(())
}
