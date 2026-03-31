// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Wrapper code generators for different runtime languages
//!
//! These generators create the high-level API wrapper code that users
//! interact with. The wrapper code calls the internal bindings registered
//! by the adapters.

pub mod javascript;
pub mod python;
