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

//! FlowCallbacks implementation for RocksDB storage.
//!
//! This module provides a concrete implementation of the `FlowCallbacks` trait
//! from `raisin-flow-runtime`, bridging the flow runtime to actual storage,
//! job system, and AI operations.
//!
//! # Design
//!
//! The implementation uses callback functions for operations that require
//! access to storage and external services. This allows the transport layer
//! to provide the actual implementations during initialization.

mod builder;
#[cfg(test)]
mod tests;
mod trait_impl;
mod types;

pub use builder::RocksDBFlowCallbacks;
pub use types::{
    AICallerCallback, AIStreamingCallerCallback, ChildrenListerCallback, FlowEventEmitterCallback,
    FunctionExecutorCallback, JobQueuerCallback, NodeCreatorCallback, NodeLoaderCallback,
    NodeSaverCallback,
};
