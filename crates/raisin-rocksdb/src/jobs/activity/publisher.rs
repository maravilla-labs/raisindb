//! Writing the activity node, coalesced, and only when something changed.

use super::{
    JobActivityTracker, ScopeKey, Snapshot, ACTIVITY_NODE_TYPE, ACTIVITY_WORKSPACE,
    MIN_PUBLISH_INTERVAL,
};
use crate::RocksDBStorage;
use raisin_error::Result;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::jobs::JobRegistry;
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Where the activity node lives, at the workspace root.
///
/// Root-level on purpose: a newly added global workspace gets its RECORD
/// created on pre-existing repositories by the definition resync, but not its
/// `initial_structure` — so a design needing a container folder would work on
/// new repos and silently fail on every repo that already exists.
pub const ACTIVITY_NODE_PATH: &str = "/activity";

/// The node id, fixed rather than random.
///
/// Two cluster members can reach the create path at the same moment. With one
/// id they collide on the same node and one write wins, which is the intended
/// last-writer-wins; with random ids they would race to put two different nodes
/// at one path.
const ACTIVITY_NODE_ID: &str = "job-activity";

/// The actor the write is attributed to. Distinct from `"system"` so
/// `updated_by` on this node names the mechanism rather than the platform at
/// large.
pub const ACTIVITY_ACTOR: &str = "job-activity";

/// The branch the activity node lives on.
///
/// Fixed to the workspace's declared `default_branch`. Activity is a property
/// of the PROCESS, not of a line of content: a per-branch node would multiply
/// cardinality by the branch count for a surface that has one answer, and a
/// branch's activity is not separable anyway — the pools are shared.
pub const ACTIVITY_BRANCH: &str = "main";

/// How often a DEGRADED scope re-checks itself.
///
/// The only timer in this module, and it exists because a breaker CLOSING is
/// not otherwise a transition anything reports. A parked job is redelivered and
/// clears the flag on its next success — but if the upstream recovers while
/// this repo happens to have nothing to retry, `degraded: true` would sit on
/// the node forever, which is a silent stall and worse than the outage it
/// describes. It arms only while `degraded` is true and disarms the moment the
/// flag clears, so a healthy idle tenant still costs ZERO writes.
const DEGRADED_RECHECK_INTERVAL: Duration = Duration::from_secs(15);

/// The channel a job start/finish notifies. Cheap and non-blocking: the caller
/// is on the job hot path and must never wait on a node write.
pub(super) struct PublishSink {
    tx: mpsc::UnboundedSender<ScopeKey>,
}

impl PublishSink {
    pub(super) fn notify(&self, key: ScopeKey) {
        // A closed channel means the publisher task is gone (shutdown). Losing
        // an activity update then is correct: there is nothing left to report
        // it to.
        let _ = self.tx.send(key);
    }
}

/// Start publishing activity for this process.
///
/// Called once, when the worker pool starts — that is precisely the moment this
/// process begins to have job activity. Calling it twice is a no-op: the sink
/// is a `OnceLock`, so a second publisher can never race the first over the
/// same node.
pub fn install_publisher(storage: Arc<RocksDBStorage>, job_registry: Arc<JobRegistry>) {
    let tracker = Arc::clone(JobActivityTracker::global());
    let (tx, rx) = mpsc::unbounded_channel();
    if tracker.sink.set(PublishSink { tx }).is_err() {
        tracing::debug!("job activity publisher already installed");
        return;
    }
    tokio::spawn(run(storage, job_registry, tracker, rx));
}

/// The coalescing loop.
///
/// One task for the whole process, holding a deadline per scope. Note there is
/// no periodic tick: with nothing pending it blocks on `recv()` indefinitely,
/// which is what makes an idle server write nothing at all.
async fn run(
    storage: Arc<RocksDBStorage>,
    job_registry: Arc<JobRegistry>,
    tracker: Arc<JobActivityTracker>,
    mut rx: mpsc::UnboundedReceiver<ScopeKey>,
) {
    // When each scope may next be written, and which scopes are waiting.
    let mut next_allowed: HashMap<ScopeKey, Instant> = HashMap::new();
    let mut pending: HashMap<ScopeKey, Instant> = HashMap::new();

    loop {
        let sleep_for = pending
            .values()
            .min()
            .map(|due| due.saturating_duration_since(Instant::now()));

        let key = match sleep_for {
            None => match rx.recv().await {
                Some(key) => Some(key),
                None => break,
            },
            Some(wait) => {
                tokio::select! {
                    received = rx.recv() => match received {
                        Some(key) => Some(key),
                        None => break,
                    },
                    _ = tokio::time::sleep(wait) => None,
                }
            }
        };

        if let Some(key) = key {
            // A transition arrived. Publish now if the floor has elapsed,
            // otherwise hold it until the window closes — held, never dropped,
            // or the last change in a burst would be the one that is lost.
            let due = next_allowed
                .get(&key)
                .copied()
                .unwrap_or_else(Instant::now)
                .max(Instant::now());
            pending.entry(key).or_insert(due);
        }

        let now = Instant::now();
        let ready: Vec<ScopeKey> = pending
            .iter()
            .filter(|(_, due)| **due <= now)
            .map(|(key, _)| key.clone())
            .collect();

        for key in ready {
            pending.remove(&key);
            next_allowed.insert(key.clone(), Instant::now() + MIN_PUBLISH_INTERVAL);
            match publish(&storage, &job_registry, &tracker, &key).await {
                Ok(degraded) => {
                    if degraded {
                        pending
                            .entry(key.clone())
                            .or_insert(Instant::now() + DEGRADED_RECHECK_INTERVAL);
                    }
                }
                Err(e) => {
                    // At `warn`, not silence: this is the surface an operator
                    // and a tenant both read, and a write that vanishes leaves
                    // a stale picture that looks authoritative.
                    tracing::warn!(
                        tenant = %key.0,
                        repo = %key.1,
                        error = %e,
                        "failed to write job activity node"
                    );
                }
            }
            // Drop the floor entry once a scope has gone quiet, so this map
            // holds live repositories rather than every repository this process
            // has ever served.
            if tracker.published(&key).is_none() {
                next_allowed.remove(&key);
            }
        }
    }
}

/// Write one repository's activity node, if it changed. Returns whether the
/// snapshot just published was degraded.
async fn publish(
    storage: &Arc<RocksDBStorage>,
    job_registry: &Arc<JobRegistry>,
    tracker: &Arc<JobActivityTracker>,
    key: &ScopeKey,
) -> Result<bool> {
    // A scope the tracker has forgotten has nothing in flight; the snapshot it
    // deserves is the idle one, which is how a repo's last job finishing gets
    // written down.
    let snapshot = tracker
        .snapshot(key)
        .unwrap_or_else(|| Snapshot::build(std::iter::empty(), std::iter::empty()));

    if !snapshot.differs_from(tracker.published(key).as_ref()) {
        return Ok(snapshot.degraded);
    }

    let (tenant, repo) = key;
    // Tenant-wide and maintained as an O(1) counter by the registry itself, so
    // it is derived rather than counted here — a counter of our own would drift
    // on every job that is cancelled or lost at shutdown and could only ever
    // drift upward, pinning the surface at a backlog that never existed.
    let tenant_pending = job_registry.active_job_count(tenant).await;

    let tx = storage.begin_context().await?;
    tx.set_tenant_repo(tenant, repo)?;
    tx.set_branch(ACTIVITY_BRANCH)?;
    tx.set_actor(ACTIVITY_ACTOR)?;
    // System privileges under the activity identity: the AUTH CONTEXT, not the
    // raw actor, is what stamps `updated_by`.
    tx.set_auth_context(AuthContext::system_as(ACTIVITY_ACTOR))?;
    tx.set_message("job activity")?;
    // Engine bookkeeping. This is the second half of the loop guard: the node
    // type's `index_types: [Property]` closes the fulltext and embedding arms,
    // and this marker closes trigger evaluation and the per-revision
    // TreeSnapshot job. The event is still emitted — that is the whole point,
    // it is what Studio subscribes to.
    tx.set_bookkeeping(true)?;

    let active_paths = resolve_paths(tx.as_ref(), &snapshot).await;

    let mut properties: HashMap<String, PropertyValue> = HashMap::new();
    properties.insert("degraded".into(), PropertyValue::Boolean(snapshot.degraded));
    properties.insert(
        "active".into(),
        PropertyValue::Integer(snapshot.active as i64),
    );
    properties.insert(
        "tenant_pending".into(),
        PropertyValue::Integer(tenant_pending as i64),
    );
    properties.insert(
        "active_paths".into(),
        PropertyValue::Array(
            active_paths
                .into_iter()
                .map(PropertyValue::String)
                .collect(),
        ),
    );
    properties.insert(
        "updated_at".into(),
        PropertyValue::String(chrono::Utc::now().to_rfc3339()),
    );
    properties.insert("origin".into(), PropertyValue::String(origin().to_string()));

    let node = Node {
        id: ACTIVITY_NODE_ID.to_string(),
        name: "activity".to_string(),
        path: ACTIVITY_NODE_PATH.to_string(),
        node_type: ACTIVITY_NODE_TYPE.to_string(),
        properties,
        ..Default::default()
    };

    // Upsert by PATH, so a repository that has never had one gets it created
    // and every later write updates the same node. One node, forever.
    tx.upsert_node(ACTIVITY_WORKSPACE, &node).await?;
    tx.commit().await?;

    let degraded = snapshot.degraded;
    tracker.set_published(key, Some(snapshot));
    Ok(degraded)
}

/// Turn the snapshot's `(workspace, node_id)` subjects into `workspace:/path`
/// strings.
///
/// The read happens HERE and not in the snapshot, because a snapshot is built
/// on every job start and finish while a write happens at most once per
/// [`MIN_PUBLISH_INTERVAL`] per repo — so this is at most a handful of reads
/// every two seconds, and never on the job hot path.
///
/// A node that cannot be read is simply omitted: it was probably deleted (a
/// `NodeDeleteCleanup` job's subject always has been), and inventing a path for
/// it would be worse than naming one fewer.
async fn resolve_paths(tx: &dyn TransactionalContext, snapshot: &Snapshot) -> Vec<String> {
    let mut out = Vec::with_capacity(snapshot.subjects.len());
    for (workspace, node_id) in &snapshot.subjects {
        if let Ok(Some(node)) = tx.get_node(workspace, node_id).await {
            // `workspace:/path` — the spelling `REFERENCES('workspace:/path')`
            // uses, so a path here is directly comparable to one a query names.
            out.push(format!("{}:{}", workspace, node.path));
        }
    }
    out
}

/// Which process wrote a record, so last-writer-wins is attributable.
fn origin() -> &'static str {
    static ORIGIN: OnceLock<String> = OnceLock::new();
    ORIGIN.get_or_init(|| {
        std::env::var("RAISIN_NODE_ID")
            .or_else(|_| std::env::var("HOSTNAME"))
            .unwrap_or_else(|_| format!("pid-{}", std::process::id()))
    })
}
