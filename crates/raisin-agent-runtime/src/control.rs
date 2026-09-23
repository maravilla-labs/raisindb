// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Control commands: atomic, deduplicated by `control_id`, authorized.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{ControlId, PrincipalKind, RequestId, Seq, SystemToken};
use crate::record::{AgentRunRecord, RunBudgets};

/// A control command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlCommand {
    /// Idempotency key.
    pub control_id: ControlId,
    /// What to do.
    pub kind: ControlKind,
    /// Who asks.
    pub issued_by: ActorRef,
    /// When, epoch ms (informational; the store's clock orders events).
    pub at_ms: u64,
}

/// Who issues a control. The transport maps the authenticated caller onto it;
/// core never trusts a self-declared id without its transport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActorRef {
    /// Kind.
    pub kind: PrincipalKind,
    /// Id.
    pub id: String,
    /// A control capability, checked against the stored hash. Never logged:
    /// it is stripped before the command enters the event log.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
}

impl ActorRef {
    /// A user actor.
    pub fn user(id: &str) -> Self {
        Self {
            kind: PrincipalKind::User,
            id: id.into(),
            capability: None,
        }
    }

    /// This actor without its capability secret.
    pub fn redacted(&self) -> Self {
        Self {
            capability: None,
            ..self.clone()
        }
    }
}

/// What a control does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum ControlKind {
    /// Stop the run.
    Stop {
        /// Reason.
        #[serde(default)]
        reason: Option<String>,
    },
    /// Pause at the next boundary.
    Pause,
    /// Resume a paused run.
    Resume {
        /// New budget values (absolute; `None` fields keep the old value).
        #[serde(default)]
        budget_increase: Option<RunBudgets>,
        /// Accept a changed reducer artifact after a `reducer_changed` pause.
        #[serde(default)]
        accept_reducer_change: bool,
    },
    /// Queue input for the next safe boundary.
    Steer {
        /// The input.
        input: Value,
    },
    /// Decide an approval request.
    Approve {
        /// The request.
        request_id: RequestId,
        /// The decision.
        decision: ApprovalDecision,
        /// Must equal the request's digest.
        subject_digest: String,
    },
    /// Answer an input request.
    ProvideInput {
        /// The request.
        request_id: RequestId,
        /// The value.
        value: Value,
    },
}

/// An approval decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum ApprovalDecision {
    /// Approve.
    Approve,
    /// Reject.
    Reject {
        /// Why.
        #[serde(default)]
        reason: Option<String>,
    },
}

/// The answer to a control.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "ack", rename_all = "snake_case")]
pub enum ControlAck {
    /// Applied; `seq` is the `ControlApplied` event.
    Applied {
        /// Seq.
        seq: Seq,
    },
    /// Already applied under this `control_id`; nothing new was written.
    Duplicate {
        /// Seq of the original ack event.
        original_seq: Seq,
    },
    /// Rejected, and the rejection logged at `seq`.
    Rejected {
        /// Why.
        reason: String,
        /// Seq of `ControlRejected`.
        seq: Seq,
    },
}

impl ControlAck {
    /// The seq the ack points at.
    pub fn seq(&self) -> Seq {
        match self {
            Self::Applied { seq } | Self::Rejected { seq, .. } => *seq,
            Self::Duplicate { original_seq } => *original_seq,
        }
    }

    /// This ack seen again under the same `control_id`.
    pub fn as_duplicate(&self) -> Self {
        match self {
            Self::Duplicate { .. } => self.clone(),
            other => Self::Duplicate {
                original_seq: other.seq(),
            },
        }
    }
}

/// `blake3` hex of a capability secret.
pub fn capability_hash(capability: &str) -> String {
    blake3::hash(capability.as_bytes()).to_hex().to_string()
}

/// `blake3` hex over the canonical JSON of a control's payload.
pub fn payload_digest(kind: &ControlKind) -> String {
    let value = serde_json::to_value(kind).unwrap_or(Value::Null);
    let canonical = raisin_agent_contract::canonical_json(&value);
    blake3::hash(canonical.as_bytes()).to_hex().to_string()
}

/// Whether `actor` may control `record`.
///
/// Any one of: the actor IS the principal (or the user it acts for); the
/// actor presents the capability given at create; the actor is `System` and
/// the caller holds a [`SystemToken`].
pub fn authorize(record: &AgentRunRecord, actor: &ActorRef, system: Option<&SystemToken>) -> bool {
    if actor.kind == PrincipalKind::System {
        return system.is_some();
    }
    // The id alone is not an identity: a user and an agent may share an id
    // string, so the KIND must match too. The person an agent acts for is a
    // user, so `on_behalf_of` only authorizes a user actor.
    let is_principal = actor.kind == record.principal.kind && actor.id == record.principal.id;
    let is_on_behalf = actor.kind == PrincipalKind::User
        && record.principal.on_behalf_of.as_deref() == Some(actor.id.as_str());
    if is_principal || is_on_behalf {
        return true;
    }
    match (&actor.capability, &record.control_capability_hash) {
        (Some(cap), Some(hash)) => &capability_hash(cap) == hash,
        _ => false,
    }
}
