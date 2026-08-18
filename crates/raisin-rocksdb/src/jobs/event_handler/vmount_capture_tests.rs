//! Capture-hook tests: which events wake a mount, and which must not.
//!
//! A child module of [`super`] (declared with `#[path]` at the bottom of
//! `vmount_capture.rs`) so the routing predicates can be exercised directly,
//! without a repository, a branch and the global nodetypes behind every case.

use super::*;
use crate::jobs::dispatcher::JobDispatcher;
use raisin_events::NodeEventKind;
use raisin_storage::jobs::{JobRegistry, JobStatus};
use raisin_storage::Storage;

const TENANT: &str = "default";
const REPO: &str = "cap-test";

fn route() -> MountRoute {
    MountRoute {
        mount_id: "m1".to_string(),
        config_branch: "main".to_string(),
        target_branch: "main".to_string(),
        target_workspace: "default".to_string(),
        mount_path: "/mail".to_string(),
    }
}

fn mounted_node() -> Node {
    let mut node = Node {
        id: "n1".to_string(),
        node_type: "raisin:Node".to_string(),
        name: "m00".to_string(),
        path: "/mail/m00.eml".to_string(),
        ..Default::default()
    };
    node.properties
        .insert("__virtual".to_string(), PropertyValue::Boolean(true));
    node.properties.insert(
        "__mount_id".to_string(),
        PropertyValue::String("m1".to_string()),
    );
    node.properties.insert(
        "__external_id".to_string(),
        PropertyValue::String("M00".to_string()),
    );
    node
}

fn event(kind: NodeEventKind, path: &str, actor: Option<&str>) -> NodeEvent {
    NodeEvent {
        tenant_id: TENANT.to_string(),
        repository_id: REPO.to_string(),
        workspace_id: "default".to_string(),
        branch: "main".to_string(),
        revision: raisin_hlc::HLC::new(1, 0),
        node_id: "n1".to_string(),
        node_type: Some("raisin:Node".to_string()),
        kind,
        path: Some(path.to_string()),
        metadata: actor.map(|a| {
            let mut m = HashMap::new();
            m.insert(
                "actor".to_string(),
                serde_json::Value::String(a.to_string()),
            );
            m
        }),
    }
}

// ---------------------------------------------------------------------------
// membership: the zero-read quick reject
// ---------------------------------------------------------------------------

#[test]
fn only_a_mount_owned_node_names_a_mount() {
    assert_eq!(mount_id_of(&mounted_node()), Some("m1"));

    // An ordinary node: the case that must cost nothing.
    assert_eq!(mount_id_of(&Node::default()), None);

    // `__mount_id` without `__virtual` is not a mount member — a user can set a
    // property, and a property alone must not aim a provider write.
    let mut faker = Node::default();
    faker.properties.insert(
        "__mount_id".to_string(),
        PropertyValue::String("m1".to_string()),
    );
    assert_eq!(mount_id_of(&faker), None);

    // An empty mount id addresses nothing.
    let mut empty = mounted_node();
    empty.properties.insert(
        "__mount_id".to_string(),
        PropertyValue::String(String::new()),
    );
    assert_eq!(mount_id_of(&empty), None);
}

// ---------------------------------------------------------------------------
// echo suppression
// ---------------------------------------------------------------------------

/// The sync engine's own writes must not wake the drain that made them.
///
/// Without this every materialized item — a 500-message backfill chunk included
/// — enqueues a drain, which loads the index and probes capabilities to discover
/// there is nothing to push.
#[test]
fn the_sync_engines_own_writes_are_echoes() {
    assert!(is_sync_echo(&event(
        NodeEventKind::Updated,
        "/mail/m00.eml",
        Some(SYNC_ACTOR)
    )));
    assert!(!is_sync_echo(&event(
        NodeEventKind::Updated,
        "/mail/m00.eml",
        Some("alice")
    )));
    // No metadata at all: not an echo. Being wrong in this direction costs a
    // redundant drain; being wrong the other way drops a user's edit.
    assert!(!is_sync_echo(&event(
        NodeEventKind::Updated,
        "/mail/m00.eml",
        None
    )));
}

// ---------------------------------------------------------------------------
// routing
// ---------------------------------------------------------------------------

#[test]
fn an_edit_routes_by_mount_id_within_the_mounts_own_scope() {
    let routes = vec![route()];
    assert!(route_for_edit(&routes, "m1", "main", "default").is_some());
    assert!(route_for_edit(&routes, "other", "main", "default").is_none());

    // A node carrying this mount's id on a DIFFERENT branch is a fork's copy.
    // Pushing it would export a branch's private state to the provider — the
    // same rule the reconcile walk applies to `RevisionMeta.branch`.
    assert!(route_for_edit(&routes, "m1", "feature", "default").is_none());
    assert!(route_for_edit(&routes, "m1", "main", "other-ws").is_none());
}

/// Path matching is on segment boundaries.
///
/// `/mailbox` under a `/mail` mount is the failure a bare `starts_with` ships:
/// a delete in an unrelated collection would wake a mailbox mount and, once
/// deletes propagate, be evaluated as one of its own.
#[test]
fn a_delete_routes_by_path_on_segment_boundaries() {
    let routes = vec![route()];
    assert!(route_for_delete(&routes, "main", "default", "/mail/m00.eml").is_some());
    // The mount root itself.
    assert!(route_for_delete(&routes, "main", "default", "/mail").is_some());
    assert!(route_for_delete(&routes, "main", "default", "/mailbox/m00.eml").is_none());
    assert!(route_for_delete(&routes, "main", "default", "/other/m00.eml").is_none());
    assert!(route_for_delete(&routes, "feature", "default", "/mail/m00.eml").is_none());
}

#[test]
fn a_mount_at_the_workspace_root_owns_everything_in_it() {
    let routes = vec![MountRoute {
        mount_path: "/".to_string(),
        ..route()
    }];
    assert!(route_for_delete(&routes, "main", "default", "/anything").is_some());
}

// ---------------------------------------------------------------------------
// end to end through the handler
// ---------------------------------------------------------------------------

fn handler(storage: &Arc<RocksDBStorage>, registry: Arc<JobRegistry>) -> UnifiedJobEventHandler {
    let (dispatcher, _rx) = JobDispatcher::new();
    UnifiedJobEventHandler::new(
        storage.clone(),
        registry,
        Arc::new(crate::jobs::JobDataStore::new(storage.db().clone())),
        Arc::new(dispatcher),
        storage.processing_rules_repository(),
    )
}

async fn queued(registry: &JobRegistry) -> Vec<JobType> {
    registry
        .list_jobs()
        .await
        .into_iter()
        .filter(|j| matches!(j.status, JobStatus::Scheduled))
        .map(|j| j.job_type)
        .collect()
}

/// A local edit to a mounted node enqueues a drain for THAT mount, addressed to
/// the mount's config branch rather than to the branch the edit landed on.
#[tokio::test]
async fn a_local_edit_enqueues_a_drain_on_the_config_branch() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(RocksDBStorage::new(dir.path()).unwrap());
    let registry = Arc::new(JobRegistry::new());
    let h = handler(&storage, registry.clone());
    // The mount materializes into `main` but its config lives on `trunk`.
    h.vmount_routes
        .seed(
            TENANT,
            REPO,
            vec![MountRoute {
                config_branch: "trunk".to_string(),
                ..route()
            }],
        )
        .await;

    h.capture_virtual_write(
        &event(NodeEventKind::Updated, "/mail/m00.eml", Some("alice")),
        Some(&mounted_node()),
    )
    .await;

    let jobs = queued(&registry).await;
    assert_eq!(
        jobs,
        vec![JobType::VirtualMountWriteDrain {
            mount_id: "m1".to_string(),
            trigger: "capture".to_string(),
        }]
    );
    // ...and on the config branch: a job enqueued against `main` would look up
    // the mount node where it does not exist and report `mount_not_found`.
    let stored = registry
        .list_jobs()
        .await
        .into_iter()
        .find(|j| matches!(j.job_type, JobType::VirtualMountWriteDrain { .. }))
        .unwrap();
    let ctx = h.job_data_store.get(TENANT, &stored.id).unwrap().unwrap();
    assert_eq!(ctx.branch, "trunk");
    assert_eq!(ctx.workspace_id, SYSTEM_WORKSPACE);
}

/// A burst of edits collapses to one pending drain.
#[tokio::test]
async fn a_burst_of_edits_collapses_to_one_drain() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(RocksDBStorage::new(dir.path()).unwrap());
    let registry = Arc::new(JobRegistry::new());
    let h = handler(&storage, registry.clone());
    h.vmount_routes.seed(TENANT, REPO, vec![route()]).await;

    for _ in 0..5 {
        h.capture_virtual_write(
            &event(NodeEventKind::Updated, "/mail/m00.eml", Some("alice")),
            Some(&mounted_node()),
        )
        .await;
    }
    assert_eq!(queued(&registry).await.len(), 1);
}

/// Nothing is enqueued for an ordinary node, for the sync's own echo, or for a
/// mount that is not in the route set (disabled, or `mode: off`).
#[tokio::test]
async fn edits_that_must_not_wake_a_mount() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(RocksDBStorage::new(dir.path()).unwrap());
    let registry = Arc::new(JobRegistry::new());
    let h = handler(&storage, registry.clone());
    h.vmount_routes.seed(TENANT, REPO, vec![route()]).await;
    let e = event(NodeEventKind::Updated, "/mail/m00.eml", Some("alice"));

    // An ordinary node, OUTSIDE any mount path.
    //
    // The path matters now. This case used to pass an unstamped node with a
    // path *inside* the mount, which under id-only routing was unambiguously
    // "ordinary". Once locally-born nodes route by path — so that outbox
    // commands, which never carry stamps, reach their mount without waiting for
    // the poll — that same fixture means the opposite: a node created inside a
    // mount's subtree, which SHOULD wake it. Moved outside so the case still
    // tests what it is named for.
    h.capture_virtual_write(
        &event(NodeEventKind::Updated, "/elsewhere/note.md", Some("alice")),
        Some(&Node::default()),
    )
    .await;
    // The sync engine's own write.
    h.capture_virtual_write(
        &event(NodeEventKind::Updated, "/mail/m00.eml", Some(SYNC_ACTOR)),
        Some(&mounted_node()),
    )
    .await;
    // No post-write node data — rejected before any routing happens, so the
    // in-mount path here is deliberate and still yields nothing.
    h.capture_virtual_write(&e, None).await;
    // A mount that asks for no writes is simply not in the route set.
    h.vmount_routes.seed(TENANT, REPO, Vec::new()).await;
    h.capture_virtual_write(&e, Some(&mounted_node())).await;

    assert!(queued(&registry).await.is_empty());
}

/// A delete under a mount path enqueues the RECONCILE walk, not a drain.
///
/// The drain works from the mount's live index and cannot see a node that is
/// gone; only the watermark walk can, via the MVCC pre-image.
#[tokio::test]
async fn a_local_delete_enqueues_the_reconcile_walk() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(RocksDBStorage::new(dir.path()).unwrap());
    let registry = Arc::new(JobRegistry::new());
    let h = handler(&storage, registry.clone());
    h.vmount_routes.seed(TENANT, REPO, vec![route()]).await;

    h.capture_virtual_delete(&event(
        NodeEventKind::Deleted,
        "/mail/m00.eml",
        Some("alice"),
    ))
    .await;
    assert_eq!(
        queued(&registry).await,
        vec![JobType::VirtualMountWriteReconcile {
            tenant_id: Some(TENANT.to_string()),
            repo_id: Some(REPO.to_string()),
        }]
    );

    // A delete outside every mount path is not this feature's business.
    h.capture_virtual_delete(&event(
        NodeEventKind::Deleted,
        "/somewhere/else",
        Some("alice"),
    ))
    .await;
    assert_eq!(queued(&registry).await.len(), 1);
}

/// A repository with no mounts caches an empty route set instead of re-reading
/// the system workspace on every local delete.
#[tokio::test]
async fn an_unmounted_repository_caches_its_emptiness() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(RocksDBStorage::new(dir.path()).unwrap());
    let cache = MountRouteCache::new();
    assert!(cache.routes(&storage, TENANT, REPO).await.is_empty());
    // Second call is served from the cache; identical `Arc` proves it was not
    // rebuilt.
    let a = cache.routes(&storage, TENANT, REPO).await;
    let b = cache.routes(&storage, TENANT, REPO).await;
    assert!(Arc::ptr_eq(&a, &b));
}

// ---------------------------------------------------------------------------
// the hook is actually WIRED
// ---------------------------------------------------------------------------

/// Only the virtual-mount jobs; the same event also schedules indexing.
async fn queued_vmount(registry: &JobRegistry) -> Vec<JobType> {
    queued(registry)
        .await
        .into_iter()
        .filter(|j| {
            matches!(
                j,
                JobType::VirtualMountWriteDrain { .. } | JobType::VirtualMountWriteReconcile { .. }
            )
        })
        .collect()
}

fn with_node_data(mut e: NodeEvent, node: &Node, source: Option<&str>) -> NodeEvent {
    let mut m = e.metadata.take().unwrap_or_default();
    m.insert("node_data".to_string(), serde_json::to_value(node).unwrap());
    if let Some(s) = source {
        m.insert(
            "source".to_string(),
            serde_json::Value::String(s.to_string()),
        );
    }
    e.metadata = Some(m);
    e
}

/// The hook is reached from the real entry points — `handle_node_change` and
/// `handle_node_delete` — and not just callable in isolation.
///
/// That placement is the whole point: it sits beside trigger evaluation,
/// downstream of `emit_node_events`, so the node API, SQL DML, functions and
/// flows are all covered by one hook rather than by one per writer.
#[tokio::test]
async fn the_hook_is_reached_from_the_event_entry_points() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(RocksDBStorage::new(dir.path()).unwrap());
    let registry = Arc::new(JobRegistry::new());
    let h = handler(&storage, registry.clone());
    h.vmount_routes.seed(TENANT, REPO, vec![route()]).await;

    let edit = with_node_data(
        event(NodeEventKind::Updated, "/mail/m00.eml", Some("alice")),
        &mounted_node(),
        None,
    );
    h.handle_node_change(&edit).await.unwrap();
    assert_eq!(
        queued_vmount(&registry).await,
        vec![JobType::VirtualMountWriteDrain {
            mount_id: "m1".to_string(),
            trigger: "capture".to_string(),
        }]
    );

    h.handle_node_delete(&event(
        NodeEventKind::Deleted,
        "/mail/m00.eml",
        Some("alice"),
    ))
    .await
    .unwrap();
    assert_eq!(
        queued_vmount(&registry).await.len(),
        2,
        "a delete must reach the reconcile walk through the same door"
    );
}

/// A REPLICATED edit captures nothing.
///
/// Writeback is one node's job. If every replica captured the replicated copy
/// of an edit, every replica would push it — N provider writes for one user
/// action, and, for a `submit` mount later, N emails.
#[tokio::test]
async fn a_replicated_edit_is_never_captured() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(RocksDBStorage::new(dir.path()).unwrap());
    let registry = Arc::new(JobRegistry::new());
    let h = handler(&storage, registry.clone());
    h.vmount_routes.seed(TENANT, REPO, vec![route()]).await;

    let replicated = with_node_data(
        event(NodeEventKind::Updated, "/mail/m00.eml", Some("alice")),
        &mounted_node(),
        Some("replication"),
    );
    h.handle_node_change(&replicated).await.unwrap();

    let mut deleted = event(NodeEventKind::Deleted, "/mail/m00.eml", Some("alice"));
    deleted.metadata.as_mut().unwrap().insert(
        "source".to_string(),
        serde_json::Value::String("replication".to_string()),
    );
    h.handle_node_delete(&deleted).await.unwrap();

    assert!(queued_vmount(&registry).await.is_empty());
}

/// An OUTBOX COMMAND is born locally and carries no mount stamps.
///
/// The materializer stamps `__virtual` + `__mount_id` on everything it creates,
/// so a mirrored edit routes by id. A command node — a checkout session the shop
/// writes and queues — has never been to the provider and has neither stamp.
///
/// `mount_id_of` therefore says "not mine", and before the path fallback that
/// ended capture right there: the command waited for the poll interval instead
/// of being sent. At the default 300s that is five minutes of a buyer watching a
/// spinner, on the one write where latency is a person.
///
/// The delete arm has always routed by path for the mirror-image reason, so the
/// fallback reuses it rather than inventing a second mechanism.
#[test]
fn a_locally_born_command_still_finds_its_mount_by_path() {
    let routes = vec![route()];

    // The shape create-order writes: inside the mount path, no stamps.
    let mut command = Node::default();
    command.path = "/mail/cs-ord-abc".to_string();
    assert_eq!(
        mount_id_of(&command),
        None,
        "a locally-born node must not claim a mount id it was never given"
    );

    // ...and is still routed, by the same helper the delete arm uses.
    assert!(
        route_for_delete(&routes, "main", "default", &command.path).is_some(),
        "a node created inside a mount's path must reach that mount, stamps or not"
    );

    // Scope still applies — the fallback widens WHICH nodes are considered,
    // never which mounts may claim them.
    assert!(route_for_delete(&routes, "feature", "default", &command.path).is_none());
    assert!(route_for_delete(&routes, "main", "other-ws", &command.path).is_none());
    assert!(route_for_delete(&routes, "main", "default", "/elsewhere/cs-ord-abc").is_none());
}
