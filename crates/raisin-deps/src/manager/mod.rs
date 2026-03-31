// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! External dependency manager for checking and managing system dependencies.

mod core;
mod types;

pub use core::ExternalDependencyManager;
pub use types::{EnableResult, SetupError, SetupResult};

#[cfg(test)]
mod tests;
