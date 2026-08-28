//! End-to-end attribution for the persistent audit log.
//!
//! Exercises the real chain rather than the repository in isolation:
//!
//! ```text
//! AuthContext ─▶ transaction commit ─▶ NodeEvent.metadata ─▶ AuditEventHandler ─▶ RocksDBAuditRepo
//! ```
//!
//! That chain is the whole point of putting the agent marker on `AuthContext`:
//! every write path (node API, SQL DML, functions, MCP tools) commits through
//! this same transaction layer, so a marker that survives it is recorded no
//! matter which surface the write arrived on.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_audit::{AuditRepository, AuditScope};
use raisin_context::RepositoryConfig;
use raisin_core::services::workspace_service::WorkspaceService;
use raisin_error::Result;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::audit_log::AuditLogAction;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_models::nodes::Node;
use raisin_models::permissions::{Operation, Permission, ResolvedPermissions};
use raisin_models::workspace::Workspace;
use raisin_rocksdb::{fractional_index, RocksDBAuditRepo, RocksDBConfig, RocksDBStorage};
use raisin_storage::scope::BranchScope;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{
    BranchRepository, CommitMetadata, NodeRepository, NodeTypeRepository, RegistryRepository,
    RepositoryManagementRepository, Storage, StorageScope,
};
use tempfile::TempDir;
use uuid::Uuid;

const TENANT: &str = "audit-attr-test";
const REPO: &str = "main-repo";
const BRANCH: &str = "main";
const WORKSPACE: &str = "default";

/// An auditable type and a non-auditable one, so the `auditable` gate is
/// exercised alongside attribution.
const AUDITED_TYPE: &str = "test:Audited";
const PLAIN_TYPE: &str = "raisin:Folder";

fn node_type(name: &str, auditable: bool) -> NodeType {
    NodeType {
        id: Some(name.to_string()),
        strict: Some(false),
        name: name.to_string(),
        extends: None,
        mixins: Vec::new(),
        overrides: None,
        description: None,
        icon: None,
        version: Some(1),
        properties: None,
        allowed_children: vec!["*".to_string()],
        required_nodes: Vec::new(),
        initial_structure: None,
        versionable: Some(true),
        publishable: Some(true),
        auditable: Some(auditable),
        indexable: Some(true),
        index_types: None,
        created_at: Some(chrono::Utc::now()),
        updated_at: None,
        published_at: None,
        published_by: None,
        previous_version: None,
        compound_indexes: None,
        is_mixin: None,
    }
}

fn build_node(path: &str, ty: &str) -> Node {
    let name = path.trim_start_matches('/').to_string();
    let mut properties = HashMap::new();
    properties.insert("title".to_string(), PropertyValue::String(name.clone()));

    Node {
        id: Uuid::new_v4().to_string(),
        name,
        path: path.to_string(),
        node_type: ty.to_string(),
        archetype: None,
        properties,
        children: Vec::new(),
        order_key: fractional_index::first(),
        has_children: Some(false),
        parent: Some("/".to_string()),
        version: 1,
        created_at: Some(chrono::Utc::now()),
        updated_at: None,
        published_at: None,
        published_by: None,
        updated_by: None,
        created_by: None,
        translations: None,
        tenant_id: Some(TENANT.to_string()),
        workspace: Some(WORKSPACE.to_string()),
        owner_id: None,
        relations: Vec::new(),
    }
}

/// Storage with the audit subscriber wired exactly as `raisin-server` wires it.
async fn setup() -> Result<(Arc<RocksDBStorage>, Arc<RocksDBAuditRepo>, TempDir)> {
    let temp_dir = TempDir::new().unwrap();
    let mut config = RocksDBConfig::default();
    config.path = temp_dir.path().to_path_buf();
    let storage = Arc::new(RocksDBStorage::with_config(config)?);

    storage
        .registry()
        .register_tenant(TENANT, HashMap::new())
        .await?;
    storage
        .repository_management()
        .create_repository(
            TENANT,
            REPO,
            RepositoryConfig {
                default_language: "en".to_string(),
                supported_languages: vec!["en".to_string()],
                locale_fallback_chains: HashMap::new(),
                default_branch: BRANCH.to_string(),
                description: Some("audit attribution test".to_string()),
                tags: HashMap::new(),
            },
        )
        .await?;
    storage
        .branches()
        .create_branch(TENANT, REPO, BRANCH, "system", None, None, false, false)
        .await?;

    // Types before the workspace: WorkspaceService::put bootstraps a root node
    // and needs the default folder type registered.
    for (name, auditable) in [(PLAIN_TYPE, false), (AUDITED_TYPE, true)] {
        storage
            .node_types()
            .upsert(
                BranchScope::new(TENANT, REPO, BRANCH),
                node_type(name, auditable),
                CommitMetadata::system("seed type"),
            )
            .await?;
    }

    let mut workspace = Workspace::new(WORKSPACE.to_string());
    workspace.config.default_branch = BRANCH.to_string();
    WorkspaceService::new(storage.clone())
        .put(TENANT, REPO, workspace)
        .await?;

    let audit = Arc::new(storage.audit_repository());
    storage
        .event_bus()
        .subscribe(Arc::new(raisin_core::AuditEventHandler::new(
            audit.clone(),
            storage.clone(),
        )));

    Ok((storage, audit, temp_dir))
}

/// Commit one create as `auth`, returning the new node's id.
async fn create_as(
    storage: &Arc<RocksDBStorage>,
    path: &str,
    ty: &str,
    auth: AuthContext,
) -> Result<String> {
    let node = build_node(path, ty);
    let id = node.id.clone();

    let tx = storage.begin_context().await?;
    tx.set_tenant_repo(TENANT, REPO)?;
    tx.set_branch(BRANCH)?;
    tx.set_message("audit attribution test write")?;
    tx.set_auth_context(auth)?;
    tx.set_validate_schema(false)?;
    tx.add_node(WORKSPACE, &node).await?;
    tx.commit().await?;

    Ok(id)
}

/// A real (non-system) user context that RLS actually admits.
///
/// `AuthContext::for_user` alone carries no resolved permissions, which RLS
/// reads as deny-all — so writes need an explicit grant. The point of these
/// tests is attribution, not authorization, so the grant is deliberately broad.
fn user(user_id: &str) -> AuthContext {
    AuthContext::for_user(user_id).with_permissions(ResolvedPermissions {
        user_id: user_id.to_string(),
        email: None,
        direct_roles: vec![],
        group_roles: vec![],
        effective_roles: vec![],
        groups: vec![],
        permissions: vec![Permission::new(
            "/**",
            vec![
                Operation::Read,
                Operation::Create,
                Operation::Update,
                Operation::Delete,
            ],
        )],
        is_system_admin: false,
        resolved_at: Some(std::time::Instant::now()),
    })
}

fn scope<'a>() -> AuditScope<'a> {
    AuditScope::new(TENANT, REPO, BRANCH, WORKSPACE)
}

/// Events are dispatched on the bus; give the subscriber a moment to land the
/// write before reading it back.
async fn settle() {
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
}

#[tokio::test]
async fn agent_marker_reaches_the_audit_log_alongside_the_human() -> Result<()> {
    let (storage, audit, _tmp) = setup().await?;

    // What the MCP transport builds: the caller's real context, marked with the
    // server slug it arrived through.
    let auth = user("alice").with_agent("mcp:studio-admin");
    let id = create_as(&storage, "/audited-via-mcp", AUDITED_TYPE, auth).await?;
    settle().await;

    let logs = audit.get_logs_scoped(scope(), &id, None).await?;
    assert_eq!(logs.len(), 1, "one create ⇒ one audit entry");
    assert_eq!(logs[0].action, AuditLogAction::Create);
    assert_eq!(
        logs[0].user_id.as_deref(),
        Some("alice"),
        "the human must still be recorded"
    );
    assert_eq!(
        logs[0].agent.as_deref(),
        Some("mcp:studio-admin"),
        "the agent marker must survive AuthContext → commit → event → audit"
    );
    Ok(())
}

#[tokio::test]
async fn a_direct_human_write_records_no_agent() -> Result<()> {
    let (storage, audit, _tmp) = setup().await?;

    let id = create_as(&storage, "/audited-direct", AUDITED_TYPE, user("bob")).await?;
    settle().await;

    let logs = audit.get_logs_scoped(scope(), &id, None).await?;
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].user_id.as_deref(), Some("bob"));
    assert_eq!(
        logs[0].agent, None,
        "an unmarked write must not acquire an agent"
    );
    Ok(())
}

// NOTE: persistence across a reopen is covered by the repository unit test
// `repositories::audit::tests::entries_survive_reopening_the_database`, not here.
// An end-to-end variant cannot reopen the data dir in-process: the audit
// subscriber holds an `Arc<RocksDBStorage>` while the event bus it is registered
// on lives inside that same storage, so the resulting cycle keeps the RocksDB
// handle (and its file lock) alive for the life of the process. That is exactly
// how `raisin-server` wires it and is harmless there.

#[tokio::test]
async fn non_auditable_types_still_produce_nothing() -> Result<()> {
    let (storage, audit, _tmp) = setup().await?;

    let id = create_as(
        &storage,
        "/plain-folder",
        PLAIN_TYPE,
        user("dave").with_agent("mcp:studio-admin"),
    )
    .await?;
    settle().await;

    assert!(
        audit.get_logs_scoped(scope(), &id, None).await?.is_empty(),
        "the `auditable` gate must still suppress entries, marked or not"
    );
    Ok(())
}

/// Decision (A): the cluster is masterless, so an operation replayed on a peer
/// must produce an audit entry identical to the one the originating node wrote.
/// Before `Operation.agent` existed, replicated events carried only
/// `{"source":"replication"}` and every replica recorded the write as nobody's.
#[tokio::test]
async fn a_replicated_write_is_attributed_exactly_as_on_the_originating_node() -> Result<()> {
    use raisin_replication::{
        operation::{ReplicatedNodeChange, ReplicatedNodeChangeKind},
        OpType, Operation as ReplOperation, VectorClock,
    };
    use raisin_rocksdb::replication::OperationApplicator;

    let (storage, audit, _tmp) = setup().await?;

    let applicator = OperationApplicator::new(
        storage.db().clone(),
        storage.event_bus(),
        Arc::new(storage.branches_impl().clone()),
    );

    // Exactly what an upstream node captures for a write made by `alice`
    // through the studio-admin MCP server.
    let node = build_node("/replicated-doc", AUDITED_TYPE);
    let id = node.id.clone();
    let revision = raisin_hlc::HLC::new(42, 0);
    let op = ReplOperation {
        op_id: Uuid::new_v4(),
        op_seq: 1,
        cluster_node_id: "peer-that-accepted-the-write".to_string(),
        timestamp_ms: 0,
        vector_clock: VectorClock::new(),
        tenant_id: TENANT.to_string(),
        repo_id: REPO.to_string(),
        branch: BRANCH.to_string(),
        op_type: OpType::ApplyRevision {
            branch_head: revision,
            node_changes: vec![ReplicatedNodeChange {
                node: node.clone(),
                parent_id: Some("/".to_string()),
                kind: ReplicatedNodeChangeKind::Upsert,
                cf_order_key: format!("{}::{}", node.order_key, node.id),
            }],
        },
        revision: Some(revision),
        actor: "alice".to_string(),
        agent: Some("mcp:studio-admin".to_string()),
        message: None,
        is_system: false,
        acknowledged_by: Default::default(),
    };

    applicator.apply_operation(&op).await.expect("apply");
    settle().await;

    let logs = audit.get_logs_scoped(scope(), &id, None).await?;
    assert_eq!(logs.len(), 1, "the replayed write must be audited here too");
    assert_eq!(
        logs[0].user_id.as_deref(),
        Some("alice"),
        "a replica is not a lesser copy: the human must survive replication"
    );
    assert_eq!(
        logs[0].agent.as_deref(),
        Some("mcp:studio-admin"),
        "the initiating principal must survive replication"
    );
    Ok(())
}

/// Decision (B): `agent` names the actor that *initiated* the write and is set
/// whether or not a human is behind it. An AI agent runs with no human, which is
/// exactly why naming it matters — otherwise the write is attributable to
/// nothing at all.
#[tokio::test]
async fn an_agent_run_with_no_human_still_records_the_agent() -> Result<()> {
    use raisin_models::auth::agent_identity;

    let (storage, audit, _tmp) = setup().await?;

    // What `resolve_auth_context_for_tool_call` now builds: system privileges
    // (unchanged) plus the identity of the agent doing the work.
    let auth = AuthContext::system().with_agent(agent_identity::agent("/agents/triage-bot"));
    let id = create_as(&storage, "/written-by-agent", AUDITED_TYPE, auth).await?;
    settle().await;

    let logs = audit.get_logs_scoped(scope(), &id, None).await?;
    assert_eq!(logs.len(), 1);
    assert_eq!(
        logs[0].agent.as_deref(),
        Some("agent:/agents/triage-bot"),
        "no human is not a reason to leave the initiator unset"
    );
    // There is no human here. `user_id` holds the system principal rather than a
    // person; the point of the assertion is that it is NOT a human identity, so
    // `agent` is the only thing that says who acted.
    assert_eq!(logs[0].user_id.as_deref(), Some("system"));
    Ok(())
}

/// Decision (C): an agent fired by a trigger must be traceable to the trigger
/// AND the agent, not just the agent.
#[tokio::test]
async fn a_trigger_fired_agent_records_both_the_agent_and_the_trigger() -> Result<()> {
    use raisin_models::auth::agent_identity;

    let (storage, audit, _tmp) = setup().await?;

    let marker = agent_identity::with_origin(
        agent_identity::agent("/agents/triage-bot"),
        Some("trigger:/triggers/on-order-created"),
    );
    let id = create_as(
        &storage,
        "/written-by-triggered-agent",
        AUDITED_TYPE,
        AuthContext::system().with_agent(marker),
    )
    .await?;
    settle().await;

    let logs = audit.get_logs_scoped(scope(), &id, None).await?;
    assert_eq!(
        logs[0].agent.as_deref(),
        Some("agent:/agents/triage-bot@trigger:/triggers/on-order-created"),
        "both hops must be recoverable from the entry"
    );
    // Kind stays leftmost so `LIKE 'agent:%'` still selects agent-driven writes.
    assert!(logs[0].agent.as_deref().unwrap().starts_with("agent:"));
    Ok(())
}

/// Scoping is what makes the persistent store tenant-safe. A read in the wrong
/// tenant/branch must not see another scope's history even with the right id.
#[tokio::test]
async fn reads_do_not_cross_tenant_or_branch() -> Result<()> {
    let (storage, audit, _tmp) = setup().await?;

    let id = create_as(&storage, "/scoped", AUDITED_TYPE, user("erin")).await?;
    settle().await;

    assert_eq!(audit.get_logs_scoped(scope(), &id, None).await?.len(), 1);
    for other in [
        AuditScope::new("other-tenant", REPO, BRANCH, WORKSPACE),
        AuditScope::new(TENANT, REPO, "other-branch", WORKSPACE),
    ] {
        assert!(
            audit.get_logs_scoped(other, &id, None).await?.is_empty(),
            "scope isolation breached"
        );
    }
    Ok(())
}

/// Decision (3): ONE logical delete must produce ONE audit row.
///
/// `apply_delete_node` used to call `apply_replicated_delete` (which emits
/// `Deleted`) and then emit `Deleted` a second time with byte-identical
/// arguments. Two rows is wrong on its own; once attribution made the rows
/// identical it also became visible. The duplicate is not cosmetic — every
/// `Deleted` subscriber ran twice, including the trigger/job dispatcher.
#[tokio::test]
async fn a_replicated_delete_produces_exactly_one_audit_row() -> Result<()> {
    use raisin_replication::{OpType, Operation as ReplOperation, VectorClock};
    use raisin_rocksdb::replication::OperationApplicator;

    let (storage, audit, _tmp) = setup().await?;

    // A node that actually exists locally: apply_delete_node loads the latest
    // snapshot and no-ops when the node is missing.
    let id = create_as(&storage, "/doomed", AUDITED_TYPE, user("alice")).await?;
    settle().await;

    let applicator = OperationApplicator::new(
        storage.db().clone(),
        storage.event_bus(),
        Arc::new(storage.branches_impl().clone()),
    );

    let revision = raisin_hlc::HLC::new(99, 0);
    let op = ReplOperation {
        op_id: Uuid::new_v4(),
        op_seq: 2,
        cluster_node_id: "peer-that-accepted-the-delete".to_string(),
        timestamp_ms: 0,
        vector_clock: VectorClock::new(),
        tenant_id: TENANT.to_string(),
        repo_id: REPO.to_string(),
        branch: BRANCH.to_string(),
        op_type: OpType::DeleteNode {
            node_id: id.clone(),
        },
        revision: Some(revision),
        actor: "alice".to_string(),
        agent: Some("trigger:/triggers/cleanup".to_string()),
        message: None,
        is_system: false,
        acknowledged_by: Default::default(),
    };

    applicator.apply_operation(&op).await.expect("apply delete");
    settle().await;

    let deletes: Vec<_> = audit
        .get_logs_scoped(scope(), &id, None)
        .await?
        .into_iter()
        .filter(|l| l.action == AuditLogAction::Delete)
        .collect();

    assert_eq!(
        deletes.len(),
        1,
        "one logical delete ⇒ one audit row, got {}: {:?}",
        deletes.len(),
        deletes
            .iter()
            .map(|l| (l.user_id.clone(), l.agent.clone()))
            .collect::<Vec<_>>()
    );
    // The surviving row is the attributed one, not a stripped fallback.
    assert_eq!(deletes[0].user_id.as_deref(), Some("alice"));
    assert_eq!(
        deletes[0].agent.as_deref(),
        Some("trigger:/triggers/cleanup")
    );
    Ok(())
}

/// The same invariant for the snapshot delete path (`DeleteNodeSnapshot`),
/// which shares `apply_replicated_delete` and must stay at one event too.
#[tokio::test]
async fn a_replicated_snapshot_delete_produces_exactly_one_audit_row() -> Result<()> {
    use raisin_replication::{OpType, Operation as ReplOperation, VectorClock};
    use raisin_rocksdb::replication::OperationApplicator;

    let (storage, audit, _tmp) = setup().await?;
    let id = create_as(&storage, "/doomed-snapshot", AUDITED_TYPE, user("alice")).await?;
    settle().await;

    let applicator = OperationApplicator::new(
        storage.db().clone(),
        storage.event_bus(),
        Arc::new(storage.branches_impl().clone()),
    );

    let revision = raisin_hlc::HLC::new(101, 0);
    let op = ReplOperation {
        op_id: Uuid::new_v4(),
        op_seq: 3,
        cluster_node_id: "peer".to_string(),
        timestamp_ms: 0,
        vector_clock: VectorClock::new(),
        tenant_id: TENANT.to_string(),
        repo_id: REPO.to_string(),
        branch: BRANCH.to_string(),
        op_type: OpType::DeleteNodeSnapshot {
            node_id: id.clone(),
            revision,
        },
        revision: Some(revision),
        actor: "alice".to_string(),
        agent: None,
        message: None,
        is_system: false,
        acknowledged_by: Default::default(),
    };

    applicator.apply_operation(&op).await.expect("apply delete");
    settle().await;

    let deletes = audit
        .get_logs_scoped(scope(), &id, None)
        .await?
        .into_iter()
        .filter(|l| l.action == AuditLogAction::Delete)
        .count();
    assert_eq!(deletes, 1, "one logical delete ⇒ one audit row");
    Ok(())
}

/// Decision (1), consumer half: a node event stamped by a flow write must land
/// in the audit log naming the flow AND the trigger behind it.
///
/// The producer is `flow_event_metadata` in
/// `raisin-functions/src/execution/flow_callbacks_factory/node_callbacks.rs`
/// (unit-tested there); this asserts the audit subscriber reads exactly those
/// keys off a flow-shaped event. Before this work the flow path sent
/// `metadata: None`, so a flow-driven write was recorded as nobody's.
#[tokio::test]
async fn a_flow_stamped_node_event_is_audited_with_the_flow_and_trigger() -> Result<()> {
    let (storage, audit, _tmp) = setup().await?;

    // The composed marker a flow-backed trigger mints.
    let marker = "flow:/flows/publish-approval@trigger:/triggers/on-order-created";

    // Byte-for-byte the bag the flow node callbacks now publish.
    let mut metadata = HashMap::new();
    metadata.insert(
        "source".to_string(),
        serde_json::Value::String("local".to_string()),
    );
    metadata.insert(
        "actor".to_string(),
        serde_json::Value::String("system".to_string()),
    );
    metadata.insert(
        "agent".to_string(),
        serde_json::Value::String(marker.to_string()),
    );

    let node = build_node("/written-by-flow", AUDITED_TYPE);
    storage
        .event_bus()
        .publish(raisin_storage::Event::Node(raisin_storage::NodeEvent {
            tenant_id: TENANT.to_string(),
            repository_id: REPO.to_string(),
            branch: BRANCH.to_string(),
            workspace_id: WORKSPACE.to_string(),
            node_id: node.id.clone(),
            node_type: Some(node.node_type.clone()),
            revision: raisin_hlc::HLC::new(7, 0),
            kind: raisin_storage::NodeEventKind::Created,
            path: Some(node.path.clone()),
            metadata: Some(metadata),
        }));
    settle().await;

    let logs = audit.get_logs_scoped(scope(), &node.id, None).await?;
    assert_eq!(logs.len(), 1, "one flow write ⇒ one audit entry");
    assert_eq!(
        logs[0].agent.as_deref(),
        Some(marker),
        "both hops must be recoverable: the flow and the trigger that fired it"
    );
    assert_eq!(
        logs[0].user_id.as_deref(),
        Some("system"),
        "a flow step has no human behind it; `agent` is what names the actor"
    );
    Ok(())
}

/// A write with no human behind it records the AGENT on the node itself.
///
/// Every flow, agent and trigger executes under `AuthContext::system()`, whose
/// `actor_id()` is the word "system" — so before this, all of them collapsed
/// into one anonymous writer on `updated_by`, the one field a UI reads to say
/// who touched a document, while the audit row beside it recorded exactly which
/// one. An automation could rewrite a page and leave no trace of itself on it.
#[tokio::test]
async fn an_automations_write_names_the_automation_on_the_node() -> Result<()> {
    let (storage, _audit, _tmp) = setup().await?;

    let marker = "flow:/flows/publish-approval@trigger:/triggers/on-order-created";
    let auth = AuthContext::system().with_agent(marker);
    let id = create_as(&storage, "/written-by-a-flow", AUDITED_TYPE, auth).await?;

    let node = storage
        .nodes()
        .get(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            &id,
            None,
        )
        .await?
        .expect("the node exists");

    assert_eq!(
        node.updated_by.as_deref(),
        Some(marker),
        "the flow (and the trigger behind it) must be readable off the node"
    );
    assert_eq!(
        node.created_by.as_deref(),
        Some(marker),
        "a node an automation created has no other author"
    );
    Ok(())
}

/// A HUMAN still outranks the marker: a function called on somebody's behalf is
/// that person's write, however it reached the engine.
#[tokio::test]
async fn a_human_behind_an_agent_is_still_the_author() -> Result<()> {
    let (storage, _audit, _tmp) = setup().await?;

    let auth = user("alice").with_agent("mcp:studio-admin");
    let id = create_as(&storage, "/written-via-mcp", AUDITED_TYPE, auth).await?;

    let node = storage
        .nodes()
        .get(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            &id,
            None,
        )
        .await?
        .expect("the node exists");

    assert_eq!(
        node.updated_by.as_deref(),
        Some("alice"),
        "the person is the author; the agent marker is how they got here"
    );
    Ok(())
}

/// A plain system write — no agent anywhere — is unchanged.
#[tokio::test]
async fn an_unmarked_system_write_still_reads_as_system() -> Result<()> {
    let (storage, _audit, _tmp) = setup().await?;

    let id = create_as(
        &storage,
        "/written-by-system",
        AUDITED_TYPE,
        AuthContext::system(),
    )
    .await?;

    let node = storage
        .nodes()
        .get(
            StorageScope::new(TENANT, REPO, BRANCH, WORKSPACE),
            &id,
            None,
        )
        .await?
        .expect("the node exists");

    assert_eq!(node.updated_by.as_deref(), Some("system"));
    Ok(())
}
