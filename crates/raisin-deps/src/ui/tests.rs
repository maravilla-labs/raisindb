// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Tests for UI module.

use super::*;

#[test]
fn test_install_action() {
    assert_eq!(InstallAction::RunCommand, InstallAction::RunCommand);
    assert_ne!(InstallAction::Skip, InstallAction::RunCommand);
}
