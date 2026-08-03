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

//! The MCP server *descriptor* — the configuration content authors declare.
//!
//! A `raisin:McpServer` node in a workspace describes one MCP endpoint. This
//! module is the parsed, validated shape of that node; [`crate::registry`]
//! reads it and assembles the live tool set.
//!
//! Split by what an author is editing at the time:
//!
//! - [`descriptor`] — the node as a whole, and its parser.
//! - [`data_policy`] — which workspaces and operations the auto-tools cover.
//! - [`custom_tool`] — hand-declared tools and the function metadata they inherit.
//! - [`ui_binding`] — the MCP Apps (SEP-1865) widget an author attaches.
//! - [`ui_policy`] — what that widget asks the host to permit.

mod custom_tool;
mod data_policy;
mod descriptor;
mod ui_binding;
mod ui_policy;

pub use custom_tool::{CustomTool, FunctionMeta};
pub use data_policy::{DataOperation, DataPolicy};
pub use descriptor::McpServerDescriptor;
pub use ui_binding::{split_entry, UiBinding, UiMode, UiResource};
pub use ui_policy::{UiCsp, UiPermissionGrant, UiPermissions};
