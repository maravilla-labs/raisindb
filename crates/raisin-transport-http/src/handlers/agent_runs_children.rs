// SPDX-License-Identifier: BSL-1.1

//! Child runs, mailboxes, checkpoints and usage over HTTP.
//!
//! Thin adapters over `raisin_agent_runtime::api_child`; every rule is core's.
//!
//! ```text
//! POST /api/agent-runs/{repo}/{run}/children                     spawn a child
//! GET  /api/agent-runs/{repo}/{run}/children                     list children
//! GET  /api/agent-runs/{repo}/{run}/children/{child}?after_seq=  inspect one
//! POST /api/agent-runs/{repo}/{run}/children/{child}/control     {control_id, action, …}
//! POST /api/agent-runs/{repo}/{run}/children/{child}/wait        {fence} (client driver)
//! GET  /api/agent-runs/{repo}/{run}/mailbox                      unacknowledged items
//! POST /api/agent-runs/{repo}/{run}/mailbox/ack                  {up_to}
//! POST /api/agent-runs/{repo}/{run}/post-to-parent               {message_id, message}
//! POST /api/agent-runs/{repo}/{run}/checkpoints                  write (compaction)
//! GET  /api/agent-runs/{repo}/{run}/checkpoints/{n|latest}       read, with state
//! GET  /api/agent-runs/{repo}/{run}/usage                        usage accounting
//! ```

use axum::extract::{Path, Query};
use axum::{Extension, Json};
use raisin_agent_runtime::api_child as child;
use raisin_agent_runtime::ids::RunId;
use raisin_models::auth::AuthContext;
use serde::Deserialize;
use serde_json::Value;

use super::agent_runs::{caller, host, map_err, scope_of, BranchQuery};
use crate::error::ApiError;
use crate::middleware::TenantInfo;

type Resp = Result<Json<Value>, ApiError>;

fn ok<T: serde::Serialize>(v: T) -> Resp {
    Ok(Json(serde_json::to_value(v).unwrap_or(Value::Null)))
}

/// Spawn a child of `run`.
pub async fn spawn_child(
    Extension(tenant): Extension<TenantInfo>,
    Path((repo, run)): Path<(String, String)>,
    Query(q): Query<BranchQuery>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<child::SpawnRequest>,
) -> Resp {
    let caller = caller(auth)?;
    let scope = scope_of(&tenant, &repo, &q);
    ok(child::spawn(&*host()?, &scope, &RunId(run), &caller, req)
        .await
        .map_err(map_err)?)
}

/// List the children of `run`.
pub async fn list_children(
    Extension(tenant): Extension<TenantInfo>,
    Path((repo, run)): Path<(String, String)>,
    Query(q): Query<BranchQuery>,
    auth: Option<Extension<AuthContext>>,
) -> Resp {
    let caller = caller(auth)?;
    let scope = scope_of(&tenant, &repo, &q);
    ok(child::children(&*host()?, &scope, &RunId(run), &caller)
        .await
        .map_err(map_err)?)
}

/// Inspect one child: record, events after `after_seq`, usage, checkpoint.
pub async fn inspect_child(
    Extension(tenant): Extension<TenantInfo>,
    Path((repo, run, c)): Path<(String, String, String)>,
    Query(q): Query<BranchQuery>,
    auth: Option<Extension<AuthContext>>,
) -> Resp {
    let caller = caller(auth)?;
    let scope = scope_of(&tenant, &repo, &q);
    let limit = q.limit.unwrap_or(200).min(5_000);
    let after = q.after_seq.unwrap_or(0);
    ok(child::inspect(
        &*host()?,
        &scope,
        &RunId(run),
        &RunId(c),
        &caller,
        after,
        limit,
    )
    .await
    .map_err(map_err)?)
}

/// `control` (message / steer / interrupt / resume) or `wait` on a child.
pub async fn child_action(
    Extension(tenant): Extension<TenantInfo>,
    Path((repo, run, c, action)): Path<(String, String, String, String)>,
    Query(q): Query<BranchQuery>,
    auth: Option<Extension<AuthContext>>,
    Json(body): Json<Value>,
) -> Resp {
    let caller = caller(auth)?;
    let scope = scope_of(&tenant, &repo, &q);
    let host = host()?;
    let (parent, c) = (RunId(run), RunId(c));
    let parse = |e: serde_json::Error| ApiError::validation_failed(e.to_string());
    match action.as_str() {
        "control" => {
            let req: child::ChildControlRequest = serde_json::from_value(body).map_err(parse)?;
            ok(
                child::control_child(&host, &scope, &parent, &c, &caller, req)
                    .await
                    .map_err(map_err)?,
            )
        }
        "wait" => {
            let req: child::WaitChildRequest = serde_json::from_value(body).map_err(parse)?;
            ok(child::wait_child(&host, &scope, &parent, &c, &caller, req)
                .await
                .map_err(map_err)?)
        }
        other => Err(ApiError::validation_failed(format!(
            "unknown child action '{other}'"
        ))),
    }
}

/// The unacknowledged mailbox.
pub async fn mailbox(
    Extension(tenant): Extension<TenantInfo>,
    Path((repo, run)): Path<(String, String)>,
    Query(q): Query<BranchQuery>,
    auth: Option<Extension<AuthContext>>,
) -> Resp {
    let caller = caller(auth)?;
    let scope = scope_of(&tenant, &repo, &q);
    ok(child::mailbox(&*host()?, &scope, &RunId(run), &caller)
        .await
        .map_err(map_err)?)
}

/// `{ up_to }`.
#[derive(Debug, Deserialize)]
pub struct AckBody {
    /// Acknowledge items with `mail_no <= up_to`.
    pub up_to: u64,
}

/// Acknowledge mailbox items.
pub async fn ack_mailbox(
    Extension(tenant): Extension<TenantInfo>,
    Path((repo, run)): Path<(String, String)>,
    Query(q): Query<BranchQuery>,
    auth: Option<Extension<AuthContext>>,
    Json(body): Json<AckBody>,
) -> Resp {
    let caller = caller(auth)?;
    let scope = scope_of(&tenant, &repo, &q);
    let left = child::ack_mailbox(&*host()?, &scope, &RunId(run), &caller, body.up_to)
        .await
        .map_err(map_err)?;
    ok(serde_json::json!({ "remaining": left }))
}

/// A child posts a message to its parent.
pub async fn post_to_parent(
    Extension(tenant): Extension<TenantInfo>,
    Path((repo, run)): Path<(String, String)>,
    Query(q): Query<BranchQuery>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<child::PostRequest>,
) -> Resp {
    let caller = caller(auth)?;
    let scope = scope_of(&tenant, &repo, &q);
    ok(
        child::post_to_parent(&*host()?, &scope, &RunId(run), &caller, req)
            .await
            .map_err(map_err)?,
    )
}

/// Write a structured checkpoint.
pub async fn write_checkpoint(
    Extension(tenant): Extension<TenantInfo>,
    Path((repo, run)): Path<(String, String)>,
    Query(q): Query<BranchQuery>,
    auth: Option<Extension<AuthContext>>,
    Json(req): Json<child::CheckpointRequest>,
) -> Resp {
    let caller = caller(auth)?;
    let scope = scope_of(&tenant, &repo, &q);
    ok(
        child::checkpoint(&*host()?, &scope, &RunId(run), &caller, req)
            .await
            .map_err(map_err)?,
    )
}

/// Read checkpoint `n` (or `latest`).
pub async fn read_checkpoint(
    Extension(tenant): Extension<TenantInfo>,
    Path((repo, run, n)): Path<(String, String, String)>,
    Query(q): Query<BranchQuery>,
    auth: Option<Extension<AuthContext>>,
) -> Resp {
    let caller = caller(auth)?;
    let scope = scope_of(&tenant, &repo, &q);
    let n =
        match n.as_str() {
            "latest" => None,
            other => Some(other.parse::<u32>().map_err(|_| {
                ApiError::validation_failed("checkpoint must be a number or 'latest'")
            })?),
        };
    ok(
        child::read_checkpoint(&*host()?, &scope, &RunId(run), &caller, n)
            .await
            .map_err(map_err)?,
    )
}

/// Usage accounting of a run and its children.
pub async fn usage(
    Extension(tenant): Extension<TenantInfo>,
    Path((repo, run)): Path<(String, String)>,
    Query(q): Query<BranchQuery>,
    auth: Option<Extension<AuthContext>>,
) -> Resp {
    let caller = caller(auth)?;
    let scope = scope_of(&tenant, &repo, &q);
    ok(child::usage(&*host()?, &scope, &RunId(run), &caller)
        .await
        .map_err(map_err)?)
}
