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

//! Trigger evaluation job handler
//!
//! This module handles trigger evaluation when node events occur.
//! When a TriggerEvaluation job is queued, this handler finds all matching
//! triggers (both inline on raisin:Function nodes and standalone raisin:Trigger nodes)
//! and enqueues FunctionExecution jobs for each match.

mod handler;
mod types;

pub use handler::TriggerEvaluationHandler;
pub use types::{
    FilterCheckResult, NodeFetcherCallback, TriggerEvaluationReport, TriggerEvaluationResult,
    TriggerEventInfo, TriggerMatch, TriggerMatcherCallback,
};
