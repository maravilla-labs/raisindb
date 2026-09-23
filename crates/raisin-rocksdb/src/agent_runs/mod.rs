// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Durable AgentRun storage on ordinary nodes, and the node plumbing that
//! drives runs through the job queue.
//!
//! # Persisted and replicated the way flow instances are
//!
//! A run is a tree of ordinary nodes in the `raisin:system` workspace — the
//! record, its events, acks, checkpoints, domain state and the indexes that
//! make every lookup a direct path read (`layout`). There is no run-specific
//! storage and no run-specific replication: each commit is ONE node
//! transaction (marked as engine bookkeeping), so it is durable, versioned and
//! replicated exactly like a `raisin:FlowInstance` save.
//!
//! # Cluster safety, with existing mechanisms only
//!
//! - **Exclusivity**: each commit's critical section is the flow-instance
//!   pattern — an in-process keyed mutex plus, when the locks subsystem is
//!   configured, a distributed `raisin-locks` lease (`lock`). A driver's
//!   long-lived claim on a run is the execution lease inside the record,
//!   fenced by its epoch on every commit.
//! - **Execution**: a wake is an `AgentRunStep` job on the ordinary queue
//!   (`waker`); any node's worker can run it, because nothing about a step is
//!   node-local. A sweeper re-issues lost wakes and takes over expired leases.
//! - **Completion**: what waits for a run outside its tree (a flow step) is
//!   resumed by a `FlowInstanceExecution` job (`waiter`), owed durably until
//!   it is enqueued.

mod executors;
mod host_slot;
mod layout;
mod lock;
mod principal_auth;
mod store;
mod store_writes;
mod waiter;
mod waker;

#[cfg(test)]
mod tests;

pub use executors::{
    model_turn_result, tool_result, FunctionCaller, FunctionModelTurnExecutor,
    FunctionOperationExecutor, ModelTurnExecutor, DEFAULT_MODEL_TURN_FUNCTION,
};
pub use host_slot::{agent_run_host, install_agent_run_host};
pub use principal_auth::resolve_principal_auth;
pub use store::NodeAgentRunStore;
pub use waiter::{FlowResumeSink, AGENT_RUN_RESUME_REASON};
pub use waker::{JobQueueWaker, AGENT_RUN_JOB_WORKSPACE};
