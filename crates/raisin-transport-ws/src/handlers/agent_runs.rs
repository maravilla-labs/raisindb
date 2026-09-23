// SPDX-License-Identifier: BSL-1.1

//! Durable agent runs over the WebSocket connection.
//!
//! The same transport-neutral API the HTTP routes and the function bindings
//! speak (`raisin_agent_runtime::api`), on the socket a client already holds:
//!
//! ```text
//! agent_run_create      CreateRunRequest                      → {run_id, created, status}
//! agent_run_get         {run_id}                              → RunView
//! agent_run_by_subject  {workspace, path?, node_id?, limit?}  → [RunView], newest (live) first
//! agent_run_list        {status?, limit?}                     → [RunView]
//! agent_run_events      {run_id, after_seq?, limit?}          → [RunEvent]
//! agent_run_subscribe   {run_id, after_seq?}                  → {subscription_id}
//!                        then events `agent_run_event` (a RunEvent) … `agent_run_end`
//! agent_run_unsubscribe {subscription_id}                     → {success}
//! agent_run_control     {run_id, control_id, command, …}      → ControlAck
//! agent_run_children    {run_id}                              → [ChildView]
//! ```
//!
//! A subscription replays from `after_seq` and then follows the durable log,
//! so a reconnecting client resumes exactly where it stopped. Scope is the
//! request context (tenant, repository, branch); the caller is the
//! connection's identity, never the payload's.

use std::sync::{Arc, OnceLock};

use dashmap::DashMap;
use parking_lot::RwLock;
use raisin_agent_runtime::api::{self, Caller, FollowItem};
use raisin_agent_runtime::host::AgentRunHost;
use raisin_agent_runtime::ids::{RunId, RunScope, SubjectRef};
use raisin_agent_runtime::state::RunStatus;
use raisin_storage::scope::StorageScope;
use raisin_storage::{NodeRepository, Storage};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{EventMessage, RequestEnvelope, ResponseEnvelope},
};

type Resp = Result<Option<ResponseEnvelope>, WsError>;

/// Live subscriptions: id → the stop signal of its forwarder.
fn subscriptions() -> &'static DashMap<String, tokio::sync::oneshot::Sender<()>> {
    static SUBS: OnceLock<DashMap<String, tokio::sync::oneshot::Sender<()>>> = OnceLock::new();
    SUBS.get_or_init(DashMap::new)
}

fn host() -> Result<Arc<AgentRunHost>, WsError> {
    raisin_rocksdb::agent_runs::agent_run_host().ok_or_else(|| {
        WsError::InvalidRequest("the agent run runtime is not running on this server".into())
    })
}

/// The connection's identity; anonymous callers are refused.
fn caller(connection_state: &Arc<RwLock<ConnectionState>>) -> Result<Caller, WsError> {
    let conn = connection_state.read();
    match conn.auth_context() {
        Some(a) if a.is_system => Ok(Caller {
            id: "system".into(),
            admin: true,
        }),
        Some(a) if a.is_anonymous_principal() => Err(WsError::PermissionDenied),
        Some(a) => Ok(Caller {
            id: a.user_id.clone().unwrap_or_default(),
            admin: false,
        }),
        None => Err(WsError::NotAuthenticated),
    }
}

fn scope(request: &RequestEnvelope) -> Result<RunScope, WsError> {
    let repo = request
        .context
        .repository
        .as_deref()
        .ok_or_else(|| WsError::InvalidRequest("repository is required".into()))?;
    Ok(api::scope(
        &request.context.tenant_id,
        repo,
        request.context.branch.as_deref(),
    ))
}

fn run_id(payload: &Value) -> Result<RunId, WsError> {
    payload
        .get("run_id")
        .and_then(Value::as_str)
        .map(|s| RunId(s.to_string()))
        .ok_or_else(|| WsError::InvalidRequest("'run_id' is required".into()))
}

fn answer<T: serde::Serialize>(request_id: String, r: Result<T, api::ApiError>) -> Resp {
    Ok(Some(match r {
        Ok(v) => {
            ResponseEnvelope::success(request_id, serde_json::to_value(v).unwrap_or(Value::Null))
        }
        Err(e) => ResponseEnvelope::error(request_id, e.code.to_uppercase(), e.message),
    }))
}

#[derive(Deserialize)]
struct SubjectQuery {
    workspace: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    node_id: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

/// Both spellings of a subject: runs are keyed by node id when their creator
/// knew it, by path otherwise.
async fn subject_spellings<S: Storage>(
    storage: &S,
    scope: &RunScope,
    q: &SubjectQuery,
) -> Vec<SubjectRef> {
    let mut out = Vec::new();
    let path = q.path.clone().unwrap_or_default();
    let mut node_id = q.node_id.clone();
    if node_id.is_none() && !path.is_empty() {
        let s = StorageScope::new(
            &scope.tenant_id,
            &scope.repo_id,
            &scope.branch,
            &q.workspace,
        );
        node_id = storage
            .nodes()
            .get_by_path(s, &path, None)
            .await
            .ok()
            .flatten()
            .map(|n| n.id);
    }
    if let Some(id) = node_id {
        out.push(SubjectRef {
            workspace: q.workspace.clone(),
            path: path.clone(),
            node_id: Some(id),
        });
    }
    if !path.is_empty() {
        out.push(SubjectRef {
            workspace: q.workspace.clone(),
            path,
            node_id: None,
        });
    }
    out
}

/// Every `agent_run_*` request.
pub async fn handle_agent_run<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
    op: &str,
) -> Resp
where
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
    B: raisin_binary::BinaryStorage + 'static,
{
    let caller = caller(connection_state)?;
    let scope = scope(&request)?;
    let host = host()?;
    let id = request.request_id.clone();
    let p = request.payload.clone();
    let bad = |e: serde_json::Error| WsError::InvalidRequest(e.to_string());
    match op {
        "create" => {
            let mut p = p;
            if let Some(obj) = p.as_object_mut() {
                obj.entry("branch").or_insert_with(|| json!(scope.branch));
            }
            let req: api::CreateRunRequest = serde_json::from_value(p).map_err(bad)?;
            answer(
                id,
                api::create(&host, &scope.tenant_id, &scope.repo_id, &caller, req).await,
            )
        }
        "get" => answer(id, api::get(&host, &scope, &run_id(&p)?, &caller).await),
        "by_subject" => {
            let q: SubjectQuery = serde_json::from_value(p).map_err(bad)?;
            let subjects = subject_spellings(state.storage.as_ref(), &scope, &q).await;
            let limit = q.limit.unwrap_or(20).min(200);
            answer(
                id,
                api::by_subject(&host, &scope, &caller, &subjects, limit).await,
            )
        }
        "list" => {
            let status: RunStatus =
                serde_json::from_value(p.get("status").cloned().unwrap_or(json!("running")))
                    .map_err(bad)?;
            let limit = p
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(50)
                .min(500) as usize;
            answer(id, api::list(&host, &scope, &caller, status, limit).await)
        }
        "events" => {
            let after = p.get("after_seq").and_then(Value::as_u64).unwrap_or(0);
            let limit = p
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(500)
                .min(5_000) as usize;
            answer(
                id,
                api::events(&host, &scope, &run_id(&p)?, &caller, after, limit).await,
            )
        }
        "control" => {
            let run = run_id(&p)?;
            let req: api::ControlRequest = serde_json::from_value(p).map_err(bad)?;
            answer(id, api::control(&host, &scope, &run, &caller, req).await)
        }
        "children" => answer(
            id,
            raisin_agent_runtime::api_child::children(&host, &scope, &run_id(&p)?, &caller).await,
        ),
        "subscribe" => subscribe(host, scope, caller, connection_state, id, &p).await,
        "unsubscribe" => {
            let sub = p
                .get("subscription_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let stopped = subscriptions()
                .remove(sub)
                .map(|(_, tx)| tx.send(()).is_ok())
                .is_some();
            Ok(Some(ResponseEnvelope::success(
                id,
                json!({ "success": true, "stopped": stopped }),
            )))
        }
        other => Err(WsError::InvalidRequest(format!(
            "unknown agent run operation '{other}'"
        ))),
    }
}

async fn subscribe(
    host: Arc<AgentRunHost>,
    scope: RunScope,
    caller: Caller,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request_id: String,
    p: &Value,
) -> Resp {
    let run = run_id(p)?;
    let after = p.get("after_seq").and_then(Value::as_u64).unwrap_or(0);
    // Authorize before answering: a refusal is a response, not a stream.
    if let Err(e) = api::get(&host, &scope, &run, &caller).await {
        return answer::<()>(request_id, Err(e));
    }
    let sub_id = Uuid::new_v4().to_string();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    subscriptions().insert(sub_id.clone(), stop_tx);
    let conn = Arc::clone(connection_state);
    let sid = sub_id.clone();
    tokio::spawn(async move {
        let send = |item: FollowItem| {
            let (kind, data) = match &item {
                FollowItem::Event(ev) => ("agent_run_event", serde_json::to_value(ev)),
                FollowItem::End { .. } => ("agent_run_end", serde_json::to_value(&item)),
            };
            let msg = EventMessage::new(sid.clone(), kind.to_string(), data.unwrap_or(Value::Null));
            conn.read().send_event(msg).is_ok()
        };
        if let Err(e) = api::follow(&host, &scope, &run, &caller, after, stop_rx, send).await {
            tracing::debug!(run = %run, error = %e.message, "agent run subscription ended with an error");
        }
        subscriptions().remove(&sid);
    });
    Ok(Some(ResponseEnvelope::success(
        request_id,
        json!({ "subscription_id": sub_id, "run_id": p.get("run_id") }),
    )))
}
