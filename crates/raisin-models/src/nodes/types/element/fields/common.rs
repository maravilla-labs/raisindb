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

//! Common types for field configuration.
//!
//! This module defines enums and shared types used by multiple field config structs.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Mode for date fields (date, time, datetime, timerange).
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub enum DateMode {
    /// Date and time picker.
    DateTime,
    /// Date only picker.
    Date,
    /// Time only picker.
    Time,
    /// Time range picker.
    TimeRange,
}
