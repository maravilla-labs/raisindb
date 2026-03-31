// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Index repository trait definitions.
//!
//! This module contains traits for various index types:
//! - [`ReferenceIndexRepository`] - For tracking PropertyValue::Reference relationships
//! - [`PropertyIndexRepository`] - For fast property-based lookups
//! - [`CompoundIndexRepository`] - For multi-column queries with ORDER BY

mod compound;
mod property;
mod reference;

pub use compound::{CompoundColumnValue, CompoundIndexRepository, CompoundIndexScanEntry};
pub use property::{PropertyIndexRepository, PropertyScanEntry};
pub use reference::ReferenceIndexRepository;
