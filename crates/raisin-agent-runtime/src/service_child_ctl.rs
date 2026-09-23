// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The run service's parent-controls-child half (message, steer, interrupt,
//! resume), the mailbox, and structured checkpoints for compaction.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::checkpoint::RunCheckpoint;
use crate::child_apply::{apply_ack, apply_child_controlled, apply_post_to_parent};
use crate::control::{authorize, ActorRef, ControlAck, ControlCommand, ControlKind};
use crate::events::{CheckpointReason, ResultRef};
use crate::ids::{ControlId, OperationId, PrincipalKind, RunId, RunScope, SubjectRef, SystemToken};
use crate::lifecycle::{Fence, LeaseFence};
use crate::record::AgentRunRecord;
use crate::service::{AgentRunService, ServiceError};
use crate::tx::Tx;

/// What a parent does to a child.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ChildAction {
    /// Information, applied at the child's next safe boundary.
    Message {
        /// The message.
        message: Value,
    },
    /// New direction, applied at the child's next safe boundary.
    Steer {
        /// The input.
        input: Value,
    },
    /// Stop (default) or pause the child.
    Interrupt {
        /// `stop` or `pause`.
        #[serde(default)]
        mode: InterruptMode,
        /// Why.
        #[serde(default)]
        reason: Option<String>,
    },
    /// Resume a paused child.
    Resume,
}

/// How an interrupt lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterruptMode {
    /// Stop the child (terminal).
    #[default]
    Stop,
    /// Pause it (resumable).
    Pause,
}

impl ChildAction {
    fn name(&self) -> &'static str {
        match self {
            Self::Message { .. } => "message",
            Self::Steer { .. } => "steer",
            Self::Interrupt { .. } => "interrupt",
            Self::Resume => "resume",
        }
    }

    fn command(&self, parent: &RunId) -> ControlKind {
        match self {
            Self::Message { message } => ControlKind::Steer {
                input: json!({ "type": "parent_message", "from_run": parent, "message": message }),
            },
            Self::Steer { input } => ControlKind::Steer {
                input: json!({ "type": "parent_steer", "from_run": parent, "input": input }),
            },
            Self::Interrupt {
                mode: InterruptMode::Stop,
                reason,
            } => ControlKind::Stop {
                reason: Some(format!(
                    "interrupted_by_parent{}",
                    reason
                        .as_ref()
                        .map(|r| format!(": {r}"))
                        .unwrap_or_default()
                )),
            },
            Self::Interrupt {
                mode: InterruptMode::Pause,
                ..
            } => ControlKind::Pause,
            Self::Resume => ControlKind::Resume {
                budget_increase: None,
                accept_reducer_change: false,
            },
        }
    }
}

/// A checkpoint written for compaction (or any other reason).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CheckpointWrite {
    /// The writer's lease, for a driver holding it.
    #[serde(skip)]
    pub fence: Option<LeaseFence>,
    /// The in-flight operation writing it (a compaction running as an op).
    #[serde(default)]
    pub operation_id: Option<String>,
    /// Why (default: compaction).
    #[serde(default)]
    pub reason: Option<CheckpointReason>,
    /// Optional prose, never load-bearing.
    #[serde(default)]
    pub summary: Option<String>,
    /// The last transcript item folded in.
    #[serde(default)]
    pub transcript_cutoff: Option<SubjectRef>,
    /// Structured state (objective, constraints, decisions, pending
    /// questions, …), stored beside the run and referenced.
    #[serde(default)]
    pub state: Option<Value>,
    /// Extra large results to reference (their keys must exist).
    #[serde(default)]
    pub large_refs: Vec<ResultRef>,
}

fn actor_is_authorized(
    rec: &AgentRunRecord,
    actor: &ActorRef,
    system: Option<&SystemToken>,
) -> Result<(), ServiceError> {
    authorize(rec, actor, system)
        .then_some(())
        .ok_or(ServiceError::Unauthorized)
}

impl AgentRunService {
    /// A parent controls one of its children. The actor is authorized on the
    /// PARENT; the control reaches the child as the runtime (its lineage is
    /// the authority), deduplicated by `control_id`.
    #[allow(clippy::too_many_arguments)]
    pub async fn control_child(
        &self,
        scope: &RunScope,
        parent: &RunId,
        child: &RunId,
        action: ChildAction,
        control_id: &str,
        actor: &ActorRef,
        system: Option<&SystemToken>,
    ) -> Result<ControlAck, ServiceError> {
        let prec = self
            .store
            .load(scope, parent)
            .await?
            .ok_or(ServiceError::NotFound)?;
        actor_is_authorized(&prec, actor, system)?;
        if !prec.children.iter().any(|l| &l.run_id == child) {
            return Err(ServiceError::Invalid(format!(
                "{child} is not a child of {parent}"
            )));
        }
        let crec = self
            .store
            .load(scope, child)
            .await?
            .ok_or(ServiceError::NotFound)?;
        if crec.parent_run_id.as_ref() != Some(parent) {
            return Err(ServiceError::Invalid("lineage mismatch".into()));
        }
        let cid = ControlId(format!("parent:{control_id}"));
        let cmd = ControlCommand {
            control_id: cid.clone(),
            kind: action.command(parent),
            issued_by: ActorRef {
                kind: PrincipalKind::System,
                id: format!("parent:{parent}"),
                capability: None,
            },
            at_ms: self.now(),
        };
        let ack = self
            .submit_control(scope, child, cmd, Some(&SystemToken::in_process()))
            .await?;
        if !matches!(ack, ControlAck::Duplicate { .. }) {
            let label = match &ack {
                ControlAck::Applied { .. } => "applied".to_string(),
                ControlAck::Rejected { reason, .. } => format!("rejected:{reason}"),
                ControlAck::Duplicate { .. } => "duplicate".to_string(),
            };
            self.apply(scope, parent, Fence::None, |p, now| {
                Ok((
                    Some(apply_child_controlled(
                        p,
                        child,
                        action.name(),
                        &cid,
                        &label,
                        now,
                    )),
                    (),
                ))
            })
            .await?;
        }
        Ok(ack)
    }

    /// A child posts a message to its parent's mailbox. Deduplicated by
    /// `message_id`.
    pub async fn post_to_parent(
        &self,
        scope: &RunScope,
        child: &RunId,
        message: Value,
        message_id: &str,
        actor: &ActorRef,
        system: Option<&SystemToken>,
    ) -> Result<u64, ServiceError> {
        crate::ids::validate_key_part(message_id)
            .map_err(|e| ServiceError::Invalid(e.to_string()))?;
        let crec = self
            .store
            .load(scope, child)
            .await?
            .ok_or(ServiceError::NotFound)?;
        actor_is_authorized(&crec, actor, system)?;
        let parent = crec
            .parent_run_id
            .clone()
            .ok_or_else(|| ServiceError::Invalid("not a child run".into()))?;
        let key = format!("mail:{child}:{message_id}");
        let (mail_no, _) = self
            .apply(scope, &parent, Fence::None, |p, now| {
                if let Some(m) = p.mailbox.iter().find(|m| m.result_key == key) {
                    return Ok((None, m.mail_no));
                }
                let t = apply_post_to_parent(p, child, &message, message_id, now)?;
                let no = t.record.counters.mail;
                Ok((Some(t), no))
            })
            .await?;
        Ok(mail_no)
    }

    /// The unacknowledged mailbox with each item's payload.
    pub async fn mailbox(&self, scope: &RunScope, run: &RunId) -> Result<Vec<Value>, ServiceError> {
        let rec = self
            .store
            .load(scope, run)
            .await?
            .ok_or(ServiceError::NotFound)?;
        let mut out = Vec::with_capacity(rec.mailbox.len());
        for item in &rec.mailbox {
            let payload = self
                .store
                .read_result(scope, run, &item.result_key)
                .await?
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
            out.push(json!({ "item": item, "payload": payload }));
        }
        Ok(out)
    }

    /// Acknowledge mailbox items up to `up_to`.
    pub async fn ack_mailbox(
        &self,
        scope: &RunScope,
        run: &RunId,
        up_to: u64,
        actor: &ActorRef,
        system: Option<&SystemToken>,
    ) -> Result<usize, ServiceError> {
        let (left, _) = self
            .apply(scope, run, Fence::None, |rec, now| {
                actor_is_authorized(rec, actor, system)?;
                let t = apply_ack(rec, up_to, now);
                let left = t
                    .as_ref()
                    .map(|t| t.record.mailbox.len())
                    .unwrap_or(rec.mailbox.len());
                Ok((t, left))
            })
            .await?;
        Ok(left)
    }

    /// Write a structured checkpoint. The writer proves it may: it holds the
    /// lease (`fence`), or names the in-flight operation doing the
    /// compaction, or is authorized on a run no driver holds.
    pub async fn write_checkpoint(
        &self,
        scope: &RunScope,
        run: &RunId,
        w: CheckpointWrite,
    ) -> Result<RunCheckpoint, ServiceError> {
        for r in &w.large_refs {
            if self.store.read_result(scope, run, &r.key).await?.is_none() {
                return Err(ServiceError::Invalid(format!(
                    "unknown result ref '{}'",
                    r.key
                )));
            }
        }
        let fence = match &w.fence {
            Some(f) => Fence::Lease(f.clone()),
            None => Fence::None,
        };
        let (ckpt, _) = self
            .apply(scope, run, fence, |rec, now| {
                if w.fence.is_none() {
                    match (&w.operation_id, rec.state.active_op(), rec.state.lease()) {
                        (Some(op), Some(active), _) if active.op_id == OperationId(op.clone()) => {}
                        (Some(_), _, _) => {
                            return Err(ServiceError::Invalid("operation is not in flight".into()))
                        }
                        (None, _, Some(_)) => {
                            return Err(ServiceError::Invalid(
                                "a driver holds the run: present its fence".into(),
                            ))
                        }
                        (None, _, None) => {}
                    }
                }
                if rec.state.is_terminal() {
                    return Err(ServiceError::Invalid("run is terminal".into()));
                }
                let mut tx = Tx::new(rec, now);
                tx.checkpoint(
                    w.reason.unwrap_or(CheckpointReason::Compaction),
                    w.summary.clone(),
                );
                let no = tx.rec.counters.checkpoint;
                let structured = w.state.as_ref().map(|state| {
                    let bytes = serde_json::to_vec(state).unwrap_or_default();
                    let r = ResultRef {
                        key: format!("ckpt:{no}:state"),
                        bytes: bytes.len() as u64,
                        content_type: "application/json".into(),
                    };
                    tx.results.push((r.clone(), bytes));
                    r
                });
                let c = tx.checkpoint.as_mut().expect("checkpoint just built");
                c.transcript_cutoff = w.transcript_cutoff.clone();
                c.structured_ref = structured;
                for r in &w.large_refs {
                    if !c.large_refs.iter().any(|x| x.key == r.key) {
                        c.large_refs.push(r.clone());
                    }
                }
                let out = c.clone();
                Ok((Some(tx.finish()), out))
            })
            .await?;
        Ok(ckpt)
    }

    /// A checkpoint (`None` = the latest) and its structured state.
    pub async fn read_checkpoint(
        &self,
        scope: &RunScope,
        run: &RunId,
        checkpoint_no: Option<u32>,
    ) -> Result<Option<(RunCheckpoint, Option<Value>)>, ServiceError> {
        let c = match checkpoint_no {
            None => self.store.latest_checkpoint(scope, run).await?,
            Some(n) => self.store.read_checkpoint(scope, run, n).await?,
        };
        let Some(c) = c else { return Ok(None) };
        let state = match &c.structured_ref {
            Some(r) => self
                .store
                .read_result(scope, run, &r.key)
                .await?
                .and_then(|b| serde_json::from_slice(&b).ok()),
            None => None,
        };
        Ok(Some((c, state)))
    }
}
