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

//! Helper functions and utilities for working with translations.
//!
//! This module provides convenience functions and utilities that simplify
//! common translation operations:
//!
//! - **Locale resolution**: Find best matching locale with fallback
//! - **Pointer manipulation**: Common JsonPointer operations
//! - **Validation helpers**: Check if fields are translatable
//! - **Merging utilities**: Combine base content with overlays

mod locale_resolution;
mod overlay_ops;

#[cfg(test)]
mod tests;

pub use locale_resolution::*;
pub use overlay_ops::*;
