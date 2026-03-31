// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Callback type definitions for RaisinDB Function API
//!
//! This module defines all callback types used by the RaisinFunctionApi.
//! Callbacks connect the function runtime to backend services through
//! the transport layer.
//!
//! ## Submodules
//!
//! - [`node_ops`] - Node CRUD operation callbacks
//! - [`sql_ops`] - SQL query/execute callbacks
//! - [`service_ops`] - HTTP, Event, AI, PDF, Resource, Task, and Function callbacks
//! - [`transaction_ops`] - Transaction operation callbacks
//! - [`builder`] - `RaisinFunctionApiCallbacks` builder

mod builder;
mod node_ops;
mod service_ops;
mod sql_ops;
mod transaction_ops;

pub use builder::RaisinFunctionApiCallbacks;
pub use node_ops::*;
pub use service_ops::*;
pub use sql_ops::*;
pub use transaction_ops::*;
