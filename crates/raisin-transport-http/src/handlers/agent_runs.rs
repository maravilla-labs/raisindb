// SPDX-License-Identifier: BSL-1.1

//! Durable agent runs over HTTP.
//!
//! Thin adapters over `raisin_agent_runtime::api`: this layer authenticates,
//! maps the caller, and serializes; every rule (who may see or control a run,
//! lease fencing, idempotency) is core's. The same shapes are spoken by the
//! function bindings and the SDKs, so an external agent can drive a run with
//! nothing but this API.
//!
//! ```text
//! POST /api/agent-runs/{repo}                         create (or find) a run
//! GET  /api/agent-runs/{repo}?status=&branch=&limit=  list visible runs
//! GET  /api/agent-runs/{repo}?subject=ws:/path        every run of a subject,
//!                                                     newest (live) first
//! GET  /api/agent-runs/{repo}/{run}?branch=           read (with projection)
//! GET  /api/agent-runs/{repo}/{run}/events?after_seq= durable events
//! GET  /api/agent-runs/{repo}/{run}/stream?after_seq= SSE: replay, then live
//! POST /api/agent-runs/{repo}/{run}/control           any control command
//! POST /api/agent-runs/{repo}/{run}/{stop|pause|resume|steer|approve|answer}
//! POST /api/agent-runs/{repo}/{run}/lease/{acquire|renew|release}
//! POST /api/agent-runs/{repo}/{run}/operations        begin (client driver)
//! POST /api/agent-runs/{repo}/{run}/operations/finish finish (body: op_id)
//! POST /api/agent-runs/{repo}/{run}/{wait|complete}   client driver
//! ```

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use raisin_agent_runtime::api::{self, Caller};
use raisin_agent_runtime::control::{ApprovalDecision, ControlKind};
use raisin_agent_runtime::host::AgentRunHost;
use raisin_agent_runtime::ids::{RequestId, RunId, RunScope, SubjectRef};
use raisin_agent_runtime::state::RunStatus;
use raisin_models::auth::AuthContext;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::ApiError;
use crate::middleware::TenantInfo;
use crate::state::AppState;

/// `?branch=`.
#[derive(Debug, Deserialize, Default)]
pub struct BranchQuery {
    /// Branch (default `main`).
    pub branch: Option<String>,
    /// Events after this seq.
    pub after_seq: Option<u64>,
    /// Page size.
    pub limit: Option<usize>,
    /// Status filter (list).
    pub status: Option<String>,
    /// Subject filter (list): `"{workspace}:{path}"`.
    pub subject: Option<String>,
    /// The subject's node id, when the caller knows it.
    pub subject_node_id: Option<String>,
}

pub(crate) fn host() -> Result<std::sync::Arc<AgentRunHost>, ApiError> {
    raisin_rocksdb::agent_runs::agent_run_host().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "agent_runs_unavailable",
            "the agent run runtime is not running on this server",
        )
    })
}

pub(crate) fn map_err(e: api::ApiError) -> ApiError {
    let status = StatusCode::from_u16(e.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    ApiError::new(status, e.code, e.message)
}

/// The authenticated caller; anonymous callers are refused.
pub(crate) fn caller(auth: Option<Extension<AuthContext>>) -> Result<Caller, ApiError> {
    let Some(Extension(auth)) = auth else {
        return Err(ApiError::unauthorized("authentication required"));
    };
    if auth.is_system {
        return Ok(Caller {
            id: "system".into(),
            admin: true,
        });
    }
    if auth.is_anonymous_principal() {
        return Err(ApiError::unauthorized("authentication required"));
    }
    let id = auth.user_id.clone().unwrap_or_default();
    Ok(Caller { id, admin: false })
}

pub(crate) fn scope_of(tenant: &TenantInfo, repo: &str, q: &BranchQuery) -> RunScope {
    api::scope(&tenant.tenant_id, repo, q.branch.as_deref())
}

type Resp = Result<Json<Value>, ApiError>;

fn ok<T: serde::Serialize>(v: T) -> Resp {
    Ok(Json(serde_json::to_value(v).unwrap_or(Value::Null)))
}

/// Create (or find) a run.
pub async fn create_run(
    Extension(tenant): Extension<TenantInfo>,
    Path(repo): Path<String>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<api::CreateRunRequest>,
) -> Resp {
    let caller = caller(auth)?;
    ok(
        api::create(&*host()?, &tenant.tenant_id, &repo, &caller, req)
            .await
            .map_err(map_err)?,
    )
}

fn parse_status(s: Option<&str>) -> Result<RunStatus, ApiError> {
    let s = s.unwrap_or("running");
    serde_json::from_value(json!(s))
        .map_err(|_| ApiError::validation_failed(format!("unknown status '{s}'")))
}

/// List the caller's runs: of one subject (newest first), or in one status.
pub async fn list_runs(
    State(state): State<AppState>,
    Extension(tenant): Extension<TenantInfo>,
    Path(repo): Path<String>,
    Query(q): Query<BranchQuery>,
    auth: Option<Extension<AuthContext>>,
) -> Resp {
    let caller = caller(auth)?;
    let limit = q.limit.unwrap_or(50).min(500);
    let scope = scope_of(&tenant, &repo, &q);
    if let Some(subject) = q.subject.as_deref() {
        let subjects =
            subject_spellings(&state, &scope, subject, q.subject_node_id.clone()).await?;
        let mut runs = api::by_subject(&*host()?, &scope, &caller, &subjects, limit)
            .await
            .map_err(map_err)?;
        if let Some(s) = q.status.as_deref() {
            let status = parse_status(Some(s))?;
            runs.retain(|v| v.status == status);
        }
        return ok(runs);
    }
    let status = parse_status(q.status.as_deref())?;
    ok(api::list(&*host()?, &scope, &caller, status, limit)
        .await
        .map_err(map_err)?)
}

/// `"{workspace}:{path}"` in both spellings a run may be keyed by: its node id
/// (when the creator knew it) and its path.
async fn subject_spellings(
    state: &AppState,
    scope: &RunScope,
    subject: &str,
    node_id: Option<String>,
) -> Result<Vec<SubjectRef>, ApiError> {
    use raisin_storage::{NodeRepository, Storage};
    let (workspace, path) = subject
        .split_once(':')
        .filter(|(w, p)| !w.is_empty() && p.starts_with('/'))
        .ok_or_else(|| ApiError::validation_failed("subject must be '{workspace}:{path}'"))?;
    let node_id = match node_id {
        Some(id) => Some(id),
        None => {
            let s = raisin_storage::scope::StorageScope::new(
                &scope.tenant_id,
                &scope.repo_id,
                &scope.branch,
                workspace,
            );
            state
                .storage
                .nodes()
                .get_by_path(s, path, None)
                .await
                .ok()
                .flatten()
                .map(|n| n.id)
        }
    };
    let mut out = Vec::new();
    if let Some(id) = node_id {
        out.push(SubjectRef {
            workspace: workspace.into(),
            path: path.into(),
            node_id: Some(id),
        });
    }
    out.push(SubjectRef {
        workspace: workspace.into(),
        path: path.into(),
        node_id: None,
    });
    Ok(out)
}

/// Read a run.
pub async fn get_run(
    Extension(tenant): Extension<TenantInfo>,
    Path((repo, run)): Path<(String, String)>,
    Query(q): Query<BranchQuery>,
    auth: Option<Extension<AuthContext>>,
) -> Resp {
    let caller = caller(auth)?;
    let scope = scope_of(&tenant, &repo, &q);
    ok(api::get(&*host()?, &scope, &RunId(run), &caller)
        .await
        .map_err(map_err)?)
}

/// Durable events after `after_seq`.
pub async fn run_events(
    Extension(tenant): Extension<TenantInfo>,
    Path((repo, run)): Path<(String, String)>,
    Query(q): Query<BranchQuery>,
    auth: Option<Extension<AuthContext>>,
) -> Resp {
    let caller = caller(auth)?;
    let scope = scope_of(&tenant, &repo, &q);
    let limit = q.limit.unwrap_or(500).min(5_000);
    let events = api::events(
        &*host()?,
        &scope,
        &RunId(run),
        &caller,
        q.after_seq.unwrap_or(0),
        limit,
    )
    .await
    .map_err(map_err)?;
    ok(events)
}

/// Any control command.
pub async fn control_run(
    Extension(tenant): Extension<TenantInfo>,
    Path((repo, run)): Path<(String, String)>,
    Query(q): Query<BranchQuery>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<api::ControlRequest>,
) -> Resp {
    let caller = caller(auth)?;
    let scope = scope_of(&tenant, &repo, &q);
    ok(api::control(&*host()?, &scope, &RunId(run), &caller, req)
        .await
        .map_err(map_err)?)
}

/// Body of the named-action shortcuts.
#[derive(Debug, Deserialize)]
pub struct ActionBody {
    /// Idempotency key.
    pub control_id: String,
    /// Capability secret, for a non-principal controller.
    #[serde(default)]
    pub capability: Option<String>,
    /// stop: reason; approve: rejection reason.
    #[serde(default)]
    pub reason: Option<String>,
    /// steer: the input.
    #[serde(default)]
    pub input: Option<Value>,
    /// approve / answer: the request.
    #[serde(default)]
    pub request_id: Option<String>,
    /// approve: `approve` (default) or `reject`.
    #[serde(default)]
    pub decision: Option<String>,
    /// approve: the digest the approval is tied to.
    #[serde(default)]
    pub subject_digest: Option<String>,
    /// answer: the value.
    #[serde(default)]
    pub value: Option<Value>,
    /// resume: accept a changed reducer.
    #[serde(default)]
    pub accept_reducer_change: bool,
}

fn need<T>(v: Option<T>, field: &str) -> Result<T, ApiError> {
    v.ok_or_else(|| ApiError::missing_required_field(field))
}

fn action_command(action: &str, b: &ActionBody) -> Result<ControlKind, ApiError> {
    Ok(match action {
        "stop" => ControlKind::Stop {
            reason: b.reason.clone(),
        },
        "pause" => ControlKind::Pause,
        "resume" => ControlKind::Resume {
            budget_increase: None,
            accept_reducer_change: b.accept_reducer_change,
        },
        "steer" => ControlKind::Steer {
            input: need(b.input.clone(), "input")?,
        },
        "approve" => ControlKind::Approve {
            request_id: RequestId(need(b.request_id.clone(), "request_id")?),
            decision: match b.decision.as_deref() {
                Some("reject") => ApprovalDecision::Reject {
                    reason: b.reason.clone(),
                },
                _ => ApprovalDecision::Approve,
            },
            subject_digest: need(b.subject_digest.clone(), "subject_digest")?,
        },
        "answer" => ControlKind::ProvideInput {
            request_id: RequestId(need(b.request_id.clone(), "request_id")?),
            value: need(b.value.clone(), "value")?,
        },
        other => return Err(ApiError::not_found(format!("unknown run action '{other}'"))),
    })
}

/// `stop | pause | resume | steer | approve | answer | wait | complete`.
pub async fn run_action(
    Extension(tenant): Extension<TenantInfo>,
    Path((repo, run, action)): Path<(String, String, String)>,
    Query(q): Query<BranchQuery>,
    auth: Option<Extension<AuthContext>>,
    Json(body): Json<Value>,
) -> Resp {
    let caller = caller(auth)?;
    let scope = scope_of(&tenant, &repo, &q);
    let host = host()?;
    let run = RunId(run);
    let bad = |e: serde_json::Error| ApiError::validation_failed(e.to_string());
    match action.as_str() {
        "wait" => ok(api::wait(
            &host,
            &scope,
            &run,
            &caller,
            serde_json::from_value(body).map_err(bad)?,
        )
        .await
        .map_err(map_err)?),
        "complete" => ok(api::complete(
            &host,
            &scope,
            &run,
            &caller,
            serde_json::from_value(body).map_err(bad)?,
        )
        .await
        .map_err(map_err)?),
        _ => {
            let b: ActionBody = serde_json::from_value(body).map_err(bad)?;
            let req = api::ControlRequest {
                control_id: b.control_id.clone(),
                command: action_command(&action, &b)?,
                capability: b.capability.clone(),
            };
            ok(api::control(&host, &scope, &run, &caller, req)
                .await
                .map_err(map_err)?)
        }
    }
}

/// `acquire | renew | release` the lease (client-driven runs).
pub async fn run_lease(
    Extension(tenant): Extension<TenantInfo>,
    Path((repo, run, action)): Path<(String, String, String)>,
    Query(q): Query<BranchQuery>,
    auth: Option<Extension<AuthContext>>,
    body: Option<Json<Value>>,
) -> Resp {
    let caller = caller(auth)?;
    let scope = scope_of(&tenant, &repo, &q);
    let host = host()?;
    let run = RunId(run);
    let body = body.map(|Json(v)| v).unwrap_or(Value::Null);
    let fence = || -> Result<api::Fence, ApiError> {
        serde_json::from_value(body.get("fence").cloned().unwrap_or(Value::Null))
            .map_err(|_| ApiError::missing_required_field("fence"))
    };
    match action.as_str() {
        "acquire" => {
            let owner = format!("client:{}:{}", caller.id, uuid::Uuid::new_v4());
            ok(api::acquire(&host, &scope, &run, &caller, &owner)
                .await
                .map_err(map_err)?)
        }
        "renew" => ok(api::renew(&host, &scope, &run, &caller, &fence()?)
            .await
            .map_err(map_err)?),
        "release" => ok(api::release(&host, &scope, &run, &caller, &fence()?)
            .await
            .map_err(map_err)?),
        other => Err(ApiError::not_found(format!(
            "unknown lease action '{other}'"
        ))),
    }
}

/// Begin an operation (client-driven runs).
pub async fn begin_operation(
    Extension(tenant): Extension<TenantInfo>,
    Path((repo, run)): Path<(String, String)>,
    Query(q): Query<BranchQuery>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<api::BeginRequest>,
) -> Resp {
    let caller = caller(auth)?;
    let scope = scope_of(&tenant, &repo, &q);
    ok(api::begin(&*host()?, &scope, &RunId(run), &caller, req)
        .await
        .map_err(map_err)?)
}

/// `{ op_id, fence, outcome, payload?, tool_calls?, usage?, resume_key? }`.
#[derive(Debug, Deserialize)]
pub struct FinishBody {
    /// The operation.
    pub op_id: String,
    /// The result.
    #[serde(flatten)]
    pub result: api::FinishRequest,
}

/// Finish an operation (client-driven runs).
pub async fn finish_operation(
    Extension(tenant): Extension<TenantInfo>,
    Path((repo, run)): Path<(String, String)>,
    Query(q): Query<BranchQuery>,
    auth: Option<Extension<AuthContext>>,
    Json(body): Json<FinishBody>,
) -> Resp {
    let caller = caller(auth)?;
    let scope = scope_of(&tenant, &repo, &q);
    ok(api::finish(
        &*host()?,
        &scope,
        &RunId(run),
        &caller,
        &body.op_id,
        body.result,
    )
    .await
    .map_err(map_err)?)
}
