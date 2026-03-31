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

//! Field configuration for number fields.
//!
//! This struct defines the configuration options for number fields in RaisinDB block schemas.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Configuration for a number field.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct NumberFieldConfig {
    /// True for integers, false for decimals (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_integer: Option<bool>,
    /// Minimum value (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_value: Option<f64>,
    /// Maximum value (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_value: Option<f64>,
}
