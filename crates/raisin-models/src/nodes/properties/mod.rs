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

#[allow(clippy::module_inception)]
pub mod properties;
pub mod references;
pub mod schema;
pub mod search;
pub mod spatial_policy;
pub mod utils;
pub mod value;

pub use properties::*;
pub use references::*;
pub use schema::{
    CompoundColumnType, CompoundIndexColumn, CompoundIndexDefinition, IndexType, PropertyType,
    PropertyValueSchema,
};
pub use spatial_policy::{
    is_wildcard_property_path, policy_key_for_path, resolve_spatial_policy, sorted_precisions,
    SpatialCoverMode, SpatialPolicy, SpatialPropertySchema, SpatialWorkspaceSchema,
    INDEX_PRECISIONS_DEFAULT, MAX_COVER_CELLS, MAX_SCAN_CELLS, PRECISION_RANGE,
    SPATIAL_NORMALIZER_VERSION,
};
pub use utils::*;
pub use value::{GeoJson, Position, PropertyValue, RaisinReference};
