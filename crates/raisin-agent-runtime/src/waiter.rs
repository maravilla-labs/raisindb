// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Something OUTSIDE the run tree waiting for a run to end — a flow step that
//! runs an agent.
//!
//! It is the hand-back of a child run, pointed at a flow instance instead of a
//! parent's mailbox, and it takes the same durable path: the run's terminal
//! commit leaves it *hand-back owed* (the store's worklist), a wake of the
//! terminal run — or the sweeper, on whichever node — hands the result to the
//! [`WaiterSink`] (in a server: a `FlowInstanceExecution` resume job), and only
//! then commits `WaiterNotified`. A crash in between re-delivers; the sink
//! deduplicates by run, and the flow ignores a result it is not waiting for.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::events::RunEventKind;
use crate::record::AgentRunRecord;
use crate::state::RunState;
use crate::tx::{Transition, Tx};

/// The waiter kind a flow step registers.
pub const FLOW_INSTANCE: &str = "flow_instance";

/// What waits for a run to end.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunWaiter {
    /// What waits ([`FLOW_INSTANCE`]).
    pub kind: String,
    /// Its id (the flow instance id).
    pub target: String,
    /// The branch it runs on (tenant and repository are the run's).
    pub branch: String,
    /// Opaque data handed back with the result (e.g. the waiting step).
    #[serde(default)]
    pub data: Value,
    /// Whether the result has been handed to the sink.
    #[serde(default)]
    pub delivered: bool,
}

/// Where a terminal run's result goes.
#[async_trait]
pub trait WaiterSink: Send + Sync {
    /// Hand `result` to `waiter`. Must be idempotent per run: a crash after a
    /// successful hand-over delivers again.
    async fn notify(
        &self,
        run: &AgentRunRecord,
        waiter: &RunWaiter,
        result: Value,
    ) -> Result<(), String>;
}

impl AgentRunRecord {
    /// Whether this (terminal) run still owes a hand-back: to its parent's
    /// mailbox, or to what waits for it.
    pub fn handback_owed(&self) -> bool {
        self.state.is_terminal()
            && (self
                .delegation
                .as_ref()
                .is_some_and(|d| !d.handback_delivered)
                || self.waiter.as_ref().is_some_and(|w| !w.delivered))
    }
}

/// The result a waiter receives: the run's status and outcome.
pub fn waiter_result(rec: &AgentRunRecord) -> Option<Value> {
    let waiter = rec.waiter.as_ref()?;
    let RunState::Terminal { terminal, outcome } = &rec.state else {
        return None;
    };
    Some(json!({
        "agent_run_id": rec.run_id,
        "status": terminal.status(),
        "outcome": outcome,
        "usage": rec.usage,
        "waiter": { "kind": waiter.kind, "target": waiter.target, "data": waiter.data },
    }))
}

/// Mark the waiter notified. `None` when nothing is owed.
pub fn apply_waiter_notified(rec: &AgentRunRecord, now: u64) -> Option<Transition> {
    let w = rec.waiter.as_ref()?;
    if w.delivered || !rec.state.is_terminal() {
        return None;
    }
    let (kind, target) = (w.kind.clone(), w.target.clone());
    let mut tx = Tx::new(rec, now);
    if let Some(w) = tx.rec.waiter.as_mut() {
        w.delivered = true;
    }
    tx.push(RunEventKind::WaiterNotified { kind, target });
    Some(tx.finish())
}
