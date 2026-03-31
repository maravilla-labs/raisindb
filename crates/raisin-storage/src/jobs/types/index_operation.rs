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

//! Index operation types for fulltext and batch indexing

use serde::{Deserialize, Serialize};

/// Operation type for index jobs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndexOperation {
    /// Add new node or update existing
    AddOrUpdate,
    /// Delete node from index
    Delete,
}

/// A single operation in a batch index job
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchIndexOperation {
    /// Node ID to index
    pub node_id: String,
    /// Operation type (add/update or delete)
    pub operation: IndexOperation,
}
