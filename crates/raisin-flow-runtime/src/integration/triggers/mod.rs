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

//! Trigger type definitions for flow lifecycle events
//!
//! This module defines the events and reasons that cause flows to start or resume.
//! These types are used by the job system to queue flow execution jobs and by
//! the subscription system to match events to waiting flows.
//!
//! # Flow Lifecycle
//!
//! Flows can be triggered by:
//! 1. **Node Events** - Created, Updated, Deleted, Published
//! 2. **Tool Results** - AI assistant calls a tool, result arrives
//! 3. **Human Tasks** - Approval, form submission, manual input
//! 4. **Scheduled Events** - Cron expressions, one-time schedules
//! 5. **Custom Events** - Application-specific triggers

mod events;
mod instance_builder;

#[cfg(test)]
mod tests;

pub use events::{FlowResumeReason, FlowTriggerEvent};
pub use instance_builder::{
    build_trigger_info_from_event, create_flow_instance_from_trigger, FlowInstanceBuilder,
};
