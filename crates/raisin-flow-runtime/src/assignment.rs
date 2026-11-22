// SPDX-License-Identifier: BSL-1.1

//! Group task assignments: one task per member, one record the members cannot touch.
//!
//! A group task ("someone bring toilet paper", "everyone confirm the trip", "we need three helpers")
//! is delivered as an ordinary `raisin:InboxTask` into every member's own inbox. A person may update
//! anything in their own inbox, so the AUTHORITY of the group — who belongs to it, the completion
//! policy, who answered what — lives in a system-only `raisin:TaskAssignment` record at
//! `raisin:access_control:/task-assignments/<id>`:
//!
//! * the record holds the policy and the member task paths;
//! * every answer is a `raisin:TaskAssignmentResponse` child (kind `response`), created once by
//!   [`crate::service::complete_task`], so counting never reads a member's copy;
//! * the record is closed by its `closed` child, which only one completion can create.
//!
//! The members' copies carry `assignment_id` and DISPLAY copies of the policy for their task UI.
//! Nothing reads those back as authority.
//!
//! Writers: [`create_assignment`] (storage level: `raisin.tasks.create`, the messaging outbox
//! handler through it) and the workflow `human_task` step (through its callbacks, sharing
//! [`record_properties`] and [`member_task_properties`]). See docs/plans/group-messaging-and-tasks.md.

use std::collections::{HashMap, HashSet};

use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::{
    CreateNodeOptions, ListOptions, NodeRepository, RelationRepository, Storage, StorageScope,
};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::types::FlowError;

/// Workspace of user homes, inbox tasks and assignment records.
pub const WORKSPACE: &str = "raisin:access_control";
/// Branch every inbox lives on.
pub const BRANCH: &str = "main";
/// Folder of the records.
pub const ASSIGNMENTS_ROOT: &str = "/task-assignments";
pub const RECORD_TYPE: &str = "raisin:TaskAssignment";
pub const RESPONSE_TYPE: &str = "raisin:TaskAssignmentResponse";
pub const TASK_TYPE: &str = "raisin:InboxTask";
/// Name of the marker child that closes a record.
pub const CLOSED: &str = "closed";
/// Relation from a user to the circle they belong to.
pub const MEMBER_OF: &str = "MEMBER_OF";
/// Fan-out guard: one task per member is written in one request.
pub const MAX_MEMBERS: usize = 1000;

/// The completion policy of one assignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    /// `any` | `all` | `quorum`
    pub completion: String,
    /// Qualifying answers that satisfy it.
    pub required: usize,
    /// Optional REL deciding whether an answer qualifies.
    pub response_condition: Option<String>,
}

impl Policy {
    /// `any` needs one answer, `all` every member, `quorum` exactly `required` (1..=members).
    pub fn new(
        completion: Option<&str>,
        required: Option<u64>,
        response_condition: Option<String>,
        members: usize,
    ) -> Result<Self, FlowError> {
        let completion = completion.unwrap_or("any").trim().to_ascii_lowercase();
        let required = match completion.as_str() {
            "any" => 1,
            "all" => members,
            "quorum" => required.unwrap_or(1) as usize,
            other => {
                return Err(FlowError::InvalidNodeConfiguration(format!(
                    "invalid completion '{}': expected any, all or quorum",
                    other
                )))
            }
        };
        if members == 0 {
            return Err(FlowError::InvalidNodeConfiguration(
                "the group has no members".to_string(),
            ));
        }
        if required == 0 || required > members {
            return Err(FlowError::InvalidNodeConfiguration(format!(
                "a {} of {} needs between 1 and {} answers",
                completion, members, members
            )));
        }
        let response_condition = response_condition.filter(|c| !c.trim().is_empty());
        Ok(Self {
            completion,
            required,
            response_condition,
        })
    }
}

/// Who belongs to a group, resolved now.
#[derive(Debug, Clone)]
pub struct GroupMembers {
    /// Path of the circle or group.
    pub group_path: String,
    /// Home paths of the members, sorted, no duplicates.
    pub members: Vec<String>,
}

/// Path of a record.
pub fn record_path(assignment_id: &str) -> String {
    format!("{}/{}", ASSIGNMENTS_ROOT, assignment_id)
}

/// Name of the response child for one member task: stable, path-safe, never the task path itself.
pub fn response_name(task_path: &str) -> String {
    let digest = Sha256::digest(task_path.as_bytes());
    format!("r-{}", &hex::encode(digest)[..16])
}

/// Is `id` usable as a record name (and so inside a path)?
pub fn valid_assignment_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 96
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The record's properties. `status` starts `open`.
pub fn record_properties(
    group_path: &str,
    policy: &Policy,
    task_paths: &[String],
    flow_instance_id: Option<&str>,
    step_id: Option<&str>,
    assigned_by: Option<&Value>,
) -> Value {
    let mut props = json!({
        "group": group_path,
        "completion": policy.completion,
        "required": policy.required,
        "total": task_paths.len(),
        "task_paths": task_paths,
        "status": "open",
        "created_at": chrono::Utc::now().to_rfc3339(),
    });
    if let Some(condition) = &policy.response_condition {
        props["response_condition"] = json!(condition);
    }
    if let Some(id) = flow_instance_id {
        props["flow_instance_id"] = json!(id);
    }
    if let Some(step) = step_id {
        props["step_id"] = json!(step);
    }
    if let Some(by) = assigned_by.filter(|v| v.is_object()) {
        props["assigned_by"] = by.clone();
    }
    props
}

/// The display slots a member's task carries next to its own content. Never the other members'
/// task paths: a person's copy names nobody else's inbox.
pub fn member_task_properties(
    base: &Value,
    assignee: &str,
    assignment_id: &str,
    group_path: &str,
    policy: &Policy,
    total: usize,
) -> Value {
    let mut props = base.clone();
    if !props.is_object() {
        props = json!({});
    }
    props["assignee"] = json!(assignee);
    props["status"] = json!("pending");
    props["assignment_id"] = json!(assignment_id);
    props["assignment_group"] = json!(group_path);
    props["assignment_completion"] = json!(policy.completion);
    props["assignment_required"] = json!(policy.required);
    props["assignment_total"] = json!(total);
    props["assignment_status"] = json!("open");
    props["assignment_responded"] = json!(0);
    props["assignment_qualifying"] = json!(0);
    if let Some(condition) = &policy.response_condition {
        props["response_condition"] = json!(condition);
    }
    if let Some(map) = props.as_object_mut() {
        map.remove("assignment_task_paths");
    }
    props
}

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

fn scope<'a>(tenant_id: &'a str, repo: &'a str) -> StorageScope<'a> {
    StorageScope::new(tenant_id, repo, BRANCH, WORKSPACE)
}

fn other(error: impl std::fmt::Display) -> FlowError {
    FlowError::Other(error.to_string())
}

/// JSON object → property bag (numbers stay integers, objects and arrays nest).
pub(crate) fn property_bag(value: &Value) -> Result<HashMap<String, PropertyValue>, FlowError> {
    serde_json::from_value(value.clone()).map_err(|e| other(format!("invalid properties: {}", e)))
}

pub(crate) fn prop_str(node: &Node, key: &str) -> Option<String> {
    match node.properties.get(key) {
        Some(PropertyValue::String(s)) => Some(s.clone()),
        _ => None,
    }
}

pub(crate) fn prop_usize(node: &Node, key: &str) -> Option<usize> {
    match node.properties.get(key) {
        Some(PropertyValue::Integer(v)) if *v >= 0 => Some(*v as usize),
        Some(PropertyValue::Float(v)) if *v >= 0.0 => Some(*v as usize),
        _ => None,
    }
}

pub(crate) fn prop_strings(node: &Node, key: &str) -> Vec<String> {
    match node.properties.get(key) {
        Some(PropertyValue::Array(values)) => values
            .iter()
            .filter_map(|v| match v {
                PropertyValue::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn no_validation() -> CreateNodeOptions {
    // The engine writes its own records and the members' tasks as the system (the same
    // privilege the human_task step and raisin.tasks.create have always had); the shapes are
    // fixed here and pinned by tests.
    CreateNodeOptions {
        validate_schema: false,
        validate_parent_allows_child: false,
        validate_workspace_allows_type: false,
        ..Default::default()
    }
}

/// Announce a write the way NodeService does, so triggers, the indexer and every person's
/// live subscription on their own inbox see it.
pub(crate) async fn publish<S: Storage>(
    storage: &S,
    tenant_id: &str,
    repo: &str,
    node: &Node,
    kind: raisin_storage::NodeEventKind,
) {
    use raisin_storage::BranchRepository;
    let revision = match storage.branches().get_branch(tenant_id, repo, BRANCH).await {
        Ok(Some(branch)) => branch.head,
        _ => raisin_hlc::HLC::new(0, 0),
    };
    storage
        .event_bus()
        .publish(raisin_storage::Event::Node(raisin_storage::NodeEvent {
            tenant_id: tenant_id.to_string(),
            repository_id: repo.to_string(),
            branch: BRANCH.to_string(),
            workspace_id: WORKSPACE.to_string(),
            node_id: node.id.clone(),
            node_type: Some(node.node_type.clone()),
            revision,
            kind,
            path: Some(node.path.clone()),
            metadata: None,
        }));
}

/// Create a node the engine owns. `Ok(false)` when the id or path is already taken: the caller
/// decides whether that is idempotency (a redelivery) or a lost race (somebody closed first).
pub(crate) async fn create_owned<S: Storage>(
    storage: &S,
    tenant_id: &str,
    repo: &str,
    id: &str,
    path: &str,
    node_type: &str,
    properties: &Value,
) -> Result<bool, FlowError> {
    let nodes = storage.nodes();
    if nodes
        .get(scope(tenant_id, repo), id, None)
        .await
        .map_err(other)?
        .is_some()
        || nodes
            .get_by_path(scope(tenant_id, repo), path, None)
            .await
            .map_err(other)?
            .is_some()
    {
        return Ok(false);
    }
    let name = path.rsplit('/').next().unwrap_or(path).to_string();
    let parent = match path.rsplit_once('/') {
        Some(("", _)) | None => None,
        Some((parent, _)) => Some(parent.to_string()),
    };
    let node = Node {
        id: id.to_string(),
        name,
        path: path.to_string(),
        node_type: node_type.to_string(),
        properties: property_bag(properties)?,
        parent,
        created_at: Some(chrono::Utc::now()),
        workspace: Some(WORKSPACE.to_string()),
        ..Default::default()
    };
    match nodes
        .create(scope(tenant_id, repo), node.clone(), no_validation())
        .await
    {
        Ok(()) => {
            publish(
                storage,
                tenant_id,
                repo,
                &node,
                raisin_storage::NodeEventKind::Created,
            )
            .await;
            Ok(true)
        }
        Err(raisin_error::Error::Conflict(_)) => Ok(false),
        Err(e) => Err(other(e)),
    }
}

/// Make sure a folder exists (records' root, a member's inbox).
async fn ensure_folder<S: Storage>(
    storage: &S,
    tenant_id: &str,
    repo: &str,
    path: &str,
    node_type: &str,
) -> Result<(), FlowError> {
    if storage
        .nodes()
        .get_by_path(scope(tenant_id, repo), path, None)
        .await
        .map_err(other)?
        .is_some()
    {
        return Ok(());
    }
    let id = uuid::Uuid::new_v4().to_string();
    create_owned(storage, tenant_id, repo, &id, path, node_type, &json!({})).await?;
    Ok(())
}

/// Load a node of the access-control workspace by path.
pub(crate) async fn load<S: Storage>(
    storage: &S,
    tenant_id: &str,
    repo: &str,
    path: &str,
) -> Result<Option<Node>, FlowError> {
    storage
        .nodes()
        .get_by_path(scope(tenant_id, repo), path, None)
        .await
        .map_err(other)
}

/// The display name of a person by home path (for "Anna B. brings it"), if they have one.
pub(crate) async fn display_name<S: Storage>(
    storage: &S,
    tenant_id: &str,
    repo: &str,
    home: &str,
) -> Option<String> {
    let node = load(storage, tenant_id, repo, home).await.ok().flatten()?;
    prop_str(&node, "display_name").filter(|n| !n.trim().is_empty())
}

// ---------------------------------------------------------------------------
// Resolving a group
// ---------------------------------------------------------------------------

fn group_membership_matches(value: &str, group_path: &str, group_id: &str) -> bool {
    let normalized = value.trim_matches('/');
    normalized == group_path.trim_matches('/')
        || normalized == group_id
        || normalized.strip_prefix("groups/") == Some(group_id)
}

/// Resolve `reference` as a group: a circle (`raisin:EntityCircle`, by path or `circle:<node id>`)
/// or a permission group (`raisin:Group`, by path). `Ok(None)` when it names neither (a user, an
/// agent): the caller assigns a single task then.
///
/// A circle's members are the `raisin:User` nodes with a `MEMBER_OF` edge to it. Only admins and
/// the system may relate, so the roster is the graph's, never something a member wrote.
pub async fn resolve_group<S: Storage>(
    storage: &S,
    tenant_id: &str,
    repo: &str,
    reference: &str,
) -> Result<Option<GroupMembers>, FlowError> {
    let reference = reference.trim();
    let node = if let Some(id) = reference.strip_prefix("circle:") {
        storage
            .nodes()
            .get(scope(tenant_id, repo), id, None)
            .await
            .map_err(other)?
    } else if reference.starts_with('/') {
        load(storage, tenant_id, repo, reference).await?
    } else {
        None
    };
    let Some(node) = node else {
        return Ok(None);
    };

    let mut members = match node.node_type.as_str() {
        "raisin:EntityCircle" => {
            let incoming = storage
                .relations()
                .get_incoming_relations(scope(tenant_id, repo), &node.id, None)
                .await
                .map_err(other)?;
            let mut homes = Vec::new();
            for (source_ws, source_id, relation) in incoming {
                if source_ws != WORKSPACE || !relation.relation_type.eq_ignore_ascii_case(MEMBER_OF)
                {
                    continue;
                }
                if let Some(user) = storage
                    .nodes()
                    .get(scope(tenant_id, repo), &source_id, None)
                    .await
                    .map_err(other)?
                {
                    if user.node_type == "raisin:User" {
                        homes.push(user.path);
                    }
                }
            }
            homes
        }
        "raisin:Group" => {
            let group_id = prop_str(&node, "group_id")
                .or_else(|| node.path.rsplit('/').next().map(String::from))
                .unwrap_or_default();
            let users = storage
                .nodes()
                .list_by_type(
                    scope(tenant_id, repo),
                    "raisin:User",
                    ListOptions::default(),
                )
                .await
                .map_err(other)?;
            users
                .into_iter()
                .filter(|user| {
                    matches!(user.properties.get("groups"), Some(PropertyValue::Array(values))
                        if values.iter().any(|v| matches!(v, PropertyValue::String(s)
                            if group_membership_matches(s, &node.path, &group_id))))
                })
                .map(|user| user.path)
                .collect()
        }
        _ => return Ok(None),
    };
    members.sort();
    members.dedup();
    Ok(Some(GroupMembers {
        group_path: node.path,
        members,
    }))
}

// ---------------------------------------------------------------------------
// Creating an assignment
// ---------------------------------------------------------------------------

/// What to assign.
#[derive(Debug, Clone)]
pub struct NewAssignment {
    /// Stable id when the creation can be redelivered (a trigger retry): the same id finds the
    /// record already written and returns it instead of assigning twice. Generated when absent.
    pub assignment_id: Option<String>,
    pub group: GroupMembers,
    pub policy: Policy,
    /// The task every member receives (task_type, title, description, options, input_schema,
    /// data, priority, due_at, conversation_id, attachments, assigned_by, …).
    pub task: Value,
    pub flow_instance_id: Option<String>,
    pub step_id: Option<String>,
    /// `{ key, display_name }` of the person who assigned it.
    pub assigned_by: Option<Value>,
}

/// What was assigned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedAssignment {
    pub assignment_id: String,
    pub task_paths: Vec<String>,
    /// False when the record already existed (redelivery): nothing was written.
    pub created: bool,
}

/// Write the record, then one task into every member's inbox. A member task is
/// `<home>/inbox/<assignment id>`. A failure part-way cancels what was delivered and closes the
/// record unsatisfied, so no half-delivered group task sits in anybody's inbox.
pub async fn create_assignment<S: Storage>(
    storage: &S,
    tenant_id: &str,
    repo: &str,
    new: NewAssignment,
) -> Result<CreatedAssignment, FlowError> {
    let members = new.group.members.clone();
    if members.is_empty() {
        return Err(FlowError::InvalidNodeConfiguration(format!(
            "'{}' has no members",
            new.group.group_path
        )));
    }
    if members.len() > MAX_MEMBERS {
        return Err(FlowError::InvalidNodeConfiguration(format!(
            "'{}' has {} members; a group task reaches at most {}",
            new.group.group_path,
            members.len(),
            MAX_MEMBERS
        )));
    }
    let assignment_id = match new.assignment_id {
        Some(id) if valid_assignment_id(&id) => id,
        Some(id) => {
            return Err(FlowError::InvalidNodeConfiguration(format!(
                "invalid assignment id '{}'",
                id
            )))
        }
        None => format!("ga-{}", uuid::Uuid::new_v4().simple()),
    };
    let path = record_path(&assignment_id);

    // A redelivery of the same creation: return what the first one wrote.
    if let Some(existing) = load(storage, tenant_id, repo, &path).await? {
        if existing.node_type != RECORD_TYPE {
            return Err(FlowError::InvalidNodeConfiguration(format!(
                "'{}' is not a task assignment",
                path
            )));
        }
        return Ok(CreatedAssignment {
            assignment_id,
            task_paths: prop_strings(&existing, "task_paths"),
            created: false,
        });
    }

    let task_paths: Vec<String> = members
        .iter()
        .map(|home| format!("{}/inbox/{}", home.trim_end_matches('/'), assignment_id))
        .collect();

    ensure_folder(
        storage,
        tenant_id,
        repo,
        ASSIGNMENTS_ROOT,
        "raisin:AclFolder",
    )
    .await?;
    let record = record_properties(
        &new.group.group_path,
        &new.policy,
        &task_paths,
        new.flow_instance_id.as_deref(),
        new.step_id.as_deref(),
        new.assigned_by.as_ref(),
    );
    if !create_owned(
        storage,
        tenant_id,
        repo,
        &assignment_id,
        &path,
        RECORD_TYPE,
        &record,
    )
    .await?
    {
        // Lost a race with an identical redelivery: it owns the delivery.
        let existing = load(storage, tenant_id, repo, &path).await?;
        return Ok(CreatedAssignment {
            assignment_id,
            task_paths: existing
                .map(|n| prop_strings(&n, "task_paths"))
                .unwrap_or_default(),
            created: false,
        });
    }

    let mut base = new.task.clone();
    if let (Some(by), Some(map)) = (new.assigned_by.as_ref(), base.as_object_mut()) {
        map.entry("assigned_by".to_string())
            .or_insert_with(|| by.clone());
    }
    if let (Some(flow), Some(map)) = (new.flow_instance_id.as_ref(), base.as_object_mut()) {
        map.insert("flow_instance_id".into(), json!(flow));
    }
    if let (Some(step), Some(map)) = (new.step_id.as_ref(), base.as_object_mut()) {
        map.insert("step_id".into(), json!(step));
    }

    let mut delivered: Vec<String> = Vec::new();
    for (home, task_path) in members.iter().zip(task_paths.iter()) {
        let props = member_task_properties(
            &base,
            home,
            &assignment_id,
            &new.group.group_path,
            &new.policy,
            members.len(),
        );
        let result = async {
            ensure_folder(
                storage,
                tenant_id,
                repo,
                &format!("{}/inbox", home.trim_end_matches('/')),
                "raisin:MessageFolder",
            )
            .await?;
            let id = format!("{}-{}", assignment_id, &response_name(task_path)[2..]);
            create_owned(storage, tenant_id, repo, &id, task_path, TASK_TYPE, &props).await
        }
        .await;
        match result {
            Ok(_) => delivered.push(task_path.clone()),
            Err(cause) => {
                for path in &delivered {
                    let _ = set_task_fields(
                        storage,
                        tenant_id,
                        repo,
                        path,
                        &[
                            ("status", json!("cancelled")),
                            ("cancellation_reason", json!("group_delivery_failed")),
                            ("assignment_status", json!("unsatisfied")),
                        ],
                    )
                    .await;
                }
                let _ = set_fields(
                    storage,
                    tenant_id,
                    repo,
                    &path,
                    &[("status", json!("unsatisfied"))],
                )
                .await;
                return Err(FlowError::FunctionExecution(format!(
                    "Failed to deliver the group task to '{}': {}",
                    home, cause
                )));
            }
        }
    }

    tracing::info!(
        assignment_id = %assignment_id,
        group = %new.group.group_path,
        members = members.len(),
        completion = %new.policy.completion,
        required = new.policy.required,
        "Group task assigned"
    );
    Ok(CreatedAssignment {
        assignment_id,
        task_paths,
        created: true,
    })
}

/// Update some properties of a node the engine owns (and announce it).
pub(crate) async fn set_fields<S: Storage>(
    storage: &S,
    tenant_id: &str,
    repo: &str,
    path: &str,
    fields: &[(&str, Value)],
) -> Result<Option<Node>, FlowError> {
    let Some(mut node) = load(storage, tenant_id, repo, path).await? else {
        return Ok(None);
    };
    for (key, value) in fields {
        if value.is_null() {
            node.properties.remove(*key);
        } else {
            let bag = property_bag(&json!({ *key: value }))?;
            node.properties.extend(bag);
        }
    }
    storage
        .nodes()
        .update(
            scope(tenant_id, repo),
            node.clone(),
            raisin_storage::UpdateNodeOptions {
                validate_schema: false,
                ..Default::default()
            },
        )
        .await
        .map_err(other)?;
    publish(
        storage,
        tenant_id,
        repo,
        &node,
        raisin_storage::NodeEventKind::Updated,
    )
    .await;
    Ok(Some(node))
}

/// [`set_fields`] for a member task, refusing anything that is not an inbox task.
pub(crate) async fn set_task_fields<S: Storage>(
    storage: &S,
    tenant_id: &str,
    repo: &str,
    path: &str,
    fields: &[(&str, Value)],
) -> Result<Option<Node>, FlowError> {
    match load(storage, tenant_id, repo, path).await? {
        Some(node) if node.node_type == TASK_TYPE => {
            set_fields(storage, tenant_id, repo, path, fields).await
        }
        _ => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Answers
// ---------------------------------------------------------------------------

/// One recorded answer.
#[derive(Debug, Clone)]
pub struct RecordedResponse {
    pub task_path: String,
    pub assignee: Option<String>,
    pub completed_by: Option<String>,
    pub completed_by_name: Option<String>,
    pub response: Value,
    pub qualifies: bool,
    pub responded_at: Option<String>,
}

/// The answers recorded under a record, oldest first.
pub async fn responses<S: Storage>(
    storage: &S,
    tenant_id: &str,
    repo: &str,
    record_path: &str,
) -> Result<Vec<RecordedResponse>, FlowError> {
    let children = storage
        .nodes()
        .list_children(scope(tenant_id, repo), record_path, ListOptions::default())
        .await
        .map_err(other)?;
    let mut out: Vec<(Option<chrono::DateTime<chrono::Utc>>, RecordedResponse)> = children
        .into_iter()
        .filter(|n| {
            n.node_type == RESPONSE_TYPE && prop_str(n, "kind").as_deref() == Some("response")
        })
        .map(|n| {
            let responded_at = match n.properties.get("responded_at") {
                Some(PropertyValue::String(s)) => Some(s.clone()),
                Some(PropertyValue::Date(d)) => Some(d.to_rfc3339()),
                _ => None,
            };
            (
                n.created_at,
                RecordedResponse {
                    task_path: prop_str(&n, "task_path").unwrap_or_default(),
                    assignee: prop_str(&n, "assignee"),
                    completed_by: prop_str(&n, "completed_by"),
                    completed_by_name: prop_str(&n, "completed_by_name"),
                    response: n
                        .properties
                        .get("response")
                        .and_then(|v| serde_json::to_value(v).ok())
                        .unwrap_or(Value::Null),
                    qualifies: matches!(
                        n.properties.get("qualifies"),
                        Some(PropertyValue::Boolean(true))
                    ),
                    responded_at,
                },
            )
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.responded_at.cmp(&b.1.responded_at)));
    let mut seen = HashSet::new();
    Ok(out
        .into_iter()
        .map(|(_, r)| r)
        .filter(|r| seen.insert(r.task_path.clone()))
        .collect())
}

/// Record one member's answer. `Ok(false)` when this task has already answered.
#[allow(clippy::too_many_arguments)]
pub async fn record_response<S: Storage>(
    storage: &S,
    tenant_id: &str,
    repo: &str,
    assignment_id: &str,
    task_path: &str,
    assignee: Option<&str>,
    completed_by: &str,
    completed_by_name: Option<&str>,
    response: &Value,
    qualifies: bool,
) -> Result<bool, FlowError> {
    let name = response_name(task_path);
    let mut props = json!({
        "kind": "response",
        "task_path": task_path,
        "completed_by": completed_by,
        "response": if response.is_object() { response.clone() } else { json!({ "value": response }) },
        "qualifies": qualifies,
        "responded_at": chrono::Utc::now().to_rfc3339(),
    });
    if let Some(a) = assignee {
        props["assignee"] = json!(a);
    }
    if let Some(n) = completed_by_name {
        props["completed_by_name"] = json!(n);
    }
    create_owned(
        storage,
        tenant_id,
        repo,
        &format!("{}-{}", assignment_id, name),
        &format!("{}/{}", record_path(assignment_id), name),
        RESPONSE_TYPE,
        &props,
    )
    .await
}

/// Close a record. `Ok(false)` when another completion closed it first.
pub async fn close<S: Storage>(
    storage: &S,
    tenant_id: &str,
    repo: &str,
    assignment_id: &str,
    satisfied: bool,
) -> Result<bool, FlowError> {
    let outcome = if satisfied {
        "satisfied"
    } else {
        "unsatisfied"
    };
    let won = create_owned(
        storage,
        tenant_id,
        repo,
        &format!("{}-{}", assignment_id, CLOSED),
        &format!("{}/{}", record_path(assignment_id), CLOSED),
        RESPONSE_TYPE,
        &json!({ "kind": "close", "outcome": outcome }),
    )
    .await?;
    if won {
        set_fields(
            storage,
            tenant_id,
            repo,
            &record_path(assignment_id),
            &[
                ("status", json!(outcome)),
                ("closed_at", json!(chrono::Utc::now().to_rfc3339())),
            ],
        )
        .await?;
    }
    Ok(won)
}

/// Convert a JSON object to `Map` (helper for callers building task content).
pub fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}
