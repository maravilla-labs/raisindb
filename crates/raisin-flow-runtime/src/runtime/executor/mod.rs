// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Main flow execution engine with hybrid batching and OCC.
//!
//! The executor:
//! - Loads flow instances with version for OCC
//! - Executes steps continuously until async boundary
//! - Handles idempotency (skips already completed flows)
//! - Retries on version conflicts
//! - Manages compensation stack for rollback

mod execution_loop;
mod helpers;
mod isolated_branch;
mod result_handlers;
mod step_dispatch;

pub use execution_loop::execute_flow;
