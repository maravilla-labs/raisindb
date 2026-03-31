// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Interactive CLI UI for dependency setup.
//!
//! This module provides:
//! - Interactive CLI prompts for guiding users through dependency installation
//! - API response types for the admin console

mod api_types;
mod cli;

pub use api_types::{ApiInstallInstructions, ApiStatus, DependencyApiStatus};
pub use cli::{DependencySetupUI, InstallAction};

#[cfg(test)]
mod tests;
