// SPDX-License-Identifier: BSL-1.1

//! Error types for HLC operations

use thiserror::Error;

/// Result type alias for HLC operations
pub type Result<T> = std::result::Result<T, HLCError>;

/// Errors that can occur during HLC operations
#[derive(Debug, Error)]
pub enum HLCError {
    /// Invalid encoding length
    #[error("Invalid HLC encoding: expected {expected} bytes, got {actual} bytes")]
    InvalidEncoding { expected: usize, actual: usize },

    /// Failed to parse HLC from string
    #[error("Failed to parse HLC from '{input}': {reason}")]
    ParseError { input: String, reason: String },

    /// Clock skew detected (time went backwards)
    #[error("Clock skew detected: wall clock {wall_clock_ms}ms is behind HLC timestamp {hlc_timestamp_ms}ms (delta: {delta_ms}ms)")]
    ClockSkew {
        wall_clock_ms: u64,
        hlc_timestamp_ms: u64,
        delta_ms: u64,
    },

    /// Storage error during persistence operations
    #[error("HLC persistence error: {0}")]
    PersistenceError(String),
}
