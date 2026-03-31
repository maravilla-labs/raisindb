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

// TODO(v0.2): Clean up unused code
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(unexpected_cfgs)]

//! # Raisin Functions
//!
//! Serverless functions for RaisinDB enabling custom logic execution in
//! JavaScript (QuickJS) and Starlark (Python-like) runtimes.
//!
//! ## Features
//!
//! - **Multiple Runtimes**: JavaScript (QuickJS), Starlark (Python-like), SQL
//! - **Sandboxed Execution**: Resource limits, timeouts, memory constraints
//! - **RaisinDB API Access**: Node operations, SQL queries, events
//! - **Allowlisted HTTP**: Controlled external API access
//! - **Flexible Triggers**: Event-driven, scheduled, HTTP, SQL calls
//!
//! ## Example JavaScript Function
//!
//! ```javascript
//! async function handler(input) {
//!     const node = await raisin.nodes.get("default", input.path);
//!     await raisin.nodes.update("default", input.path, {
//!         properties: { ...node.properties, status: "processed" }
//!     });
//!     return { success: true };
//! }
//! ```

pub mod api;
pub mod execution;
pub mod executor;
pub mod loader;
pub mod runtime;
pub mod types;

// Re-exports
pub use api::{
    // Callback types for building RaisinFunctionApi
    EmitEventCallback,
    FunctionApi,
    HttpRequestCallback,
    NodeCreateCallback,
    NodeDeleteCallback,
    NodeGetByIdCallback,
    NodeGetCallback,
    NodeGetChildrenCallback,
    NodeQueryCallback,
    NodeUpdateCallback,
    RaisinFunctionApi,
    RaisinFunctionApiCallbacks,
    SqlExecuteCallback,
    SqlQueryCallback,
};
pub use executor::{ExecutionHandle, FunctionExecutor};
pub use loader::FunctionLoader;
pub use runtime::{FunctionRuntime, RuntimeRegistry};
pub use types::*;
