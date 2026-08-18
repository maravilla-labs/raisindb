//! Capture: turning a local edit to a mounted node into a prompt writeback.
//!
//! # This layer is an OPTIMIZATION and nothing else
//!
//! Correctness belongs to two other things, both of which run whether or not a
//! single event ever reaches this file:
//!
//! * every ordinary `VirtualMountSync` drains the mount before it reads
//!   (`virtual_mount_sync::phases::run_phases`), and
//! * `VirtualMountWriteReconcile` walks `RevisionMeta` from a durable watermark,
//!   which is the only thing that can see a DELETE at all.
//!
//! So this hook is allowed to be noisy, allowed to miss events, and allowed to
//! be switched off. What it is NOT allowed to do is block the event bus or make
//! a provider call — it enqueues a job and returns. The bus is on the commit
//! path; a network call here would stall every write in the process.
//!
//! # Two kinds of node reach a mount, and only one carries stamps
//!
//! A node the materializer created carries `__virtual` + `__mount_id`, and
//! routes by id. A node BORN LOCALLY inside a mount's path carries neither — it
//! has never been to the provider — and an OUTBOX COMMAND is exactly that: a
//! node the application writes and queues to be sent.
//!
//! Requiring the stamps therefore skipped the latency path for every command,
//! which is the one case where latency is a person waiting. Locally-born nodes
//! fall back to routing by PATH, the same way deletes always have.
//!
//! # Why it sits where it does
//!
//! Beside trigger evaluation, in `handle_local_node_change` /
//! `handle_node_delete`. That is downstream of `emit_node_events`, so it sees
//! the node API, SQL DML (including the bulk paths, which do go through
//! `TransactionalContext` and do emit at commit), functions and flows uniformly
//! — one hook instead of one per writer.
//!
//! The one write path that is genuinely silent is
//! `repositories/nodes/crud/update.rs`: no event, no `RevisionMeta`, and it
//! cannot set `updated_by`. It is out of contract for mount-owned nodes, and
//! deliberately NOT hooked — hooking it would be endorsing it.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use raisin_core::services::node_service::NodeService;
use raisin_events::NodeEvent;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::jobs::{JobContext, JobType};
use tokio::sync::RwLock;

use super::UnifiedJobEventHandler;
use crate::jobs::handlers::virtual_mount_sync::{check, MountConfig, SYNC_ACTOR, SYSTEM_WORKSPACE};
use crate::RocksDBStorage;

/// How long a repository's mount routes are trusted before being re-read.
///
/// A mount created, re-pointed or switched to `mode: off` takes effect for
/// capture within this window. Making it shorter buys nothing — the scheduled
/// sync and the reconcile walk both re-read the mount on every tick — and
/// making it much longer would leave a freshly configured mount without its
/// latency path for as long as an operator is likely to be watching.
const ROUTE_CACHE_TTL: Duration = Duration::from_secs(30);

/// Where one writable mount materializes, and where its config lives.
///
/// Only mounts that are enabled AND ask for outbound writes are ever built into
/// a route, so a lookup that finds one has already answered "should this edit be
/// captured?".
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MountRoute {
    pub mount_id: String,
    /// The branch the mount NODE lives on — where a drain job must be addressed.
    /// Not the same as `target_branch`, and confusing the two produces a job
    /// that cannot find its own mount.
    pub config_branch: String,
    pub target_branch: String,
    pub target_workspace: String,
    pub mount_path: String,
}

impl MountRoute {
    /// Whether this route materializes into the branch+workspace an event names.
    fn covers(&self, branch: &str, workspace: &str) -> bool {
        self.target_branch == branch && self.target_workspace == workspace
    }

    /// Whether `path` is the mount root or lives under it.
    ///
    /// Prefix matching on the SEGMENT boundary, never on the raw string:
    /// `/mailbox` is not under `/mail`, and a bare `starts_with` would have a
    /// delete in one collection wake up a completely unrelated mount.
    fn contains(&self, path: &str) -> bool {
        let root = self.mount_path.trim_end_matches('/');
        if root.is_empty() {
            // A mount at the workspace root owns everything in it.
            return true;
        }
        path == root || path.starts_with(&format!("{root}/"))
    }
}

/// The mount this node belongs to, or `None` — at ZERO storage reads.
///
/// `metadata.node_data` carries the full post-write node for creates and
/// updates, and the two reserved props the materializer stamps on everything it
/// writes are the membership test. Every node in the system that is not
/// mount-owned leaves here, which is what keeps this hook off the hot path.
pub(crate) fn mount_id_of(node: &Node) -> Option<&str> {
    if !matches!(
        node.properties.get("__virtual"),
        Some(PropertyValue::Boolean(true))
    ) {
        return None;
    }
    match node.properties.get("__mount_id") {
        Some(PropertyValue::String(s)) if !s.is_empty() => Some(s),
        _ => None,
    }
}

/// Whether this event is the sync engine hearing its own write.
///
/// `metadata.actor` is stamped on every commit-path event, deletes included,
/// from the transaction's auth context — and the materializer runs as
/// `AuthContext::system_as(SYNC_ACTOR)`. A user edit can never carry this actor,
/// so the filter drops echoes without ever dropping real work.
///
/// It is a NOISE filter, not the loop guard. The loop is broken by
/// `__pushed_state` and the etag in the drain's converge check; a capture layer
/// that relied on actor filtering for correctness would be relying on a
/// best-effort signal for a durability property.
pub(crate) fn is_sync_echo(node_event: &NodeEvent) -> bool {
    node_event
        .metadata
        .as_ref()
        .and_then(|m| m.get("actor"))
        .and_then(|v| v.as_str())
        .map(|a| a == SYNC_ACTOR)
        .unwrap_or(false)
}

/// The route for a node that names its own mount.
pub(crate) fn route_for_edit<'a>(
    routes: &'a [MountRoute],
    mount_id: &str,
    branch: &str,
    workspace: &str,
) -> Option<&'a MountRoute> {
    routes
        .iter()
        .find(|r| r.mount_id == mount_id && r.covers(branch, workspace))
}

/// The route for a node that no longer exists.
///
/// A delete carries no `node_data` — the node is gone, so nothing can be read
/// off it — which is why this matches on the stored path instead. Resolving the
/// external id is left to the reconcile walk, which reads the MVCC pre-image;
/// doing it here would be a storage read on the event bus for a value the walk
/// has to fetch anyway.
pub(crate) fn route_for_delete<'a>(
    routes: &'a [MountRoute],
    branch: &str,
    workspace: &str,
    path: &str,
) -> Option<&'a MountRoute> {
    routes
        .iter()
        .find(|r| r.covers(branch, workspace) && r.contains(path))
}

struct CacheEntry {
    loaded_at: Instant,
    routes: Arc<Vec<MountRoute>>,
}

/// Per-repository mount routes, cached so a delete event costs a map lookup.
///
/// The delete arm has no cheap rejection available to it (see
/// [`route_for_delete`]), so every local delete in the process consults this.
/// A repository with no writable mounts caches an empty vector and answers in
/// nanoseconds — which is the case that has to stay free, because it is almost
/// every repository.
#[derive(Default)]
pub(crate) struct MountRouteCache {
    entries: RwLock<HashMap<(String, String), CacheEntry>>,
}

impl MountRouteCache {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// This repository's writable mounts, re-reading them when the TTL lapses.
    pub(crate) async fn routes(
        &self,
        storage: &Arc<RocksDBStorage>,
        tenant: &str,
        repo: &str,
    ) -> Arc<Vec<MountRoute>> {
        let key = (tenant.to_string(), repo.to_string());
        {
            let entries = self.entries.read().await;
            if let Some(e) = entries.get(&key) {
                if e.loaded_at.elapsed() < ROUTE_CACHE_TTL {
                    return e.routes.clone();
                }
            }
        }
        let routes = Arc::new(load_routes(storage, tenant, repo).await);
        let mut entries = self.entries.write().await;
        entries.insert(
            key,
            CacheEntry {
                loaded_at: Instant::now(),
                routes: routes.clone(),
            },
        );
        routes
    }

    /// Install routes directly, bypassing the load. Tests only: building a real
    /// mount node requires a repository, a branch and the global nodetypes.
    #[cfg(test)]
    pub(crate) async fn seed(&self, tenant: &str, repo: &str, routes: Vec<MountRoute>) {
        self.entries.write().await.insert(
            (tenant.to_string(), repo.to_string()),
            CacheEntry {
                loaded_at: Instant::now(),
                routes: Arc::new(routes),
            },
        );
    }
}

/// Read every enabled, write-wanting mount in one repository.
///
/// A failure caches an empty result for the TTL rather than retrying per event:
/// this layer is best-effort, and a repository whose system workspace cannot be
/// listed would otherwise pay that failed read on every delete.
async fn load_routes(storage: &Arc<RocksDBStorage>, tenant: &str, repo: &str) -> Vec<MountRoute> {
    let branch = check::config_branch(storage, tenant, repo).await;
    let svc: NodeService<RocksDBStorage> = NodeService::new_with_context(
        storage.clone(),
        tenant.to_string(),
        repo.to_string(),
        branch.clone(),
        SYSTEM_WORKSPACE.to_string(),
    )
    .with_auth(AuthContext::system());

    let nodes = match svc.list_by_type("raisin:VirtualMount").await {
        Ok(n) => n,
        Err(e) => {
            tracing::debug!(
                tenant = %tenant, repo = %repo, error = %e,
                "vmount capture: could not list mounts; skipping the latency path for now"
            );
            return Vec::new();
        }
    };

    nodes
        .iter()
        .filter_map(|node| MountConfig::from_node(node).ok())
        // An unparseable mount is reported by the sync check, not here — this
        // layer has no state to record a verdict on and no business writing one.
        .filter(|m| m.enabled && m.write_config.wants_any_write())
        .map(|m| MountRoute {
            mount_id: m.mount_id,
            config_branch: branch.clone(),
            target_branch: m.target_branch,
            target_workspace: m.target_workspace,
            mount_path: m.mount_path,
        })
        .collect()
}

impl UnifiedJobEventHandler {
    /// A local create/update: if it touched a mounted node, drain that mount now.
    pub(crate) async fn capture_virtual_write(&self, node_event: &NodeEvent, node: Option<&Node>) {
        // No `node_data` means the post-commit read of the node failed. There is
        // nothing to quick-reject on and nothing worth a storage read for: the
        // scheduled sync's drain phase covers it.
        let Some(node) = node else { return };
        if is_sync_echo(node_event) {
            return;
        }
        let routes = self
            .vmount_routes
            .routes(
                &self.storage,
                &node_event.tenant_id,
                &node_event.repository_id,
            )
            .await;

        // Two ways to find the mount, because there are two ways a node gets
        // into one.
        //
        // A node the materializer created carries `__virtual` + `__mount_id`, so
        // it routes by id — the cheap, exact path, and the one every mirrored
        // edit takes.
        //
        // A node BORN LOCALLY inside a mount's path carries neither stamp: it
        // has never been to the provider. That is exactly what an OUTBOX COMMAND
        // is — a `stripe:CheckoutSession` written by the shop and queued to be
        // sent — and requiring the stamps meant capture returned early on every
        // one of them. The instant path was skipped and the command waited for
        // the poll: five minutes, at the default interval, of a buyer staring at
        // "getting your payment link" while the session sat `queued`.
        //
        // The delete arm has always routed by PATH, for the same reason in
        // mirror image (a deleted node has no data to read stamps off). So the
        // fallback here is not a new mechanism, it is the existing one applied
        // to the other case that has no stamps.
        let route = match mount_id_of(node) {
            Some(mount_id) => route_for_edit(
                &routes,
                mount_id,
                &node_event.branch,
                &node_event.workspace_id,
            ),
            None => match node_event.path.as_deref() {
                Some(path) => {
                    route_for_delete(&routes, &node_event.branch, &node_event.workspace_id, path)
                }
                None => None,
            },
        };
        let Some(route) = route else {
            return;
        };
        // Note the node FIRST, before the drain job is enqueued: if a sync run
        // holds the mount's lease, the drain no-ops ("locked_elsewhere") and
        // this note is what lets the running sync push the edit at its next
        // page boundary instead of after the whole import. If no sync is
        // running, the drain's fresh index nominates the node anyway and the
        // note is consumed later as a one-point-read no-op.
        let key = crate::jobs::handlers::virtual_mount_sync::write::pending::pending_key(
            &node_event.tenant_id,
            &node_event.repository_id,
            &route.config_branch,
            &route.mount_id,
        );
        crate::jobs::handlers::virtual_mount_sync::write::pending::note(&key, &node_event.node_id);

        self.enqueue_vmount(
            node_event,
            route,
            JobType::VirtualMountWriteDrain {
                mount_id: route.mount_id.clone(),
                trigger: "capture".to_string(),
            },
        )
        .await;
    }

    /// A local delete under a mount path: run the durable walk now.
    ///
    /// Deliberately NOT a drain. The drain works from the mount's live index, so
    /// a node that has been deleted is invisible to it; the reconcile walk is the
    /// only thing that can see the delete, and it recovers the external id from
    /// the MVCC pre-image. Enqueuing it here shortens the window between the
    /// delete and its durable record — the one window in the whole design that
    /// is bounded by MVCC garbage collection rather than by a retry.
    pub(crate) async fn capture_virtual_delete(&self, node_event: &NodeEvent) {
        if is_sync_echo(node_event) {
            return;
        }
        let Some(path) = node_event.path.as_deref() else {
            return;
        };
        let routes = self
            .vmount_routes
            .routes(
                &self.storage,
                &node_event.tenant_id,
                &node_event.repository_id,
            )
            .await;
        let Some(route) =
            route_for_delete(&routes, &node_event.branch, &node_event.workspace_id, path)
        else {
            return;
        };
        self.enqueue_vmount(
            node_event,
            route,
            JobType::VirtualMountWriteReconcile {
                tenant_id: Some(node_event.tenant_id.clone()),
                repo_id: Some(node_event.repository_id.clone()),
            },
        )
        .await;
    }

    /// Enqueue against the mount's CONFIG scope, not the event's.
    ///
    /// The event names the branch and workspace the virtual nodes materialize
    /// into; every job here reads the mount NODE, which lives on the config
    /// branch in `raisin:system`. Passing the event's scope through would give a
    /// job that reports `mount_not_found` and no clue why.
    async fn enqueue_vmount(&self, node_event: &NodeEvent, route: &MountRoute, job: JobType) {
        let context = JobContext {
            tenant_id: node_event.tenant_id.clone(),
            repo_id: node_event.repository_id.clone(),
            branch: route.config_branch.clone(),
            workspace_id: SYSTEM_WORKSPACE.to_string(),
            revision: node_event.revision,
            metadata: HashMap::new(),
        };
        if let Err(e) = self.enqueue_job(job, &context).await {
            tracing::warn!(
                mount_id = %route.mount_id,
                error = %e,
                "vmount capture: could not enqueue the writeback job; the scheduled \
                 sync and the reconcile walk still cover this edit"
            );
        }
    }
}

#[cfg(test)]
#[path = "vmount_capture_tests.rs"]
mod vmount_capture_tests;
