// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Test doubles: a scripted planner, a recording executor that dedupes by
//! idempotency key and counts side effects, and a scripted reducer. The store
//! conformance suite is in [`crate::conformance`].

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use raisin_agent_contract::{ReducerRequest, ReducerResponse};
use serde_json::Value;

use crate::domain::{DomainReducer, ReducerCallError, ReducerRef};
use crate::driver::{ExecContext, NextAction, OperationExecutor, StepPlanner};
use crate::events::{OpOutcome, OpUsage};
use crate::ids::{CallId, OperationId};
use crate::lifecycle::OperationResult;
use crate::record::{AgentRunRecord, SteerEntry};

/// Returns scripted actions in order, then `Nothing`. Records the steers each
/// call saw.
#[derive(Default)]
pub struct ScriptedPlanner {
    script: Mutex<VecDeque<NextAction>>,
    seen: Mutex<Vec<Vec<SteerEntry>>>,
}

impl ScriptedPlanner {
    /// A planner that returns `actions` in order.
    pub fn new(actions: Vec<NextAction>) -> Self {
        Self {
            script: Mutex::new(actions.into()),
            seen: Mutex::default(),
        }
    }

    /// Append an action.
    pub fn push(&self, action: NextAction) {
        self.script
            .lock()
            .expect("planner poisoned")
            .push_back(action);
    }

    /// Every steer delivered to the planner, flattened.
    pub fn steers_seen(&self) -> Vec<SteerEntry> {
        self.seen
            .lock()
            .expect("planner poisoned")
            .iter()
            .flatten()
            .cloned()
            .collect()
    }
}

#[async_trait]
impl StepPlanner for ScriptedPlanner {
    async fn next(&self, _record: &AgentRunRecord, consumed: &[SteerEntry]) -> NextAction {
        self.seen
            .lock()
            .expect("planner poisoned")
            .push(consumed.to_vec());
        self.script
            .lock()
            .expect("planner poisoned")
            .pop_front()
            .unwrap_or(NextAction::Nothing)
    }
}

/// How the [`RecordingExecutor`] behaves for one operation.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExecBehavior {
    /// Virtual time the operation takes.
    pub delay_ms: u64,
    /// Outcome (default: succeeded).
    pub outcome: Option<OpOutcome>,
    /// Result payload.
    pub payload: Option<Value>,
    /// Model turn: tool calls requested.
    pub tool_calls: Vec<CallId>,
    /// Waiting: resume key.
    pub resume_key: Option<String>,
    /// Usage.
    pub usage: Option<OpUsage>,
}

/// Executes nothing real. Behaviors come from a script (then the default);
/// a repeated idempotency key returns the FIRST result without a new side
/// effect, which is the contract a replay-safe tool honours.
#[derive(Default)]
pub struct RecordingExecutor {
    default: ExecBehavior,
    script: Mutex<VecDeque<ExecBehavior>>,
    side_effects: Mutex<HashMap<String, u32>>,
    results: Mutex<HashMap<String, OperationResult>>,
    started: Mutex<Vec<(OperationId, u32)>>,
    cancelled: Mutex<Vec<OperationId>>,
}

impl RecordingExecutor {
    /// An executor whose default behavior is `default`.
    pub fn new(default: ExecBehavior) -> Self {
        Self {
            default,
            ..Self::default()
        }
    }

    /// Queue a behavior for the next execution.
    pub fn push(&self, behavior: ExecBehavior) {
        self.script
            .lock()
            .expect("executor poisoned")
            .push_back(behavior);
    }

    /// Side effects performed under `key`.
    pub fn side_effects(&self, key: &str) -> u32 {
        self.side_effects
            .lock()
            .expect("executor poisoned")
            .get(key)
            .copied()
            .unwrap_or(0)
    }

    /// Total side effects.
    pub fn total_side_effects(&self) -> u32 {
        self.side_effects
            .lock()
            .expect("executor poisoned")
            .values()
            .sum()
    }

    /// `(op, attempt)` of every execution started.
    pub fn started(&self) -> Vec<(OperationId, u32)> {
        self.started.lock().expect("executor poisoned").clone()
    }

    /// Operations that observed their cancellation token.
    pub fn observed_cancel(&self) -> Vec<OperationId> {
        self.cancelled.lock().expect("executor poisoned").clone()
    }
}

#[async_trait]
impl OperationExecutor for RecordingExecutor {
    async fn execute(&self, ctx: ExecContext) -> OperationResult {
        self.started
            .lock()
            .expect("executor poisoned")
            .push((ctx.op.op_id.clone(), ctx.op.attempt));
        let key = ctx.op.idempotency_key.clone();
        if let Some(r) = self
            .results
            .lock()
            .expect("executor poisoned")
            .get(&key)
            .cloned()
        {
            return r;
        }
        let b = self
            .script
            .lock()
            .expect("executor poisoned")
            .pop_front()
            .unwrap_or_else(|| self.default.clone());
        let sleep = tokio::time::sleep(std::time::Duration::from_millis(b.delay_ms));
        if ctx.op.interruptible {
            tokio::select! {
                _ = sleep => {}
                _ = ctx.token.cancelled() => {
                    self.cancelled.lock().expect("executor poisoned").push(ctx.op.op_id.clone());
                    return OperationResult { outcome: Some(OpOutcome::Cancelled), ..OperationResult::default() };
                }
            }
        } else {
            sleep.await;
            if ctx.token.is_cancelled() {
                self.cancelled
                    .lock()
                    .expect("executor poisoned")
                    .push(ctx.op.op_id.clone());
            }
        }
        *self
            .side_effects
            .lock()
            .expect("executor poisoned")
            .entry(key.clone())
            .or_default() += 1;
        let result = OperationResult {
            outcome: b.outcome,
            usage: b.usage,
            payload: b.payload,
            tool_calls: b.tool_calls,
            resume_key: b.resume_key,
        };
        self.results
            .lock()
            .expect("executor poisoned")
            .insert(key, result.clone());
        result
    }
}

/// The closure behind a [`ScriptedReducer`].
pub type ReduceFn =
    dyn Fn(&ReducerRequest) -> Result<ReducerResponse, ReducerCallError> + Send + Sync;

/// A reducer answering from a closure, recording every request.
pub struct ScriptedReducer {
    reducer: ReducerRef,
    f: Box<ReduceFn>,
    calls: Mutex<Vec<ReducerRequest>>,
}

impl ScriptedReducer {
    /// A reducer with artifact hash `hash`.
    pub fn new(
        hash: &str,
        f: impl Fn(&ReducerRequest) -> Result<ReducerResponse, ReducerCallError> + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            reducer: ReducerRef {
                function_path: "/lib/test/reducer".into(),
                handler: "reduce".into(),
                artifact_hash: hash.into(),
            },
            f: Box::new(f),
            calls: Mutex::default(),
        })
    }

    /// Every request seen.
    pub fn calls(&self) -> Vec<ReducerRequest> {
        self.calls.lock().expect("reducer poisoned").clone()
    }
}

#[async_trait]
impl DomainReducer for ScriptedReducer {
    fn reducer_ref(&self) -> &ReducerRef {
        &self.reducer
    }

    async fn reduce(&self, req: &ReducerRequest) -> Result<ReducerResponse, ReducerCallError> {
        self.calls
            .lock()
            .expect("reducer poisoned")
            .push(req.clone());
        (self.f)(req)
    }
}
