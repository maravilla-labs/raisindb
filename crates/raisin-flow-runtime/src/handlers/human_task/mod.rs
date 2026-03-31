// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Human task handler
//!
//! Creates inbox tasks for human interaction in workflows.
//! Pauses flow execution until the task is completed by a user.
//!
//! # Example
//!
//! ```yaml
//! nodes:
//!   - id: approval-task
//!     type: human_task
//!     properties:
//!       task_type: approval
//!       title: "Approve Budget Request"
//!       description: "Please review and approve this budget request"
//!       assignee: "/users/manager"
//!       options:
//!         - value: "approve"
//!           label: "Approve"
//!           style: "success"
//!         - value: "reject"
//!           label: "Reject"
//!           style: "danger"
//!       due_in_seconds: 86400  # 24 hours
//!       priority: 4
//! ```

pub mod handler;
mod step_handler_impl;

#[cfg(test)]
mod tests;

pub use handler::HumanTaskHandler;
