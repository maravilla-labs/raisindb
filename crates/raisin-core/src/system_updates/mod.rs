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

//! System updates module for detecting and applying changes to built-in definitions
//!
//! This module provides functionality to:
//! - Detect breaking changes between NodeType/Workspace versions
//! - Check for pending updates in a repository
//! - Apply updates with proper validation

mod breaking_changes;
mod pending;

pub use breaking_changes::{detect_nodetype_breaking_changes, detect_workspace_breaking_changes};
pub use pending::{check_pending_updates, PendingUpdatesChecker};
