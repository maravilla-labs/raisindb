// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The run lifecycle as an enum in which illegal states cannot be written.
//!
//! None of these can be constructed: an active operation outside
//! `Running::Operating` or `Cancelling`; a lease outside `Running` or
//! `Cancelling`; `Cancelling` without an operation; `Waiting` with no open
//! request; an open request while an operation is in flight; a pending pause on
//! anything but an in-flight operation; a terminal run with a lease, an
//! operation or open requests.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::ids::ControlId;
use crate::record::{ActiveOperation, Lease, PendingRequest};

pub use raisin_agent_contract::RunStatus;

/// Where a run is in its lifecycle.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RunState {
    /// Waiting for a driver.
    Queued {
        /// Requests still open (e.g. after a steer woke a waiting run).
        open: Vec<PendingRequest>,
        /// Why it was queued.
        wake: WakeReason,
    },
    /// A driver holds the lease.
    Running {
        /// The lease.
        lease: Lease,
        /// Idle at a boundary, or operating.
        activity: Activity,
    },
    /// Stop requested while an operation is in flight.
    Cancelling {
        /// The lease of the driver running the operation.
        lease: Lease,
        /// The operation being cancelled.
        op: ActiveOperation,
        /// The stop.
        stop: StopInfo,
    },
    /// Blocked on at least one open request.
    Waiting {
        /// The open requests.
        open: NonEmpty<PendingRequest>,
    },
    /// Held; resumable.
    Paused {
        /// Requests still open.
        open: Vec<PendingRequest>,
        /// Why.
        reason: PauseReason,
    },
    /// Finished.
    Terminal {
        /// How.
        terminal: TerminalStatus,
        /// With what.
        outcome: RunOutcome,
    },
}

/// What a running driver is doing.
///
/// One value per run, persisted and cloned rarely; boxing the operation would
/// buy nothing but an indirection in every match.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "activity", rename_all = "snake_case")]
pub enum Activity {
    /// Between operations — the only safe boundary.
    Idle {
        /// Requests open at the boundary.
        open: Vec<PendingRequest>,
    },
    /// An operation is in flight; no open request can exist.
    Operating {
        /// The operation.
        op: ActiveOperation,
        /// A pause landed while it ran; applied when it finishes.
        pause_requested: bool,
    },
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalStatus {
    /// Finished normally.
    Completed,
    /// Failed.
    Failed,
    /// Stopped.
    Stopped,
}

impl TerminalStatus {
    /// The flat status.
    pub fn status(self) -> RunStatus {
        match self {
            Self::Completed => RunStatus::Completed,
            Self::Failed => RunStatus::Failed,
            Self::Stopped => RunStatus::Stopped,
        }
    }
}

/// What a finished run produced.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RunOutcome {
    /// `succeeded`, `partial`, `blocked`, `failed`, `stopped`, …
    pub kind: String,
    /// Stable code, for failures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Human message or summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Domain detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}

impl RunOutcome {
    /// An outcome of `kind` with an optional message.
    pub fn new(kind: &str, message: Option<String>) -> Self {
        Self {
            kind: kind.into(),
            message,
            ..Self::default()
        }
    }
}

/// Why a run was queued.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeReason {
    /// Just created.
    Created,
    /// A steer arrived.
    Steer,
    /// The last open request was resolved or expired.
    RequestResolved,
    /// Resumed from pause.
    Resumed,
    /// A driver released its lease at a boundary.
    LeaseReleased,
    /// An external result was delivered.
    ExternalResult,
    /// A domain run reached a terminal state that no driver will finalize
    /// (e.g. stopped while waiting): the woken driver delivers the rest of its
    /// log to the reducer.
    Finalize,
    /// A child ended: its hand-back is owed to its parent's mailbox.
    Handback,
    /// A parent ended with live children: they are owed a stop.
    Cascade,
}

/// Why a run is paused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PauseReason {
    /// A pause control.
    User,
    /// A budget was exceeded under policy `Pause`.
    Budget {
        /// Which budget.
        which: String,
    },
    /// The domain reducer could not be used (`reducer_unavailable`,
    /// `reducer_changed`); a redeploy or an explicit resume continues it.
    Reducer {
        /// Why.
        reason: String,
    },
}

/// The stop being applied to a cancelling run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopInfo {
    /// The stop control.
    pub control_id: ControlId,
    /// Reason given.
    pub reason: Option<String>,
}

/// A vector with at least one element. Serialized as an array; `[]` does not
/// deserialize.
#[derive(Debug, Clone, PartialEq)]
pub struct NonEmpty<T> {
    head: T,
    tail: Vec<T>,
}

impl<T> NonEmpty<T> {
    /// One element.
    pub fn new(head: T) -> Self {
        Self {
            head,
            tail: Vec::new(),
        }
    }

    /// `None` for an empty vector.
    pub fn from_vec(mut v: Vec<T>) -> Option<Self> {
        if v.is_empty() {
            return None;
        }
        let head = v.remove(0);
        Some(Self { head, tail: v })
    }

    /// Back to a vector.
    pub fn into_vec(self) -> Vec<T> {
        let mut v = Vec::with_capacity(1 + self.tail.len());
        v.push(self.head);
        v.extend(self.tail);
        v
    }

    /// Iterate.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        std::iter::once(&self.head).chain(self.tail.iter())
    }

    /// Number of elements (≥ 1).
    pub fn len(&self) -> usize {
        1 + self.tail.len()
    }

    /// Always false; present for API symmetry.
    pub fn is_empty(&self) -> bool {
        false
    }
}

impl<T: Clone> NonEmpty<T> {
    /// A cloned vector.
    pub fn to_vec(&self) -> Vec<T> {
        self.clone().into_vec()
    }
}

impl<T: Serialize> Serialize for NonEmpty<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(self.iter())
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for NonEmpty<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Vec::<T>::deserialize(d)?;
        Self::from_vec(v).ok_or_else(|| serde::de::Error::custom("empty array for NonEmpty"))
    }
}

impl RunState {
    /// The flat status tag.
    pub fn status(&self) -> RunStatus {
        match self {
            Self::Queued { .. } => RunStatus::Queued,
            Self::Running { .. } => RunStatus::Running,
            Self::Cancelling { .. } => RunStatus::Cancelling,
            Self::Waiting { .. } => RunStatus::Waiting,
            Self::Paused { .. } => RunStatus::Paused,
            Self::Terminal { terminal, .. } => terminal.status(),
        }
    }

    /// The lease, where one exists.
    pub fn lease(&self) -> Option<&Lease> {
        match self {
            Self::Running { lease, .. } | Self::Cancelling { lease, .. } => Some(lease),
            _ => None,
        }
    }

    /// The in-flight operation, where one exists.
    pub fn active_op(&self) -> Option<&ActiveOperation> {
        match self {
            Self::Running {
                activity: Activity::Operating { op, .. },
                ..
            }
            | Self::Cancelling { op, .. } => Some(op),
            _ => None,
        }
    }

    /// The open requests (empty where none can exist).
    pub fn open_requests(&self) -> Vec<PendingRequest> {
        match self {
            Self::Queued { open, .. }
            | Self::Paused { open, .. }
            | Self::Running {
                activity: Activity::Idle { open },
                ..
            } => open.clone(),
            Self::Waiting { open } => open.to_vec(),
            _ => Vec::new(),
        }
    }

    /// Whether the run has ended.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Terminal { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_empty_rejects_empty_array() {
        let err = serde_json::from_str::<NonEmpty<u32>>("[]");
        assert!(err.is_err());
        let ok: NonEmpty<u32> = serde_json::from_str("[1,2]").unwrap();
        assert_eq!(ok.to_vec(), vec![1, 2]);
        assert_eq!(serde_json::to_string(&ok).unwrap(), "[1,2]");
    }
}
