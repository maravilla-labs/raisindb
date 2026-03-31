// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Platform and Linux distribution detection.
//!
//! This module provides functionality for detecting the current operating
//! system and Linux distribution, as well as utility functions for
//! running commands and checking if executables exist.

mod linux;
mod os;
mod utils;

pub use linux::LinuxDistro;
pub use os::Platform;
pub use utils::{command_exists, run_command};

#[cfg(test)]
mod tests;
