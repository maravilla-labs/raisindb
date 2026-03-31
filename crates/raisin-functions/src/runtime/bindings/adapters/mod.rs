// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Runtime adapters that translate shared bindings to runtime-specific code
//!
//! Each adapter takes the shared bindings registry and generates the appropriate
//! bindings for its target runtime (QuickJS, Starlark, etc.)

pub mod quickjs;
pub mod starlark;
