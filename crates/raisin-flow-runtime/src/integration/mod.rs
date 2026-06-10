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
//! This module provides trigger type definitions for flow lifecycle events.
//!
//! Flow instance execution itself is handled by the production job handler
//! in raisin-rocksdb (`FlowInstanceExecutionHandler`), which drives
//! `runtime::execute_flow` / `resume_flow` / `check_flow_timeout`.

pub mod triggers;

pub use triggers::{
    build_trigger_info_from_event, create_flow_instance_from_trigger, FlowInstanceBuilder,
    FlowResumeReason, FlowTriggerEvent,
};
