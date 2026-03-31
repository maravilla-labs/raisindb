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

use thiserror::Error;

#[derive(Error, Debug)]
pub enum GraphError {
    #[error("Node not found in projection: {0}")]
    NodeNotFound(String),

    #[error("Algorithm execution failed: {0}")]
    ExecutionError(String),

    #[error("Storage error: {0}")]
    Storage(String),
}

pub type Result<T> = std::result::Result<T, GraphError>;
