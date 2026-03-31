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

pub mod audit_log;
pub mod types;
pub mod version;

// Bring Node and DeepNode types into this namespace directly from node/core/
#[path = "node/core/mod.rs"]
mod node_core;
pub use node_core::*;
#[path = "node/graph.rs"]
mod graph;
pub use graph::*;

// Re-export types for easier access
pub use types::*;
// Re-export version for easier access
pub use version::*;

// Re-export properties module
pub mod properties;

// Re-export audit_log module
pub use audit_log::*;
