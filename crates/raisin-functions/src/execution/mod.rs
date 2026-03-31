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

//! Execution module for job system callbacks.
//!
//! This module provides the callback implementations needed by the job system
//! to execute functions, SQL, and other background operations.
//!
//! # Overview
//!
//! The job system in `raisin-rocksdb` needs callbacks to execute various
//! operations. These callbacks were originally defined inline in `main.rs`
//! but have been extracted here for better maintainability.
//!
//! # Execution Modes
//!
//! - **Debug mode**: Prints events in color for development/debugging
//! - **Production mode**: Full execution (to be implemented)
//!
//! # Usage
//!
//! ```ignore
//! use raisin_functions::execution::{ExecutionProvider, ExecutionMode};
//!
//! // Create debug callbacks
//! let callbacks = ExecutionProvider::create_callbacks(ExecutionMode::Debug);
//!
//! // Or use environment-based configuration
//! let callbacks = ExecutionProvider::from_env();
//!
//! // Pass to init_job_system
//! let (pool, token) = storage.init_job_system(
//!     indexing_engine,
//!     hnsw_engine,
//!     callbacks.sql_executor,
//!     None, // copy_tree_executor
//!     callbacks.function_executor,
//!     callbacks.function_enabled_checker,
//!     trigger_matcher,
//!     None, // scheduled_trigger_finder
//!     callbacks.binary_retrieval,
//! ).await?;
//! ```
//!
//! # Module Structure
//!
//! - `types`: Callback type definitions and `ExecutionCallbacks` struct
//! - `debug`: Debug handlers that print colored output
//! - `provider`: Factory for creating callbacks
//! - `callbacks`: Production callback implementations (nodes, sql, http, ai, events)
//! - `code_loader`: Load function code from storage/binary
//! - `executor`: Main function execution orchestration
//! - `backup_main_callbacks`: Preserved original code from main.rs (reference only)

pub mod ai_provider;
mod backup_main_callbacks;
pub mod callbacks;
pub mod code_loader;
mod debug;
mod executor;
pub mod flow_callbacks_factory;
mod provider;
mod types;

// Re-export public API
pub use provider::ExecutionProvider;
pub use types::{
    BinaryRetrievalCallback, ExecutionCallbacks, ExecutionDependencies, ExecutionMode,
    FunctionContext, FunctionEnabledChecker, FunctionExecutionConfig, FunctionExecutionResult,
    FunctionExecutorCallback, SqlExecutorCallback,
};

// Re-export debug handlers for custom usage
pub use debug::{
    create_debug_binary_retrieval, create_debug_enabled_checker, create_debug_function_executor,
    create_debug_function_executor_with_storage, create_debug_sql_executor,
};

// Re-export executor functions
pub use executor::{create_function_checker, create_function_executor, execute_function};

// Re-export flow callbacks factory
pub use flow_callbacks_factory::{create_flow_callbacks, FlowCallbacks};
