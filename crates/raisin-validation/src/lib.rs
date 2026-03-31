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

//! Shared validation logic for RaisinDB
//!
//! This crate provides field helpers, inheritance resolution, and schema validation
//! that can be used by both CLI WASM and Server.
//!
//! # Overview
//!
//! The `raisin-validation` crate centralizes validation logic that was previously
//! duplicated across CLI and server implementations. It provides:
//!
//! - **Field Helpers**: Utilities for extracting information from `FieldSchema` variants
//! - **Inheritance Resolution**: Trait-based system for resolving type hierarchies
//! - **Schema Validation**: Field-level and type-level validation with structured errors
//! - **Error Reporting**: Consistent error codes and severity levels
//!
//! # Examples
//!
//! ```rust,ignore
//! use raisin_validation::{field_helpers, validate_fields, ValidationError};
//!
//! // Check if a field is required
//! let is_required = field_helpers::is_required(&field);
//!
//! // Validate field values
//! let errors = Vec::new();
//! validate_fields(&fields, &values, "element.fields", |err| {
//!     errors.push(err);
//! });
//! ```

pub mod errors;
pub mod field_helpers;
pub mod inheritance;
pub mod schema;

pub use errors::*;
pub use field_helpers::*;
pub use inheritance::*;
pub use schema::*;
