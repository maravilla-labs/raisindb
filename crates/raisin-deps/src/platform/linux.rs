// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Linux distribution detection.

use super::utils::command_exists;
use std::path::Path;

/// Linux distribution variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinuxDistro {
    /// Debian-based: Debian, Ubuntu, Linux Mint, Pop!_OS
    Debian,
    /// Red Hat-based: Fedora, RHEL, CentOS, Rocky Linux, AlmaLinux
    Fedora,
    /// Arch-based: Arch Linux, Manjaro, EndeavourOS
    Arch,
    /// Alpine Linux (musl-based)
    Alpine,
    /// openSUSE / SUSE Linux
    OpenSuse,
    /// Unknown Linux distribution
    Unknown,
}

impl LinuxDistro {
    /// Detect the Linux distribution.
    pub fn detect() -> Self {
        // Try reading /etc/os-release first (modern standard)
        if let Some(distro) = Self::detect_from_os_release() {
            return distro;
        }

        // Fall back to checking distro-specific files
        Self::detect_from_release_files()
    }

    /// Detect distro from /etc/os-release file.
    fn detect_from_os_release() -> Option<Self> {
        let content = std::fs::read_to_string("/etc/os-release").ok()?;

        // Look for ID= line
        for line in content.lines() {
            if let Some(id) = line.strip_prefix("ID=") {
                let id = id.trim_matches('"').to_lowercase();
                return Some(Self::from_os_release_id(&id));
            }
        }

        // Look for ID_LIKE= line (for derivatives)
        for line in content.lines() {
            if let Some(id_like) = line.strip_prefix("ID_LIKE=") {
                let id_like = id_like.trim_matches('"').to_lowercase();
                // ID_LIKE can contain multiple values, space-separated
                for id in id_like.split_whitespace() {
                    let distro = Self::from_os_release_id(id);
                    if distro != LinuxDistro::Unknown {
                        return Some(distro);
                    }
                }
            }
        }

        None
    }

    /// Detect distro from release files.
    fn detect_from_release_files() -> Self {
        if Path::new("/etc/debian_version").exists() {
            return LinuxDistro::Debian;
        }
        if Path::new("/etc/fedora-release").exists() {
            return LinuxDistro::Fedora;
        }
        if Path::new("/etc/redhat-release").exists() {
            return LinuxDistro::Fedora; // RHEL/CentOS
        }
        if Path::new("/etc/arch-release").exists() {
            return LinuxDistro::Arch;
        }
        if Path::new("/etc/alpine-release").exists() {
            return LinuxDistro::Alpine;
        }
        if Path::new("/etc/SuSE-release").exists() {
            return LinuxDistro::OpenSuse;
        }

        LinuxDistro::Unknown
    }

    /// Map os-release ID to distro enum.
    pub(super) fn from_os_release_id(id: &str) -> Self {
        match id {
            // Debian family
            "debian" | "ubuntu" | "linuxmint" | "pop" | "elementary" | "zorin" | "kali"
            | "raspbian" | "neon" => LinuxDistro::Debian,

            // Red Hat family
            "fedora" | "rhel" | "centos" | "rocky" | "almalinux" | "oracle" | "scientific"
            | "amzn" | "amazon" => LinuxDistro::Fedora,

            // Arch family
            "arch" | "manjaro" | "endeavouros" | "artix" | "garuda" => LinuxDistro::Arch,

            // Alpine
            "alpine" => LinuxDistro::Alpine,

            // openSUSE family
            "opensuse" | "opensuse-leap" | "opensuse-tumbleweed" | "suse" | "sles" => {
                LinuxDistro::OpenSuse
            }

            _ => LinuxDistro::Unknown,
        }
    }

    /// Check if a package manager is available for this distro.
    pub fn has_package_manager(&self, pm: &str) -> bool {
        match self {
            LinuxDistro::Debian => matches!(pm, "apt" | "apt-get" | "dpkg"),
            LinuxDistro::Fedora => matches!(pm, "dnf" | "yum" | "rpm"),
            LinuxDistro::Arch => matches!(pm, "pacman" | "yay" | "paru"),
            LinuxDistro::Alpine => matches!(pm, "apk"),
            LinuxDistro::OpenSuse => matches!(pm, "zypper" | "rpm"),
            LinuxDistro::Unknown => false,
        }
    }

    /// Get the primary package manager for this distro.
    pub fn primary_package_manager(&self) -> Option<&'static str> {
        match self {
            LinuxDistro::Debian => Some("apt"),
            LinuxDistro::Fedora => {
                // Prefer dnf over yum
                if command_exists("dnf") {
                    Some("dnf")
                } else {
                    Some("yum")
                }
            }
            LinuxDistro::Arch => Some("pacman"),
            LinuxDistro::Alpine => Some("apk"),
            LinuxDistro::OpenSuse => Some("zypper"),
            LinuxDistro::Unknown => None,
        }
    }

    /// Human-readable name for this distro.
    pub fn display_name(&self) -> &'static str {
        match self {
            LinuxDistro::Debian => "Linux (Debian/Ubuntu)",
            LinuxDistro::Fedora => "Linux (Fedora/RHEL)",
            LinuxDistro::Arch => "Linux (Arch)",
            LinuxDistro::Alpine => "Linux (Alpine)",
            LinuxDistro::OpenSuse => "Linux (openSUSE)",
            LinuxDistro::Unknown => "Linux",
        }
    }
}
