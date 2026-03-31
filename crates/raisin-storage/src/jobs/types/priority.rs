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

//! Job priority definitions

use serde::{Deserialize, Serialize};
use std::fmt;

/// Priority level for job execution
///
/// Jobs with higher priority are processed before lower priority jobs.
/// This ensures user-facing operations (triggers, functions) are not blocked
/// by background maintenance tasks (indexing, cleanup).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum JobPriority {
    /// User-facing operations: triggers, function execution, flows
    /// These directly impact user experience and should be processed first.
    High = 0,
    /// Standard operations: scheduled checks, package operations
    #[default]
    Normal = 1,
    /// Background operations: indexing, cleanup, compaction
    /// These can be delayed without impacting user experience.
    Low = 2,
}

impl fmt::Display for JobPriority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JobPriority::High => write!(f, "high"),
            JobPriority::Normal => write!(f, "normal"),
            JobPriority::Low => write!(f, "low"),
        }
    }
}
