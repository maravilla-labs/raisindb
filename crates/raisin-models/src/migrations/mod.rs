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

//! Data migration utilities for handling schema evolution
//!
//! This module provides lenient deserializers that can handle legacy data
//! with type mismatches (e.g., boolean false stored where strings are expected).
//!
//! These deserializers work with both JSON and MessagePack formats by implementing
//! the visitor pattern directly, allowing them to intercept type mismatches before
//! deserialization fails.

mod datetime_lenient;
mod string_lenient;
mod vec_lenient;

#[cfg(test)]
mod tests;

pub use datetime_lenient::*;
pub use string_lenient::*;
pub use vec_lenient::*;
