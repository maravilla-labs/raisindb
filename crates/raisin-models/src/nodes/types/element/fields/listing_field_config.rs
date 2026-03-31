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

//! Field configuration for listing fields.
//!
//! This struct defines the configuration options for listing fields in RaisinDB block schemas.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Configuration for a listing field (references multiple entries).
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct ListingFieldConfig {
    /// Types of referenced entries (optional).
    pub allowed_entry_types: Option<Vec<String>>,
    /// Field to sort by (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_by: Option<String>,
    /// Ascending or descending (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_order: Option<String>,
    /// Maximum number of entries to show (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}
