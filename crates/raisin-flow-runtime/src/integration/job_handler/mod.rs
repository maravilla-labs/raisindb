// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Job handler for flow execution
//!
//! This module provides the `FlowExecutionHandler` which processes flow execution
//! jobs through the unified job queue system. It follows the same pattern as
//! other job handlers in `raisin-rocksdb/src/jobs/handlers/`.
//!
//! # Architecture
//!
//! When a flow needs to be executed or resumed:
//!
//! 1. A job is created with `JobRegistry.register_job()` using `JobType::FlowExecution`
//! 2. Flow instance data is stored with `JobDataStore.put()`
//! 3. The handler loads the instance, executes steps, and persists state
//! 4. At async boundaries (AI calls, human tasks), execution pauses and a new job is queued
//!
//! # Error Handling
//!
//! - Transient errors trigger automatic retries via the job system
//! - Fatal errors update the flow instance status to Failed
//! - Compensation runs on failure (if configured in the flow definition)

mod handler;
mod step_execution;

#[cfg(test)]
mod tests;

pub use handler::FlowExecutionHandler;
