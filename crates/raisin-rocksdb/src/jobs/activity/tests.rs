//! Tests for the activity tracker and its write-on-transition rule.
//!
//! The loop guard itself — that writing this node enqueues NO indexing,
//! embedding, asset or trigger job — is tested where the enqueueing lives, in
//! `jobs::event_handler::tests::activity_node_write_enqueues_no_indexing_jobs`.

use super::*;
use raisin_hlc::HLC;

fn context(tenant: &str, repo: &str, workspace: &str) -> JobContext {
    JobContext {
        tenant_id: tenant.to_string(),
        repo_id: repo.to_string(),
        branch: "main".to_string(),
        workspace_id: workspace.to_string(),
        revision: HLC::new(1, 0),
        metadata: HashMap::new(),
    }
}

fn embedding_job(node_id: &str) -> JobType {
    JobType::EmbeddingGenerate {
        node_id: node_id.to_string(),
    }
}

/// An idle scope produces a snapshot that is not worth writing.
///
/// This is the constraint the whole module exists to satisfy: a tenant doing
/// nothing must mint no node revisions. If this ever fails, every tenant on the
/// box grows MVCC history forever.
#[test]
fn an_idle_scope_is_never_worth_writing() {
    let idle = Snapshot::build_with_degraded(std::iter::empty(), false);
    assert!(idle.is_idle());
    assert!(
        !idle.differs_from(None),
        "an idle scope that has never published must not write"
    );
    assert!(
        !idle.differs_from(Some(&idle)),
        "an idle scope that already published idle must not write again"
    );
}

/// Work starting is a transition; the same work continuing is not.
#[test]
fn only_a_change_is_worth_writing() {
    let idle = Snapshot::build_with_degraded(std::iter::empty(), false);
    let busy = Snapshot::build_with_degraded([("default", Some("node1"))].into_iter(), false);

    assert!(busy.differs_from(Some(&idle)), "work starting is a change");
    assert!(
        !busy.differs_from(Some(&busy)),
        "the same work still running is not a change, however long it runs"
    );
    assert!(idle.differs_from(Some(&busy)), "work finishing is a change");
}

/// Degradation is a transition on its own, with no job movement behind it.
///
/// Without this, an upstream that recovered while a repo happened to be quiet
/// would leave `degraded: true` on the node forever — a silent stall, which is
/// worse than the outage it describes.
#[test]
fn a_degraded_flag_flip_is_a_change() {
    let healthy = Snapshot::build_with_degraded(std::iter::empty(), false);
    let degraded = Snapshot::build_with_degraded(std::iter::empty(), true);

    assert!(!degraded.is_idle(), "degraded is never idle");
    assert!(degraded.differs_from(Some(&healthy)));
    assert!(healthy.differs_from(Some(&degraded)));
}

/// Two snapshots of the same work must compare equal regardless of the order
/// the in-flight map happened to iterate in.
///
/// Without the sort in `Snapshot::build`, every job start would look like a
/// change to every other scope member, the coalescing would never settle, and a
/// busy repo would write a node revision every two seconds forever.
#[test]
fn subject_order_does_not_make_a_change() {
    let a = Snapshot::build_with_degraded(
        [("default", Some("n1")), ("assets", Some("n2"))].into_iter(),
        false,
    );
    let b = Snapshot::build_with_degraded(
        [("assets", Some("n2")), ("default", Some("n1"))].into_iter(),
        false,
    );
    assert_eq!(a, b);
    assert!(!a.differs_from(Some(&b)));
}

/// The subject list is CAPPED. It exists so the SPA can say "working on
/// these", not so anyone can reconstruct the queue — an uncapped list would
/// grow with the incident it describes.
#[test]
fn the_subject_list_is_capped_but_the_count_is_not() {
    let ids: Vec<String> = (0..50).map(|i| format!("node{i:02}")).collect();
    let snapshot =
        Snapshot::build_with_degraded(ids.iter().map(|id| ("default", Some(id.as_str()))), false);
    assert_eq!(snapshot.subjects.len(), ACTIVE_PATHS_CAP);
    assert_eq!(
        snapshot.active, 50,
        "the count reports all the work even though the list names a few of it"
    );
}

/// A maintenance job is counted but names no path: it is about the repository,
/// not about a node, and inventing a subject for it would be a lie.
#[test]
fn a_subjectless_job_is_counted_without_a_path() {
    let snapshot = Snapshot::build_with_degraded(
        [("default", None), ("default", Some("n1"))].into_iter(),
        false,
    );
    assert_eq!(snapshot.active, 2);
    assert_eq!(
        snapshot.subjects,
        vec![("default".to_string(), "n1".to_string())]
    );
}

/// The guard is what makes the count trustworthy: dropping it must decrement,
/// including on the paths that never reach a success.
#[test]
fn the_guard_balances_the_count() {
    let tracker = Arc::new(JobActivityTracker::new());
    let key = ("t1".to_string(), "r1".to_string());
    let ctx = context("t1", "r1", "default");

    let a = tracker.track(&JobId::new(), &embedding_job("n1"), &ctx);
    let b = tracker.track(&JobId::new(), &embedding_job("n2"), &ctx);
    assert_eq!(tracker.snapshot(&key).unwrap().active, 2);

    drop(a);
    assert_eq!(tracker.snapshot(&key).unwrap().active, 1);
    drop(b);
    assert_eq!(tracker.snapshot(&key).unwrap().active, 0);
}

/// Two repositories are two records, and neither sees the other's work.
///
/// Tenant isolation on this surface comes from the key prefix on the node, but
/// the numbers have to be separated here too — a count that mixed tenants would
/// leak how busy someone else is.
#[test]
fn scopes_do_not_bleed_into_each_other() {
    let tracker = Arc::new(JobActivityTracker::new());
    let one = ("t1".to_string(), "r1".to_string());
    let two = ("t2".to_string(), "r1".to_string());

    let _held = tracker.track(
        &JobId::new(),
        &embedding_job("n1"),
        &context("t1", "r1", "default"),
    );

    assert_eq!(tracker.snapshot(&one).unwrap().active, 1);
    assert!(
        tracker.snapshot(&two).is_none(),
        "a tenant with no work must not appear at all"
    );
}

/// A scope that goes quiet and has published its idle state is FORGOTTEN, so
/// the map holds live repositories rather than every repository this process
/// has ever served.
#[test]
fn a_settled_idle_scope_is_dropped() {
    let tracker = Arc::new(JobActivityTracker::new());
    let key = ("t1".to_string(), "r1".to_string());
    let ctx = context("t1", "r1", "default");

    let held = tracker.track(&JobId::new(), &embedding_job("n1"), &ctx);
    tracker.set_published(&key, tracker.snapshot(&key));
    drop(held);

    // The publisher records the idle snapshot it just wrote; that is what
    // retires the entry.
    tracker.set_published(
        &key,
        Some(Snapshot::build_with_degraded(std::iter::empty(), false)),
    );
    assert!(tracker.snapshot(&key).is_none());
}

/// The degraded bit follows AFFECTED WORK, not the host's breaker board.
///
/// A tenant with nothing parked is never told about an outage on a shared
/// upstream: the banner claims "your processing is paused and will catch up",
/// and for that tenant it is false. This is also what removes the fan-out
/// problem rather than managing it — no park, no write, no banner.
#[test]
fn a_tenant_with_no_parked_work_is_not_degraded() {
    let tracker = Arc::new(JobActivityTracker::new());
    let key = ("t1".to_string(), "r1".to_string());
    let ctx = context("t1", "r1", "default");

    let _held = tracker.track(&JobId::new(), &embedding_job("n1"), &ctx);
    assert!(
        !tracker.snapshot(&key).unwrap().degraded,
        "work in flight against a healthy upstream is not degradation"
    );
}

/// A key that has no breaker at all reads as NOT degraded.
///
/// An upstream that has never been called is not an upstream that is down, and
/// a fresh tenant must not see a banner because no embedding has ever run.
#[test]
fn an_unknown_upstream_is_not_degraded() {
    let tracker = Arc::new(JobActivityTracker::new());
    let key = ("t1".to_string(), "r1".to_string());
    let ctx = context("t1", "r1", "default");

    // A key the process-wide registry has never seen. `status()` answers None,
    // which must read as healthy rather than as unknown-therefore-bad.
    tracker.record_parked(&ctx, "embedding:openai:https://never-dialled.invalid");
    assert!(!tracker.snapshot(&key).unwrap().degraded);
}

/// A success clears the park, so the banner goes away on the first call that
/// works rather than on the publisher's next re-check.
#[test]
fn a_success_clears_the_park() {
    let tracker = Arc::new(JobActivityTracker::new());
    let key = ("t1".to_string(), "r1".to_string());
    let ctx = context("t1", "r1", "default");

    tracker.record_parked(&ctx, "embedding:openai:https://example.invalid");
    tracker.record_upstream_ok(&ctx, "embedding:openai:https://example.invalid");
    assert!(!tracker.snapshot(&key).unwrap().degraded);
}

/// One tenant's park does not degrade another tenant on the same upstream.
///
/// The breaker they share is one object; the bit they see is not.
#[test]
fn a_park_degrades_only_the_tenant_that_parked() {
    let tracker = Arc::new(JobActivityTracker::new());
    let victim = context("t1", "r1", "default");
    let bystander = ("t2".to_string(), "r1".to_string());

    tracker.record_parked(&victim, "embedding:openai:https://example.invalid");
    assert!(
        tracker.snapshot(&bystander).is_none(),
        "a tenant with no affected work must not even appear, let alone be degraded"
    );
}
