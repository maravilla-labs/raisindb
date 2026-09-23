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

//! The durable AgentRun runtime.
//!
//! A run is ONE aggregate, guarded by two locks that are never confused:
//!
//! - the store's short, in-process **commit** critical section (per run) — every
//!   commit takes it for microseconds; controls take ONLY this, so a stop never
//!   waits behind a model call;
//! - the **execution lease** — owner, expiry and a fencing token
//!   (`lease_epoch`), persisted in the record and held by a driver for the life
//!   of an operation.
//!
//! There is one write path ([`store::AgentRunStore::commit`]), which checks the
//! fence, the version and the invariants inside its critical section. Seqs come
//! from persisted state (`last_seq + 1`), never a process-local counter.
//! Operation ids are deterministic (`{run}/op/{n}`) and are the idempotency keys
//! of mutating work. One operation at a time per run. Only durable facts enter
//! the log; the log is also the domain reducer's inbox.
//!
//! Core knows nothing about any domain: a domain plugs in as a
//! [`domain::DomainReducer`] speaking the `raisin-agent-contract` envelopes.
//!
//! Runs form trees. A parent spawns a child with a typed objective
//! ([`child::ChildObjective`]); admission reserves the child's budgets out of
//! the parent's, the child's terminal hand-back lands in the parent's durable
//! mailbox, and a terminal parent stops its live children. Every cross-run step
//! is an idempotent commit on ONE run, retried by the job queue and the
//! sweeper on whichever node picks it up ([`service_child`]). Checkpoints are
//! structured records that reference large results instead of copying them
//! ([`service_child_ctl::CheckpointWrite`]).

#![warn(missing_docs)]

pub mod api;
pub mod api_child;
pub mod api_driver;
pub mod api_follow;
pub mod budget;
pub mod cancel;
pub mod checkpoint;
pub mod child;
pub mod child_admit;
pub mod child_apply;
pub mod clock;
pub mod control;
mod control_apply;
pub mod domain;
mod domain_apply;
pub mod domain_step;
pub mod driver;
pub mod events;
pub mod host;
pub mod ids;
pub mod keyed;
pub mod lifecycle;
mod lifecycle_ops;
pub mod memory;
pub mod record;
pub mod service;
pub mod service_child;
pub mod service_child_ctl;
pub mod service_exec;
pub mod state;
pub mod store;
pub mod tx;
pub mod waiter;
pub mod wake;

#[cfg(any(test, feature = "testing"))]
pub mod conformance;
#[cfg(any(test, feature = "testing"))]
mod conformance_child;
#[cfg(any(test, feature = "testing"))]
pub mod testing;

pub use raisin_agent_contract as contract;
