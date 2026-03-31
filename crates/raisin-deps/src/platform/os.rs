// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Platform detection.

use super::linux::LinuxDistro;
use super::utils::command_exists;

/// Target platform for installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Platform {
    /// macOS (Darwin)
    MacOS,
    /// Linux with specific distribution
    Linux(LinuxDistro),
    /// Windows
    Windows,
    /// Unknown platform
    #[default]
    Unknown,
}

impl Platform {
    /// Detect the current platform.
    pub fn detect() -> Self {
        #[cfg(target_os = "macos")]
        {
            Platform::MacOS
        }

        #[cfg(target_os = "windows")]
        {
            Platform::Windows
        }

        #[cfg(target_os = "linux")]
        {
            Platform::Linux(LinuxDistro::detect())
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            Platform::Unknown
        }
    }

    /// Check if a specific package manager is available on this platform.
    pub fn has_package_manager(&self, pm: &str) -> bool {
        match self {
            Platform::MacOS => matches!(pm, "brew" | "port"),
            Platform::Windows => matches!(pm, "winget" | "choco" | "scoop"),
            Platform::Linux(distro) => distro.has_package_manager(pm),
            Platform::Unknown => false,
        }
    }

    /// Get the primary package manager for this platform.
    pub fn primary_package_manager(&self) -> Option<&'static str> {
        match self {
            Platform::MacOS => {
                // Check if brew is available
                if command_exists("brew") {
                    Some("brew")
                } else {
                    None
                }
            }
            Platform::Windows => {
                // Prefer winget, fall back to choco
                if command_exists("winget") {
                    Some("winget")
                } else if command_exists("choco") {
                    Some("choco")
                } else {
                    None
                }
            }
            Platform::Linux(distro) => distro.primary_package_manager(),
            Platform::Unknown => None,
        }
    }

    /// Human-readable name for this platform.
    pub fn display_name(&self) -> &'static str {
        match self {
            Platform::MacOS => "macOS",
            Platform::Windows => "Windows",
            Platform::Linux(distro) => distro.display_name(),
            Platform::Unknown => "Unknown OS",
        }
    }
}
