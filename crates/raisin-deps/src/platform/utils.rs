// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Utility functions for command execution.

use std::process::Command;

/// Check if a command exists in PATH.
pub fn command_exists(cmd: &str) -> bool {
    which::which(cmd).is_ok()
}

/// Run a command and capture its output.
pub fn run_command(cmd: &str, args: &[&str]) -> Result<String, std::io::Error> {
    let output = Command::new(cmd).args(args).output()?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(std::io::Error::other(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ))
    }
}
