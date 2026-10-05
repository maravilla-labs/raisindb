//! Plan Phase 13g: the `timestamp_backfill` repair gives nodes written before
//! `created_at`/`updated_at` were stamped the timestamps their history
//! implies, through the write funnel — so the built-in
//! `(__parent_path, __created_at)` index, which `compound_builds` refuses
//! (quietly, as an expected state) while such nodes exist, can be built.

use super::builtin_index_tests::{at, born};
use super::env::{node, repair_options, Env, REPO, TENANT, WS};
use super::node_types::register_type;
use super::replica_tests::{applicator, upsert_as_stored};
use chrono::{DateTime, Utc};
use raisin_error::Result;
use raisin_events::{Event, EventHandler};
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_replication::OpType;
use raisin_rocksdb::management::async_indexing::repair::{
    enqueue_compound_builds_if_owed, load_state, override_timestamp_backfill,
    pending_timestamp_backfill_branches, repair_node_id, request_compound_builds_after_backfill,
    run_repair, start_chain, RepairKind, RepairOptions, RepairReport,
    COMPOUND_BUILDS_REFUSED_STATUS,
};
use raisin_rocksdb::{cf, keys, OpLogRepository};
use raisin_storage::jobs::JobType;
use raisin_storage::{NodeRepository, RevisionRepository, Storage};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

/// A fresh revision from `env`'s allocator, as the origin's write took it —
/// so the writes that follow (a delete, an edit) land above it.
pub(super) fn rev(env: &Env) -> HLC {
    std::thread::sleep(std::time::Duration::from_millis(5));
    env.storage.revisions().allocate_revision()
}

pub(super) fn time_of(revision: &HLC) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(revision.timestamp_ms as i64).unwrap()
}

/// A version of `id` at `/{id}` as an origin stored it BEFORE the write layer
/// stamped timestamps (replication carries it verbatim).
pub(super) async fn legacy(env: &Env, id: &str, revision: HLC, edit: impl FnOnce(&mut Node)) {
    let mut n = node(id, &format!("/{id}"), &[("title", id)]);
    edit(&mut n);
    upsert_as_stored(&applicator(env), &n, &format!("a{id}"), revision).await;
}

/// Every stored revision of `id` on `branch`, newest first.
pub(super) fn versions(env: &Env, branch: &str, id: &str) -> Vec<HLC> {
    let prefix = keys::node_key_prefix(TENANT, REPO, branch, WS, id);
    let db = env.storage.db();
    let cf = db.cf_handle(cf::NODES).unwrap();
    db.iterator_cf(
        cf,
        rocksdb::IteratorMode::From(&prefix, rocksdb::Direction::Forward),
    )
    .map(|item| item.unwrap().0)
    .take_while(|key| key.starts_with(&prefix))
    .map(|key| keys::extract_revision_from_key(&key).unwrap())
    .collect()
}

impl Env {
    pub(super) async fn backfill_with(
        &self,
        branch: &str,
        options: RepairOptions,
    ) -> Result<RepairReport> {
        let mut reports = run_repair(
            &self.storage,
            TENANT,
            REPO,
            Some(branch),
            RepairKind::TimestampBackfill,
            options,
        )
        .await?;
        Ok(reports.remove(0))
    }

    pub(super) async fn backfill(&self, branch: &str) -> Result<RepairReport> {
        self.backfill_with(branch, repair_options()).await
    }

    pub(super) async fn stored(&self, id: &str) -> Result<Node> {
        Ok(self
            .storage
            .nodes()
            .get(self.scope("main"), id, None)
            .await?
            .expect("node"))
    }

    /// Live `compound_builds` jobs queued for `branch` (no worker runs here).
    pub(super) async fn queued_compound_links(&self, branch: &str) -> usize {
        self.storage
            .job_registry()
            .list_jobs()
            .await
            .into_iter()
            .filter(|job| {
                matches!(&job.job_type, JobType::IndexRepair { repair, branch: Some(b), .. }
                    if repair == "compound_builds" && b == branch)
            })
            .count()
    }
}

/// Every node event the storage's bus publishes.
struct Recorder(Arc<Mutex<Vec<String>>>);

impl EventHandler for Recorder {
    fn name(&self) -> &str {
        "timestamp_backfill_recorder"
    }

    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if let Event::Node(e) = event {
                self.0
                    .lock()
                    .unwrap()
                    .push(format!("{:?} {}", e.kind, e.node_id));
            }
            Ok(())
        })
    }
}

/// The scenario of the production refusal: legacy nodes block the built-in
/// index (recorded, no error); the backfill stamps them from their history
/// through the write funnel — in place at the version's revision, nothing
/// else changed, no node event — re-requests the link, and the index then
/// builds and lists them.
#[tokio::test]
async fn legacy_nodes_get_their_history_timestamps_and_the_builtin_index_builds() -> Result<()> {
    let env = Env::new(false).await?;
    env.add("main", born("w", "/w", 5)).await?;
    let (r1, r2, r3) = (rev(&env), rev(&env), rev(&env));
    legacy(&env, "a", r1, |_| {}).await;
    legacy(&env, "a", r2, |_| {}).await;
    legacy(&env, "b", r3, |n| n.updated_at = Some(at(7))).await;

    // Refused, quietly: a completed link, the state recorded.
    let link = env.compound_builds("main").await?;
    assert!(link.completed, "an expected refusal is not a failure");
    assert_eq!(
        (
            link.compound.refused,
            link.compound.refused_missing_order_values
        ),
        (1, 2)
    );
    assert!(!env.builtin_ready("main"));
    let node_id = repair_node_id(&env.storage);
    let state = load_state(
        env.storage.db(),
        TENANT,
        REPO,
        "main",
        "compound_builds",
        &node_id,
    )?
    .expect("state");
    assert_eq!(state.status, COMPOUND_BUILDS_REFUSED_STATUS);
    assert_eq!(state.refused_missing_order_values, Some(2));

    let events = Arc::new(Mutex::new(Vec::new()));
    env.storage
        .event_bus()
        .subscribe(Arc::new(Recorder(events.clone())));
    let a_before = env.stored("a").await?;
    let report = env.backfill("main").await?;
    assert!(report.completed);
    let t = &report.timestamps;
    assert_eq!((t.missing, t.backfilled, t.failed), (2, 2, 0));
    assert!(t.nodes >= 3, "every live node is looked at");

    let a = env.stored("a").await?;
    assert_eq!(
        a.created_at,
        Some(time_of(&r1)),
        "the FIRST revision's time"
    );
    assert_eq!(
        a.updated_at,
        Some(time_of(&r2)),
        "the newest revision's time"
    );
    assert_eq!(a.properties, a_before.properties, "no property changed");
    assert_eq!(a.version, a_before.version, "a backfill is not an edit");
    assert_eq!(
        versions(&env, "main", "a"),
        [r2, r1],
        "rewritten in place at the newest version's revision"
    );
    let b = env.stored("b").await?;
    assert_eq!(b.created_at, Some(time_of(&r3)));
    assert_eq!(b.updated_at, Some(at(7)), "a stored updated_at is kept");
    assert_eq!(
        versions(&env, "main", "w").len(),
        1,
        "complete nodes untouched"
    );
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(
        events.lock().unwrap().is_empty(),
        "a backfill fires no node event: {:?}",
        events.lock().unwrap()
    );
    // The recorder does hear node events (a replicated write publishes one).
    legacy(&env, "probe", rev(&env), |_| {}).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(!events.lock().unwrap().is_empty(), "the recorder is live");
    env.storage
        .nodes()
        .delete(env.scope("main"), "probe", Default::default())
        .await?;

    // The finished branch re-requested its link past the refusal.
    assert_eq!(env.queued_compound_links("main").await, 1);
    let link = env.compound_builds("main").await?;
    assert_eq!((link.compound.built, link.compound.refused), (1, 0));
    assert!(env.builtin_ready("main"));
    assert_eq!(env.children("main", "/").await?, ["b", "a", "w"]);

    // Idempotent: nothing missing, nothing written.
    let again = env.backfill("main").await?;
    assert_eq!(
        (again.timestamps.missing, again.timestamps.backfilled),
        (0, 0)
    );
    assert_eq!(versions(&env, "main", "a").len(), 2);
    Ok(())
}

/// A `versionable: false` node is rewritten in place (no new revision); a
/// deleted legacy node is left alone.
#[tokio::test]
async fn versionable_false_is_rewritten_in_place_and_deleted_nodes_are_skipped() -> Result<()> {
    let env = Env::new(false).await?;
    register_type(&env.storage, "main", "test:Health", None, Some(false)).await?;
    let r1 = rev(&env);
    legacy(&env, "h", r1, |n| n.node_type = "test:Health".to_string()).await;
    legacy(&env, "d", rev(&env), |_| {}).await;
    env.storage
        .nodes()
        .delete(env.scope("main"), "d", Default::default())
        .await?;
    let deleted_versions = versions(&env, "main", "d");

    let report = env.backfill("main").await?;
    let t = &report.timestamps;
    assert_eq!(
        (t.missing, t.backfilled),
        (1, 1),
        "the deleted node is skipped"
    );
    assert_eq!(versions(&env, "main", "h"), [r1], "rewritten in place");
    let h = env.stored("h").await?;
    assert_eq!(h.created_at, Some(time_of(&r1)));
    assert_eq!(h.updated_at, Some(time_of(&r1)));
    assert_eq!(
        versions(&env, "main", "d"),
        deleted_versions,
        "deleted: untouched"
    );
    Ok(())
}

/// The backfill replicates like any write: a replica that holds the same
/// legacy version applies the captured `ApplyRevision` and gets the same
/// timestamps (and can then build its own built-in index).
#[tokio::test]
async fn the_backfill_replicates_to_a_replica() -> Result<()> {
    let origin = Env::new_with(false, true).await?;
    let replica = Env::new(false).await?;
    let r1 = rev(&origin);
    legacy(&origin, "a", r1, |_| {}).await;
    legacy(&replica, "a", r1, |_| {}).await;

    assert_eq!(origin.backfill("main").await?.timestamps.backfilled, 1);
    let mut captured: Vec<_> = OpLogRepository::new(origin.storage.db().clone())
        .get_all_operations(TENANT, REPO)?
        .into_values()
        .flatten()
        .filter(|op| match &op.op_type {
            OpType::ApplyRevision { node_changes, .. } => node_changes
                .iter()
                .any(|c| c.node.id == "a" && c.node.created_at.is_some()),
            _ => false,
        })
        .collect();
    assert_eq!(captured.len(), 1, "one ApplyRevision for the backfill");
    let op = captured.remove(0);
    assert_eq!(op.actor, "system", "written as the system actor");
    applicator(&replica)
        .apply_operation(&op)
        .await
        .expect("apply the backfill");

    let a = replica.stored("a").await?;
    assert_eq!(a.created_at, Some(time_of(&r1)));
    assert_eq!(a.updated_at, Some(time_of(&r1)));
    assert_eq!(replica.compound_builds("main").await?.compound.built, 1);
    assert!(replica.builtin_ready("main"));
    assert_eq!(replica.children("main", "/").await?, ["a"]);
    Ok(())
}

/// `RAISIN_TIMESTAMP_BACKFILL=0` (here: the process-wide override) leaves the
/// nodes alone: nothing pending, nothing queued. A refused branch is not
/// re-linked by every request; the backfill's re-request goes past that.
#[tokio::test]
async fn opt_out_leaves_nodes_alone_and_refusals_are_not_relinked() -> Result<()> {
    let env = Env::new(false).await?;
    legacy(&env, "a", rev(&env), |_| {}).await;

    override_timestamp_backfill(Some(false));
    let pending = pending_timestamp_backfill_branches(&env.storage)?;
    let queued = start_chain(&env.storage, RepairKind::TimestampBackfill).await?;
    override_timestamp_backfill(Some(true));
    let pending_on = pending_timestamp_backfill_branches(&env.storage)?;
    override_timestamp_backfill(None);
    assert!(pending.is_empty(), "switched off: nothing pending");
    assert_eq!(queued, 0, "switched off: nothing queued");
    assert!(env.stored("a").await?.created_at.is_none(), "left alone");
    assert_eq!(pending_on.len(), 1, "switched on: the branch is owed");

    assert!(env.compound_builds("main").await?.completed);
    assert_eq!(
        enqueue_compound_builds_if_owed(&env.storage, TENANT, REPO, "main").await?,
        0,
        "refused on the same owed work: not re-linked per request"
    );
    assert_eq!(
        request_compound_builds_after_backfill(&env.storage, TENANT, REPO, "main").await?,
        1
    );
    Ok(())
}

/// Stopped after its first chunk (a crash), the run resumes from its cursor
/// and ends where an uninterrupted run would.
#[tokio::test]
async fn the_backfill_resumes_after_a_crash() -> Result<()> {
    let env = Env::new(false).await?;
    for (i, id) in ["a", "b", "c"].into_iter().enumerate() {
        legacy(&env, id, rev(&env), |n| {
            n.properties
                .insert("n".to_string(), PropertyValue::Integer(i as i64));
        })
        .await;
    }
    // One live node per chunk; "crash" after two chunks (the workspace may
    // hold a node of its own before `a`, so the first chunk can be empty).
    let one_node_chunks = RepairOptions {
        batch_bytes: 1,
        stop_after_batches: Some(2),
        ..repair_options()
    };
    let first = env.backfill_with("main", one_node_chunks).await?;
    assert!(!first.completed);
    assert!((1..=2).contains(&first.timestamps.backfilled));
    let second = env.backfill("main").await?;
    assert!(second.resumed && second.completed);
    assert_eq!(
        first.timestamps.backfilled + second.timestamps.backfilled,
        3,
        "the rest, never again what the first run did"
    );
    assert_eq!(second.timestamps.unchanged, 0);
    for id in ["a", "b", "c"] {
        assert!(env.stored(id).await?.created_at.is_some(), "{id}");
        assert_eq!(
            versions(&env, "main", id).len(),
            1,
            "{id}: rewritten in place"
        );
    }
    Ok(())
}
