// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Parallel execution handler
//!
//! Handles fork/join patterns for parallel execution of flow branches.
//!
//! # Fork Pattern
//!
//! When entering a parallel container, creates child flow instances for each
//! branch and returns a Wait result. The flow pauses until all branches complete.
//!
//! # Join Pattern
//!
//! When all branches complete, collects their outputs and merges them into
//! the parent flow context.

pub mod handler;
mod merge_strategies;
mod step_handler_impl;

#[cfg(test)]
mod tests;

pub use handler::ParallelHandler;
