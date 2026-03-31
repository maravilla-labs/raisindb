// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Tests for the dependency manager.

use super::*;
use crate::checker::{DependencyChecker, DependencyStatus, InstallInstructions};
use crate::Platform;

struct MockChecker {
    name: &'static str,
    installed: bool,
}

impl DependencyChecker for MockChecker {
    fn name(&self) -> &str {
        self.name
    }

    fn display_name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        "Mock dependency"
    }

    fn check(&self) -> DependencyStatus {
        if self.installed {
            DependencyStatus::Installed {
                version: "1.0.0".to_string(),
                path: std::path::PathBuf::from("/usr/bin/mock"),
            }
        } else {
            DependencyStatus::NotInstalled
        }
    }

    fn install_instructions(&self) -> InstallInstructions {
        InstallInstructions {
            platform: Platform::Unknown,
            package_manager: None,
            command: "echo install".to_string(),
            needs_sudo: false,
            post_install: None,
            manual_url: None,
            provides: vec![],
        }
    }
}

#[test]
fn test_manager_check_all() {
    let manager = ExternalDependencyManager::new()
        .register(MockChecker {
            name: "installed",
            installed: true,
        })
        .register(MockChecker {
            name: "missing",
            installed: false,
        });

    let results = manager.check_all();
    assert_eq!(results.len(), 2);

    let installed = results.iter().find(|r| r.name == "installed").unwrap();
    assert!(installed.status.is_installed());

    let missing = results.iter().find(|r| r.name == "missing").unwrap();
    assert!(!missing.status.is_installed());
}

#[test]
fn test_manager_check_single() {
    let manager = ExternalDependencyManager::new().register(MockChecker {
        name: "test",
        installed: true,
    });

    let result = manager.check("test");
    assert!(result.is_some());
    assert!(result.unwrap().status.is_installed());

    let result = manager.check("nonexistent");
    assert!(result.is_none());
}
