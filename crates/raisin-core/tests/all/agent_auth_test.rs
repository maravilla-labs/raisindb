// SPDX-License-Identifier: BSL-1.1

//! WHOSE PERMISSIONS an agent runs with.
//!
//! An agent reaches storage from two directions — its own tool-call job, and a
//! tool call inside a flow step — and both now go through
//! `services::agent_auth`. These tests hold that resolver to the three answers
//! that matter, because each failure mode is silent in a different way:
//!
//!   * granted rights that are NOT applied leave an agent with full system
//!     privileges while its configuration says otherwise — the UI lies;
//!   * an empty grant list treated as a lockout leaves the agent able to do
//!     NOTHING, with no error anyone can read;
//!   * an unreadable agent treated as "no restriction" is a silent widening,
//!     which is the one outcome nobody notices.

use std::collections::HashMap;
use std::sync::Arc;

use raisin_core::services::agent_auth::{execution_of, resolve_agent_context, AgentExecution};
use raisin_models::nodes::{properties::PropertyValue, Node};
use raisin_storage::{CreateNodeOptions, NodeRepository, Storage, StorageScope};
use raisin_storage_memory::InMemoryStorage;

const TENANT: &str = "default";
const REPO: &str = "example";
const BRANCH: &str = "main";
const FUNCTIONS: &str = "functions";
const ACCESS: &str = "raisin:access_control";

fn s(value: &str) -> PropertyValue {
    PropertyValue::String(value.to_string())
}

fn list(values: &[&str]) -> PropertyValue {
    PropertyValue::Array(values.iter().map(|v| s(v)).collect())
}

fn node(path: &str, node_type: &str, properties: HashMap<String, PropertyValue>) -> Node {
    Node {
        id: nanoid::nanoid!(),
        name: path.rsplit('/').next().unwrap().to_string(),
        path: path.to_string(),
        node_type: node_type.to_string(),
        properties,
        created_at: Some(chrono::Utc::now()),
        ..Default::default()
    }
}

fn unvalidated() -> CreateNodeOptions {
    CreateNodeOptions {
        validate_schema: false,
        validate_parent_allows_child: false,
        validate_workspace_allows_type: false,
        ..Default::default()
    }
}

async fn put(storage: &Arc<InMemoryStorage>, workspace: &str, n: Node) {
    storage
        .nodes()
        .create(
            StorageScope::new(TENANT, REPO, BRANCH, workspace),
            n,
            unvalidated(),
        )
        .await
        .expect("seed node");
}

/// A role granting exactly one thing, so a resolved context is recognisable.
async fn seed_role(storage: &Arc<InMemoryStorage>) {
    let mut props = HashMap::new();
    props.insert("role_id".to_string(), s("ticket_reader"));
    props.insert("name".to_string(), s("Ticket reader"));
    props.insert(
        "permissions".to_string(),
        PropertyValue::Array(vec![PropertyValue::Object(HashMap::from([
            ("path".to_string(), s("/tickets/**")),
            (
                "operations".to_string(),
                PropertyValue::Array(vec![s("read")]),
            ),
            ("workspace".to_string(), s("ticketing")),
        ]))]),
    );
    put(
        storage,
        ACCESS,
        node("/roles/ticket-reader", "raisin:Role", props),
    )
    .await;
}

async fn seed_agent(storage: &Arc<InMemoryStorage>, path: &str, context: &str, roles: &[&str]) {
    let mut props = HashMap::new();
    props.insert("title".to_string(), s("Bot"));
    props.insert("execution_context".to_string(), s(context));
    if !roles.is_empty() {
        props.insert("roles".to_string(), list(roles));
    }
    put(storage, FUNCTIONS, node(path, "raisin:AIAgent", props)).await;
}

const MARKER: &str = "agent:/agents/bot@flow:/flows/triage";

async fn resolve(
    storage: &Arc<InMemoryStorage>,
    path: &str,
) -> Result<Option<raisin_models::auth::AuthContext>, String> {
    resolve_agent_context(storage, TENANT, REPO, BRANCH, FUNCTIONS, path, MARKER).await
}

#[tokio::test]
async fn an_agent_with_its_own_roles_runs_under_them() {
    let storage = Arc::new(InMemoryStorage::default());
    seed_role(&storage).await;
    seed_agent(&storage, "/agents/bot", "agent", &["ticket_reader"]).await;

    let ctx = resolve(&storage, "/agents/bot")
        .await
        .expect("resolution succeeds")
        .expect("the agent asked for its own rights");

    assert!(
        !ctx.is_system,
        "an agent under its own roles is NOT the system"
    );
    assert_eq!(
        ctx.user_id.as_deref(),
        Some(MARKER),
        "the agent's marker identifies it, so RLS conditions and the authorship stamp agree",
    );
    assert_eq!(ctx.agent.as_deref(), Some(MARKER), "provenance survives");

    // WHAT THIS HARNESS CAN AND CANNOT SHOW. The agent's own role list reaches
    // the resolved context, and the context is not an admin — which is the part
    // this resolver owns. The role NODE's grants do not materialize here: role
    // lookup is `find_by_property("role_id")`, and the in-memory backend's
    // property index is not populated for a node type the test never
    // registered. Expanding grants is `PermissionService`'s job and has its own
    // tests; asserting it through this seam would be asserting the harness.
    let permissions = ctx.permissions().expect("permissions resolved");
    assert!(!permissions.is_system_admin, "a narrow role is NOT admin");
    assert!(
        permissions
            .direct_roles
            .iter()
            .any(|r| r == "ticket_reader"),
        "the agent's own role list is what it runs under: {:?}",
        permissions.direct_roles,
    );
}

#[tokio::test]
async fn an_agent_that_did_not_ask_keeps_the_callers_context() {
    let storage = Arc::new(InMemoryStorage::default());
    seed_role(&storage).await;
    // Roles present, but the agent runs as the system — the default, and the
    // behaviour every existing agent must keep.
    seed_agent(&storage, "/agents/bot", "system", &["ticket_reader"]).await;

    assert!(resolve(&storage, "/agents/bot").await.unwrap().is_none());
}

#[tokio::test]
async fn asking_for_own_rights_with_no_grants_falls_back_rather_than_locking_out() {
    let storage = Arc::new(InMemoryStorage::default());
    seed_agent(&storage, "/agents/bot", "agent", &[]).await;

    assert!(
        resolve(&storage, "/agents/bot").await.unwrap().is_none(),
        "an agent that could do nothing at all is a silent, baffling failure; \
         the caller's context stands and the resolver logs why",
    );
}

#[tokio::test]
async fn a_missing_agent_is_not_an_error() {
    let storage = Arc::new(InMemoryStorage::default());

    assert!(
        resolve(&storage, "/agents/gone").await.unwrap().is_none(),
        "an agent node that moved must not break flows that work today",
    );
}

#[test]
fn execution_context_reads_exactly_one_word_as_own_rights() {
    let own = node(
        "/agents/a",
        "raisin:AIAgent",
        HashMap::from([("execution_context".to_string(), s("agent"))]),
    );
    assert_eq!(execution_of(&own), AgentExecution::OwnRights);

    for other in ["system", "user", "", "AGENT"] {
        let n = node(
            "/agents/a",
            "raisin:AIAgent",
            HashMap::from([("execution_context".to_string(), s(other))]),
        );
        assert_eq!(execution_of(&n), AgentExecution::System, "{other}");
    }

    // Absent is the legacy default, and legacy means system.
    let bare = node("/agents/a", "raisin:AIAgent", HashMap::new());
    assert_eq!(execution_of(&bare), AgentExecution::System);
}
