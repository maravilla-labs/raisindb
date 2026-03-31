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

//! Permission models for Row-Level Security (RLS).
//!
//! This module defines the core permission structures used throughout RaisinDB:
//!
//! - [`Permission`] - A single permission grant with path pattern, operations, and conditions
//! - [`RoleCondition`] - Conditions that must be met for a permission to apply
//! - [`ResolvedPermissions`] - The flattened, resolved permissions for a user
//! - [`PermissionScope`] - Execution context (workspace, branch) for permission evaluation
//! - [`ScopeMatcher`] - Pre-compiled workspace/branch pattern matcher
//! - [`PathMatcher`] - Pre-compiled path pattern matcher

mod condition;
pub mod path_matcher;
mod permission;
mod resolution;
pub mod scope_matcher;

pub use condition::*;
pub use path_matcher::PathMatcher;
pub use permission::*;
pub use resolution::*;
pub use scope_matcher::{PermissionScope, ScopeMatcher};
