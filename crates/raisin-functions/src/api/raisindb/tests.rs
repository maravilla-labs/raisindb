// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Tests for RaisinFunctionApi

use super::RaisinFunctionApi;

#[test]
fn test_glob_match() {
    // Test simple wildcard
    assert!(RaisinFunctionApi::glob_match(
        "https://api.example.com/*",
        "https://api.example.com/users"
    ));

    // Test double wildcard
    assert!(RaisinFunctionApi::glob_match(
        "https://api.example.com/**",
        "https://api.example.com/users/123/profile"
    ));

    // Test exact match
    assert!(RaisinFunctionApi::glob_match(
        "https://api.example.com/users",
        "https://api.example.com/users"
    ));

    // Test no match
    assert!(!RaisinFunctionApi::glob_match(
        "https://api.example.com/*",
        "https://api.other.com/users"
    ));
}
