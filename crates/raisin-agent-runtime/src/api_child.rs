// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The public, transport-neutral API of child runs, mailboxes, checkpoints and
//! usage. HTTP, the function bindings (any language) and the SDKs all speak
//! these shapes; see [`crate::api`] for the run API they extend.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::api::{load_for, ApiError, Caller, Fence, ReducerSpec, R};
use crate::checkpoint::RunCheckpoint;
use crate::child::{ChildLink, SpawnChild};
use crate::control::{ActorRef, ControlAck};
use crate::events::RunEvent;
use crate::host::AgentRunHost;
use crate::ids::{PrincipalKind, RunId, RunScope, Seq, SystemToken};
use crate::lifecycle::NewRequest;
use crate::record::PendingKind;
use crate::service::AgentRunService;
use crate::service_child::SpawnOutcome;
use crate::service_child_ctl::{CheckpointWrite, ChildAction};
use crate::state::RunStatus;

fn actor(caller: &Caller) -> (ActorRef, Option<SystemToken>) {
    if caller.admin {
        (
            ActorRef {
                kind: PrincipalKind::System,
                id: caller.id.clone(),
                capability: None,
            },
            Some(SystemToken::in_process()),
        )
    } else {
        (ActorRef::user(&caller.id), None)
    }
}

/// Spawn a child.
#[derive(Debug, Clone, Deserialize)]
pub struct SpawnRequest {
    /// Objective, budgets, spawn key, … (see [`SpawnChild`]).
    #[serde(flatten)]
    pub spawn: SpawnChild,
    /// Server-driven child: its reducer function (any language).
    #[serde(default)]
    pub reducer: Option<ReducerSpec>,
    /// Server-driven child bound to the PARENT's reducer (same artifact).
    #[serde(default)]
    pub inherit_reducer: bool,
}

/// Spawn a child of `parent`.
pub async fn spawn(
    host: &AgentRunHost,
    scope: &RunScope,
    parent: &RunId,
    caller: &Caller,
    req: SpawnRequest,
) -> R<SpawnOutcome> {
    let prec = load_for(host, scope, parent, caller).await?;
    let mut spawn = req.spawn;
    spawn.reducer =
        match (&req.reducer, req.inherit_reducer) {
            (Some(spec), _) => Some(
                host.reducers()
                    .bind(
                        scope,
                        &spec.function_path,
                        spec.handler.as_deref().unwrap_or(""),
                    )
                    .await
                    .map_err(|e| ApiError::new(422, "reducer_unavailable", e.to_string()))?,
            ),
            (None, true) => Some(prec.domain.as_ref().map(|d| d.reducer.clone()).ok_or_else(
                || ApiError::new(400, "invalid", "the parent has no reducer to inherit"),
            )?),
            (None, false) => None,
        };
    let (who, system) = actor(caller);
    Ok(host
        .service()
        .spawn_child(scope, parent, spawn, &who, system.as_ref())
        .await?)
}

/// A child as its parent lists it.
#[derive(Debug, Clone, Serialize)]
pub struct ChildView {
    /// The parent's link (reservation, hand-back key, final usage).
    pub link: ChildLink,
    /// The child's CURRENT status (from its own record).
    pub status: Option<RunStatus>,
    /// The child's own usage so far.
    pub usage: Option<crate::record::RunUsage>,
}

/// Every child of `parent`.
pub async fn children(
    host: &AgentRunHost,
    scope: &RunScope,
    parent: &RunId,
    caller: &Caller,
) -> R<Vec<ChildView>> {
    let prec = load_for(host, scope, parent, caller).await?;
    let mut out = Vec::with_capacity(prec.children.len());
    for link in prec.children {
        let rec = host.service().get(scope, &link.run_id).await?;
        out.push(ChildView {
            status: rec.as_ref().map(|r| r.state.status()),
            usage: rec.as_ref().map(|r| r.usage),
            link,
        });
    }
    Ok(out)
}

async fn check_lineage(
    host: &AgentRunHost,
    scope: &RunScope,
    parent: &RunId,
    child: &RunId,
    caller: &Caller,
) -> R<()> {
    let prec = load_for(host, scope, parent, caller).await?;
    if !prec.children.iter().any(|l| &l.run_id == child) {
        return Err(ApiError::new(
            404,
            "unknown_child",
            format!("{child} is not a child of {parent}"),
        ));
    }
    Ok(())
}

/// A parent's inspection of one child.
#[derive(Debug, Clone, Serialize)]
pub struct ChildInspection {
    /// The child's run view.
    pub run: crate::api::RunView,
    /// Its events after `after_seq`.
    pub events: Vec<RunEvent>,
    /// Its usage accounting.
    pub usage: Value,
    /// Its latest checkpoint, if any.
    pub checkpoint: Option<RunCheckpoint>,
}

/// Inspect one child: record, recent events, usage, latest checkpoint.
pub async fn inspect(
    host: &AgentRunHost,
    scope: &RunScope,
    parent: &RunId,
    child: &RunId,
    caller: &Caller,
    after_seq: u64,
    limit: usize,
) -> R<ChildInspection> {
    check_lineage(host, scope, parent, child, caller).await?;
    // The lineage is the authority: whoever may read the parent may read its
    // children, even one running as another agent.
    let lineage = Caller {
        id: caller.id.clone(),
        admin: true,
    };
    let run = crate::api::get(host, scope, child, &lineage).await?;
    let events = host
        .service()
        .read_events(scope, child, Seq(after_seq), limit)
        .await?;
    let usage = AgentRunService::usage_report(&run.run, host.service().now());
    let checkpoint = host
        .service()
        .store()
        .latest_checkpoint(scope, child)
        .await
        .map_err(crate::service::ServiceError::from)?;
    Ok(ChildInspection {
        run,
        events,
        usage,
        checkpoint,
    })
}

/// Message, steer, interrupt or resume a child.
#[derive(Debug, Clone, Deserialize)]
pub struct ChildControlRequest {
    /// Idempotency key.
    pub control_id: String,
    /// What to do.
    #[serde(flatten)]
    pub action: ChildAction,
}

/// A parent controls a child.
pub async fn control_child(
    host: &AgentRunHost,
    scope: &RunScope,
    parent: &RunId,
    child: &RunId,
    caller: &Caller,
    req: ChildControlRequest,
) -> R<ControlAck> {
    load_for(host, scope, parent, caller).await?;
    let (who, system) = actor(caller);
    Ok(host
        .service()
        .control_child(
            scope,
            parent,
            child,
            req.action,
            &req.control_id,
            &who,
            system.as_ref(),
        )
        .await?)
}

/// A client-driven parent waits (under its lease) for a child.
#[derive(Debug, Clone, Deserialize)]
pub struct WaitChildRequest {
    /// The parent's lease.
    pub fence: Fence,
    /// Expiry.
    #[serde(default)]
    pub expires_at_ms: Option<u64>,
}

/// Move the parent to `Waiting` on a child's hand-back.
pub async fn wait_child(
    host: &AgentRunHost,
    scope: &RunScope,
    parent: &RunId,
    child: &RunId,
    caller: &Caller,
    req: WaitChildRequest,
) -> R<RunStatus> {
    check_lineage(host, scope, parent, child, caller).await?;
    let request = NewRequest {
        kind: PendingKind::Child {
            child_run_id: child.clone(),
        },
        effect_id: None,
        expires_at_ms: req.expires_at_ms,
    };
    Ok(host
        .service()
        .wait(scope, parent, &(&req.fence).into(), vec![request])
        .await?
        .state
        .status())
}

/// The unacknowledged mailbox, payloads included.
pub async fn mailbox(
    host: &AgentRunHost,
    scope: &RunScope,
    run: &RunId,
    caller: &Caller,
) -> R<Vec<Value>> {
    load_for(host, scope, run, caller).await?;
    Ok(host.service().mailbox(scope, run).await?)
}

/// Acknowledge mailbox items up to `up_to`; returns how many remain.
pub async fn ack_mailbox(
    host: &AgentRunHost,
    scope: &RunScope,
    run: &RunId,
    caller: &Caller,
    up_to: u64,
) -> R<usize> {
    load_for(host, scope, run, caller).await?;
    let (who, system) = actor(caller);
    Ok(host
        .service()
        .ack_mailbox(scope, run, up_to, &who, system.as_ref())
        .await?)
}

/// A child posts to its parent.
#[derive(Debug, Clone, Deserialize)]
pub struct PostRequest {
    /// Idempotency key.
    pub message_id: String,
    /// The message (bounded).
    pub message: Value,
}

/// Post a message from `child` to its parent's mailbox.
pub async fn post_to_parent(
    host: &AgentRunHost,
    scope: &RunScope,
    child: &RunId,
    caller: &Caller,
    req: PostRequest,
) -> R<Value> {
    load_for(host, scope, child, caller).await?;
    let (who, system) = actor(caller);
    let mail_no = host
        .service()
        .post_to_parent(
            scope,
            child,
            req.message,
            &req.message_id,
            &who,
            system.as_ref(),
        )
        .await?;
    Ok(json!({ "mail_no": mail_no }))
}

/// Write a checkpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct CheckpointRequest {
    /// The writer's lease, when it holds one.
    #[serde(default)]
    pub fence: Option<Fence>,
    /// What to write.
    #[serde(flatten)]
    pub write: CheckpointWrite,
}

/// Write a structured checkpoint (compaction).
pub async fn checkpoint(
    host: &AgentRunHost,
    scope: &RunScope,
    run: &RunId,
    caller: &Caller,
    req: CheckpointRequest,
) -> R<RunCheckpoint> {
    load_for(host, scope, run, caller).await?;
    let mut write = req.write;
    write.fence = req.fence.as_ref().map(Into::into);
    Ok(host.service().write_checkpoint(scope, run, write).await?)
}

/// Read a checkpoint (`None` = latest) with its structured state.
pub async fn read_checkpoint(
    host: &AgentRunHost,
    scope: &RunScope,
    run: &RunId,
    caller: &Caller,
    n: Option<u32>,
) -> R<Value> {
    load_for(host, scope, run, caller).await?;
    match host.service().read_checkpoint(scope, run, n).await? {
        Some((c, state)) => Ok(json!({ "checkpoint": c, "state": state })),
        None => Err(ApiError::new(404, "not_found", "no such checkpoint")),
    }
}

/// Usage accounting of a run and its children.
pub async fn usage(
    host: &AgentRunHost,
    scope: &RunScope,
    run: &RunId,
    caller: &Caller,
) -> R<Value> {
    let rec = load_for(host, scope, run, caller).await?;
    Ok(AgentRunService::usage_report(&rec, host.service().now()))
}
