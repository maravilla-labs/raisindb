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

//! RaisinDB Flow Runtime
//!
//! A stateful workflow execution engine for RaisinDB that enables:
//! - AI agent loops with tool calls
//! - Human-in-the-loop workflows
//! - Complex decision trees with branching
//! - Long-running workflows that can pause and resume
//! - Saga-based compensation for rollback
//!
//! # Architecture
//!
//! The flow runtime uses a hybrid batching model:
//! - Synchronous steps execute continuously without persistence
//! - Async operations (functions, AI, human tasks) create jobs and pause execution
//! - State is persisted at async boundaries with optimistic concurrency control
//!
//! # Example
//!
//! ```rust,ignore
//! use raisin_flow_runtime::types::{FlowInstance, FlowDefinition};
//!
//! // Create a flow instance
//! let instance = FlowInstance::new(
//!     "/flows/my-flow".to_string(),
//!     1,
//!     flow_definition_snapshot,
//!     input_data,
//!     "start".to_string(),
//! );
//!
//! // Execute the flow
//! // runtime.execute_flow(&instance).await?;
//! ```

// TODO(v0.2): Re-enable documentation warnings when docs are complete
// #![warn(missing_docs)]
// TODO(v0.2): Clean up unused code
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(ambiguous_glob_reexports)]
#![warn(clippy::all)]

pub mod compiler;
pub mod handlers;
pub mod integration;
pub mod runtime;
pub mod service;
pub mod types;

// Re-export all types and runtime functions at the crate root for convenience
pub use compiler::{CompiledFlow, CompiledMetadata, FlowCompiler};
pub use handlers::{
    AgentStepHandler, AiContainerHandler, ChatStepHandler, DecisionHandler, ErrorClass,
    FunctionStepHandler, HumanTaskHandler, OnErrorBehavior, ParallelHandler, StepError,
    StepHandler,
};
pub use integration::{FlowExecutionHandler, FlowResumeReason, FlowTriggerEvent};
pub use runtime::*;
pub use types::*;

/// Version of the flow runtime
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Flow runtime error type alias
pub type Error = types::FlowError;

/// Flow runtime result type alias
pub type Result<T> = std::result::Result<T, Error>;
