// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The public run API, independent of transport.
//!
//! HTTP, the function bindings (`raisin.agentRuns.*`, any language) and every
//! SDK speak these request/response shapes, so a run behaves the same whether
//! Studio's Builder drives it, a function drives it, or an external coding
//! agent drives it over the network. The transport only authenticates and
//! maps the caller onto [`Caller`]; every rule below is core's.
//!
//! Two ways to drive a run:
//! - **server-driven**: create it with a `reducer` (any function); the node's
//!   job queue drives it;
//! - **client-driven**: create it without one and drive it with the lease
//!   calls ([`acquire`], [`begin`], [`finish`], ...). The run is still
//!   durable, stoppable and steerable by anyone authorized, and its operations
//!   still fence on the lease epoch.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::control::{ActorRef, ControlAck, ControlCommand, ControlKind};
use crate::events::RunEvent;
use crate::host::AgentRunHost;
use crate::ids::{ControlId, Principal, PrincipalKind, RunId, RunScope, Seq, SubjectRef};
use crate::record::{AgentRunRecord, RunBudgets};
use crate::service::{CreateRun, ServiceError};
use crate::state::RunStatus;
use crate::store::CreateOutcome;

/// The authenticated caller, as the transport established it.
#[derive(Debug, Clone)]
pub struct Caller {
    /// Identity id (a user id).
    pub id: String,
    /// A system/administrative caller (sees and controls every run).
    pub admin: bool,
}

/// An API failure, with the HTTP status a transport should use.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ApiError {
    /// HTTP status.
    pub status: u16,
    /// Stable code.
    pub code: String,
    /// Message.
    pub message: String,
}

impl ApiError {
    pub(crate) fn new(status: u16, code: &str, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
        }
    }
    fn forbidden() -> Self {
        Self::new(403, "forbidden", "not your run")
    }
}

impl From<ServiceError> for ApiError {
    fn from(e: ServiceError) -> Self {
        match &e {
            ServiceError::NotFound => Self::new(404, "not_found", "run not found"),
            ServiceError::Unauthorized => Self::new(403, "unauthorized", e.to_string()),
            ServiceError::Invalid(_) => Self::new(400, "invalid", e.to_string()),
            ServiceError::Refused(r) => Self::new(409, &r.code, &r.message),
            ServiceError::Begin(_) => Self::new(409, "begin_refused", e.to_string()),
            ServiceError::Store(s) if e.is_lease_lost() => Self::new(409, s.code(), e.to_string()),
            ServiceError::Store(s) => Self::new(500, s.code(), e.to_string()),
        }
    }
}

pub(crate) type R<T> = Result<T, ApiError>;

/// `{ function_path, handler? }`.
#[derive(Debug, Clone, Deserialize)]
pub struct ReducerSpec {
    /// Function path of the reducer (any language).
    pub function_path: String,
    /// Handler / entrypoint, when the function node does not select one.
    #[serde(default)]
    pub handler: Option<String>,
}

fn main_branch() -> String {
    "main".into()
}

/// Create a run.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateRunRequest {
    /// Branch the run executes on.
    #[serde(default = "main_branch")]
    pub branch: String,
    /// What the run is about (one live run per subject).
    pub subject: SubjectRef,
    /// Opaque agent reference (e.g. an agent node path).
    #[serde(default)]
    pub agent_ref: Option<String>,
    /// Idempotency key of the create.
    #[serde(default)]
    pub create_key: Option<String>,
    /// Budgets.
    #[serde(default)]
    pub budgets: RunBudgets,
    /// Input, delivered as `run_started.data.input`.
    #[serde(default)]
    pub input: Value,
    /// Server-driven: the reducer function. Omitted: client-driven.
    #[serde(default)]
    pub reducer: Option<ReducerSpec>,
    /// Opaque executor configuration (e.g. `model_turn_function`).
    #[serde(default)]
    pub executor_config: Option<Value>,
    /// A secret whose holder may control the run (stored hashed).
    #[serde(default)]
    pub control_capability: Option<String>,
    /// Run as this agent (`"ws:/path"` or `"/path"`), on behalf of the caller.
    #[serde(default)]
    pub as_agent: Option<String>,
    /// The user the run acts for. Honoured ONLY for an admin (system) caller —
    /// a trigger creating a run for the user whose message started it; any
    /// other caller always acts for itself.
    #[serde(default)]
    pub on_behalf_of: Option<String>,
    /// What waits for the run's end outside its tree (a flow step). System
    /// callers only.
    #[serde(default)]
    pub waiter: Option<crate::waiter::RunWaiter>,
}

/// The user a create acts for: an admin caller may name one, nobody else can.
fn acting_user(caller: &Caller, requested: Option<&str>) -> String {
    match requested {
        Some(user) if caller.admin && !user.trim().is_empty() => user.trim().to_string(),
        _ => caller.id.clone(),
    }
}

/// The answer to a create.
#[derive(Debug, Clone, Serialize)]
pub struct CreateRunResponse {
    /// The run.
    pub run_id: RunId,
    /// False when the subject's live run (or the create key's run) came back.
    pub created: bool,
    /// Its status.
    pub status: RunStatus,
}

/// A run as a reader sees it.
#[derive(Debug, Clone, Serialize)]
pub struct RunView {
    /// The record (the capability hash removed).
    pub run: AgentRunRecord,
    /// Flat status.
    pub status: RunStatus,
    /// The projection to display, overridden by the run status.
    pub projection: Option<raisin_agent_contract::PlanProjection>,
}

fn scope_of(tenant: &str, repo: &str, branch: &str) -> RunScope {
    RunScope::new(tenant, repo, branch)
}

pub(crate) fn may_access(rec: &AgentRunRecord, caller: &Caller) -> bool {
    caller.admin
        || (rec.principal.kind == PrincipalKind::User && rec.principal.id == caller.id)
        || rec.principal.on_behalf_of.as_deref() == Some(caller.id.as_str())
}

pub(crate) async fn load_for(
    host: &AgentRunHost,
    scope: &RunScope,
    run: &RunId,
    caller: &Caller,
) -> R<AgentRunRecord> {
    let rec = host
        .service()
        .get(scope, run)
        .await?
        .ok_or_else(|| ApiError::new(404, "not_found", "run not found"))?;
    if !may_access(&rec, caller) {
        return Err(ApiError::forbidden());
    }
    Ok(rec)
}

/// Create (or find) a run.
pub async fn create(
    host: &AgentRunHost,
    tenant: &str,
    repo: &str,
    caller: &Caller,
    req: CreateRunRequest,
) -> R<CreateRunResponse> {
    let scope = scope_of(tenant, repo, &req.branch);
    let user = acting_user(caller, req.on_behalf_of.as_deref());
    let principal = match &req.as_agent {
        Some(agent) => Principal {
            kind: PrincipalKind::Agent,
            id: agent.clone(),
            on_behalf_of: Some(user),
        },
        None => Principal::user(&user),
    };
    // Only a system caller may point a run's result at a flow instance.
    let waiter = match req.waiter {
        Some(_) if !caller.admin => {
            return Err(ApiError::new(
                403,
                "forbidden",
                "only a system caller may register a waiter",
            ))
        }
        // It waits on the run's own branch.
        w => w.map(|w| crate::waiter::RunWaiter {
            delivered: false,
            branch: req.branch.clone(),
            ..w
        }),
    };
    let reducer = match &req.reducer {
        Some(spec) => Some(
            host.reducers()
                .bind(
                    &scope,
                    &spec.function_path,
                    spec.handler.as_deref().unwrap_or(""),
                )
                .await
                .map_err(|e| ApiError::new(422, "reducer_unavailable", e.to_string()))?,
        ),
        None => None,
    };
    let outcome = host
        .service()
        .create(
            CreateRun {
                scope,
                subject: req.subject,
                principal,
                control_capability: req.control_capability,
                agent_ref: req.agent_ref,
                create_key: req.create_key,
                budgets: req.budgets,
                input: req.input,
                reducer,
                executor_config: req.executor_config,
                waiter,
            },
            None,
        )
        .await?;
    Ok(match outcome {
        CreateOutcome::Created { run_id, .. } => CreateRunResponse {
            run_id,
            created: true,
            status: RunStatus::Queued,
        },
        CreateOutcome::Existing { run_id, status } => CreateRunResponse {
            run_id,
            created: false,
            status,
        },
    })
}

/// Read a run.
pub async fn get(
    host: &AgentRunHost,
    scope: &RunScope,
    run: &RunId,
    caller: &Caller,
) -> R<RunView> {
    let mut rec = load_for(host, scope, run, caller).await?;
    let projection = host.service().effective_projection(scope, run).await?;
    rec.control_capability_hash = None;
    Ok(RunView {
        status: rec.state.status(),
        run: rec,
        projection,
    })
}

/// Runs of a scope in `status` that the caller may see.
pub async fn list(
    host: &AgentRunHost,
    scope: &RunScope,
    caller: &Caller,
    status: RunStatus,
    limit: usize,
) -> R<Vec<RunView>> {
    let mut out = Vec::new();
    let store = host.service().store();
    for id in store
        .scan_status(scope, status, limit.saturating_mul(4).max(limit))
        .await
        .map_err(ServiceError::from)?
    {
        if let Some(mut rec) = store.load(scope, &id).await.map_err(ServiceError::from)? {
            if may_access(&rec, caller) {
                rec.control_capability_hash = None;
                out.push(RunView {
                    status: rec.state.status(),
                    run: rec,
                    projection: None,
                });
                if out.len() >= limit {
                    break;
                }
            }
        }
    }
    Ok(out)
}

/// Durable events after `after_seq` (gap-free, replay-safe).
pub async fn events(
    host: &AgentRunHost,
    scope: &RunScope,
    run: &RunId,
    caller: &Caller,
    after_seq: u64,
    limit: usize,
) -> R<Vec<RunEvent>> {
    load_for(host, scope, run, caller).await?;
    Ok(host
        .service()
        .read_events(scope, run, Seq(after_seq), limit)
        .await?)
}

/// A control command.
#[derive(Debug, Clone, Deserialize)]
pub struct ControlRequest {
    /// Idempotency key (a retry with the same payload is a no-op).
    pub control_id: String,
    /// The command (`{"command":"stop"}`, `{"command":"steer","input":…}`, …).
    pub command: ControlKind,
    /// A control capability, for a caller that is not the principal.
    #[serde(default)]
    pub capability: Option<String>,
}

/// Submit a control. Authorization is core's (`control::authorize`).
pub async fn control(
    host: &AgentRunHost,
    scope: &RunScope,
    run: &RunId,
    caller: &Caller,
    req: ControlRequest,
) -> R<ControlAck> {
    let cmd = ControlCommand {
        control_id: ControlId(req.control_id),
        kind: req.command,
        issued_by: ActorRef {
            kind: PrincipalKind::User,
            id: caller.id.clone(),
            capability: req.capability,
        },
        at_ms: host.service().now(),
    };
    let system = caller.admin.then(crate::ids::SystemToken::in_process);
    let cmd = if caller.admin {
        ControlCommand {
            issued_by: ActorRef {
                kind: PrincipalKind::System,
                ..cmd.issued_by
            },
            ..cmd
        }
    } else {
        cmd
    };
    Ok(host
        .service()
        .submit_control(scope, run, cmd, system.as_ref())
        .await?)
}

pub use crate::api_driver::*;
pub use crate::api_follow::*;

/// Scope helper for transports.
pub fn scope(tenant: &str, repo: &str, branch: Option<&str>) -> RunScope {
    scope_of(tenant, repo, branch.unwrap_or("main"))
}
