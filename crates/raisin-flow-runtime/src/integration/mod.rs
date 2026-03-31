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

//! Integration layer for RaisinDB flow runtime
//!
//! This module provides the bridge between the flow runtime and RaisinDB's
//! job system. It includes:
//!
//! - Job handler for executing flow instances via the unified job queue
//! - Trigger type definitions for flow lifecycle events
//!
//! # Architecture
//!
//! The flow runtime integrates with RaisinDB through the unified job system:
//!
//! 1. **Flow Execution**: Jobs are registered with `JobRegistry.register_job()`
//!    and data is stored with `JobDataStore.put()`
//! 2. **State Persistence**: Flow state is persisted at async boundaries using
//!    the `FlowCallbacks` trait
//! 3. **Event-Driven Resumption**: Flows pause at async boundaries and resume
//!    when events arrive (tool results, human input, etc.)

pub mod job_handler;
pub mod triggers;

pub use job_handler::FlowExecutionHandler;
pub use triggers::{
    build_trigger_info_from_event, create_flow_instance_from_trigger, FlowInstanceBuilder,
    FlowResumeReason, FlowTriggerEvent,
};
