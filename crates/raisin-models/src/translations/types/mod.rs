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

//! Core type definitions for the translation system.
//!
//! This module contains the fundamental types used throughout the translation system:
//! - [`LocaleOverlay`]: Per-locale translation data overlays
//! - [`JsonPointer`]: RFC 6901 compliant paths for field addressing
//! - [`LocaleCode`]: Validated BCP 47 language tags

mod json_pointer;
mod locale_code;
mod locale_overlay;

pub use json_pointer::JsonPointer;
pub use locale_code::LocaleCode;
pub use locale_overlay::LocaleOverlay;
