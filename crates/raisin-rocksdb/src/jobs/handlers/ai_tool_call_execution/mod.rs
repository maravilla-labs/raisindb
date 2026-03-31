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

//! AIToolCall execution job handler
//!
//! This module handles the OOTB (out of the box) execution of AI tool calls.
//! When an AIToolCall node is created with status='pending', this handler:
//! 1. Loads the AIToolCall node
//! 2. Updates status to 'running'
//! 3. Resolves function_ref to a Function node
//! 4. Executes the function inline (no nested job)
//! 5. Creates AIToolSingleCallResult child node (triggers aggregation job)
//! 6. Updates status to 'completed' or 'failed'
//!
//! This eliminates the need for a JavaScript tool-executor trigger handler
//! and avoids the deadlock issues with nested job execution.

mod aggregator;
mod auth_context;
mod execution;
mod handler;
mod types;

pub use self::handler::AIToolCallExecutionHandler;
pub use self::types::NodeCreatorCallback;
