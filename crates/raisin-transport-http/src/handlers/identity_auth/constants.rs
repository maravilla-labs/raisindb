// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Authentication-related constants.

/// Default session duration in nanoseconds (24 hours)
pub const SESSION_DURATION_NANOS: i64 = 24 * 60 * 60 * 1_000_000_000;

/// Extended session duration in nanoseconds for "remember me" (30 days)
pub const REMEMBER_ME_DURATION_NANOS: i64 = 30 * 24 * 60 * 60 * 1_000_000_000;

/// Access token lifetime in seconds (1 hour)
pub const ACCESS_TOKEN_SECONDS: i64 = 3600;

/// Get the session duration based on the remember_me flag.
pub fn session_duration_nanos(remember_me: bool) -> i64 {
    if remember_me {
        REMEMBER_ME_DURATION_NANOS
    } else {
        SESSION_DURATION_NANOS
    }
}
