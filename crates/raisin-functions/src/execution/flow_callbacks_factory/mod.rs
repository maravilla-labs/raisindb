// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Factory functions for creating flow instance execution callbacks
//!
//! This module provides factory functions that create the callback implementations
//! needed by `FlowInstanceExecutionHandler` to execute stateful workflows.
//!
//! # Architecture
//!
//! Flow instances require callbacks for:
//! - Node operations (load/save/create) - for flow instance persistence
//! - Job queuing - for async operations within flows
//! - AI calls - for AI-powered flow steps
//! - Function execution - for serverless function steps
//!
//! These callbacks are created from `ExecutionDependencies` which provides
//! access to storage, binary storage, indexing engines, etc.

mod ai_callback;
mod function_callback;
mod job_callback;
mod node_callbacks;
mod types;

pub use types::*;

use crate::execution::ExecutionDependencies;
use raisin_binary::BinaryStorage;
use raisin_storage::{transactional::TransactionalStorage, Storage};
use std::sync::Arc;

/// Create flow execution callbacks from execution dependencies
///
/// This function creates all the callbacks needed by FlowInstanceExecutionHandler
/// using the provided execution dependencies (storage, binary storage, etc.).
///
/// # Arguments
///
/// * `deps` - Shared execution dependencies (storage, indexing engines, etc.)
///
/// # Returns
///
/// A `FlowCallbacks` struct containing all callback implementations
pub fn create_flow_callbacks<S, B>(deps: Arc<ExecutionDependencies<S, B>>) -> FlowCallbacks
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    FlowCallbacks {
        node_loader: node_callbacks::create_node_loader(&deps),
        node_saver: node_callbacks::create_node_saver(&deps),
        node_creator: node_callbacks::create_node_creator(&deps),
        job_queuer: job_callback::create_job_queuer(&deps),
        ai_caller: ai_callback::create_ai_caller(&deps),
        ai_streaming_caller: Some(ai_callback::create_ai_streaming_caller(&deps)),
        function_executor: function_callback::create_function_executor(&deps),
        children_lister: node_callbacks::create_children_lister(&deps),
    }
}
