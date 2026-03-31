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

//! Job category definitions for pool isolation

use serde::{Deserialize, Serialize};
use std::fmt;

/// Category of a job, determining which worker pool handles it.
///
/// Jobs are categorized into three pools to prevent cross-category starvation:
/// - **Realtime**: User-facing operations that must be responsive
/// - **Background**: Long-running operations that can tolerate delays
/// - **System**: Administrative and maintenance tasks
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobCategory {
    /// Triggers, functions, AI, flows — user-facing operations
    Realtime,
    /// Indexing, embedding, replication, maintenance — background work
    Background,
    /// Auth, packages, cleanup, scheduled checks — system tasks
    System,
}

impl fmt::Display for JobCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JobCategory::Realtime => write!(f, "realtime"),
            JobCategory::Background => write!(f, "background"),
            JobCategory::System => write!(f, "system"),
        }
    }
}
