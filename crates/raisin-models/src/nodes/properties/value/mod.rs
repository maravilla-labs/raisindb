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

//! PropertyValue and related types.
//!
//! This module is split into submodules for maintainability:
//! - `property_value` - The core `PropertyValue` enum and `DateTimeTimestamp` alias
//! - `domain_types` - `RaisinReference`, `RaisinUrl`, `Resource`
//! - `element` - `Element`, `Composite` (with custom serde)
//! - `geojson` - `GeoJson` geometry types

mod domain_types;
mod element;
mod geojson;
mod property_value;

#[cfg(test)]
mod tests;

// Re-export all public types to preserve the existing public API
pub use domain_types::{RaisinReference, RaisinUrl, Resource};
pub use element::{Composite, Element};
pub use geojson::GeoJson;
pub use property_value::{DateTimeTimestamp, PropertyValue};
