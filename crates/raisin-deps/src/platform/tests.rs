// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Tests for platform detection.

use super::*;

#[test]
fn test_platform_detection() {
    let platform = Platform::detect();
    // Just verify it doesn't panic and returns a valid variant
    match platform {
        Platform::MacOS => assert_eq!(platform.display_name(), "macOS"),
        Platform::Windows => assert_eq!(platform.display_name(), "Windows"),
        Platform::Linux(_) => assert!(platform.display_name().starts_with("Linux")),
        Platform::Unknown => assert_eq!(platform.display_name(), "Unknown OS"),
    }
}

#[test]
fn test_linux_distro_from_id() {
    assert_eq!(
        LinuxDistro::from_os_release_id("ubuntu"),
        LinuxDistro::Debian
    );
    assert_eq!(
        LinuxDistro::from_os_release_id("fedora"),
        LinuxDistro::Fedora
    );
    assert_eq!(LinuxDistro::from_os_release_id("arch"), LinuxDistro::Arch);
    assert_eq!(
        LinuxDistro::from_os_release_id("alpine"),
        LinuxDistro::Alpine
    );
    assert_eq!(
        LinuxDistro::from_os_release_id("unknown_distro"),
        LinuxDistro::Unknown
    );
}

#[test]
fn test_package_manager_detection() {
    let debian = LinuxDistro::Debian;
    assert!(debian.has_package_manager("apt"));
    assert!(!debian.has_package_manager("pacman"));

    let arch = LinuxDistro::Arch;
    assert!(arch.has_package_manager("pacman"));
    assert!(!arch.has_package_manager("apt"));
}
