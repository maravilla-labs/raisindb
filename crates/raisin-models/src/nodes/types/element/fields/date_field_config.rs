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

//! Field configuration for date fields.
//!
//! This struct defines the configuration options for date fields in RaisinDB block schemas.

use super::common::DateMode;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Configuration for a date field.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct DateFieldConfig {
    /// ISO 8601 or custom formats (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_format: Option<String>,
    /// Determines the picker type: datetime, date, time, timerange (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_mode: Option<DateMode>,
}
