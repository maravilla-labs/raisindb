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

//! Job status definitions

use serde::{Deserialize, Serialize};

/// Status of a background job
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum JobStatus {
    /// Job is scheduled but not yet running
    Scheduled,
    /// Worker claimed job, about to spawn handler (~microseconds)
    Running,
    /// Handler task actively running (seconds to minutes)
    Executing,
    /// Job completed successfully
    Completed,
    /// Job was cancelled
    Cancelled,
    /// Job failed with an error
    Failed(String),
}
