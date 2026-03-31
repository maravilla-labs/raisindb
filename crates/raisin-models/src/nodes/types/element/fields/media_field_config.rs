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

//! Field configuration for media fields.
//!
//! This struct defines the configuration options for media fields in RaisinDB block schemas.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Configuration for a media field (e.g., image, video).
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, JsonSchema)]
pub struct MediaFieldConfig {
    /// Allowed media types (e.g., ["image", "video"]).
    pub allowed_types: Option<Vec<String>>,
}
