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

//! Field configuration for options fields.
//!
//! This struct defines the configuration options for options fields (dropdown, radio, etc.) in RaisinDB block schemas.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Configuration for an options field.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct OptionsFieldConfig {
    /// Available options for selection.
    pub options: Vec<String>,
    /// How options are rendered (dropdown, radio, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub render_as: Option<OptionsRenderType>,
    /// Allow multiple selections (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multi_select: Option<bool>,
}

/// How options are rendered in the UI.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub enum OptionsRenderType {
    Dropdown,
    Radio,
    Checkbox,
}
