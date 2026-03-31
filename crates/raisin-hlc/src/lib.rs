// SPDX-License-Identifier: BSL-1.1

//! Hybrid Logical Clock (HLC) implementation for RaisinDB
//!
//! This crate provides a lock-free implementation of Hybrid Logical Clocks for distributed
//! timestamp generation in RaisinDB. HLC combines physical wall-clock time with logical counters
//! to provide causally-consistent timestamps across distributed nodes.
//!
//! # Key Features
//!
//! - Lock-free timestamp generation using atomic operations
//! - Descending lexicographic encoding for RocksDB range scans
//! - Full ordering semantics (implements `Ord`)
//! - String serialization for human readability and APIs
//! - Binary serialization for efficient storage
//!
//! # Example
//!
//! ```
//! use raisin_hlc::{HLC, NodeHLCState};
//!
//! // Create HLC state for a node
//! let state = NodeHLCState::new("node-1".to_string());
//!
//! // Generate monotonic timestamps
//! let hlc1 = state.tick();
//! let hlc2 = state.tick();
//! assert!(hlc2 > hlc1);
//!
//! // Update from remote timestamp
//! let remote = HLC::new(hlc1.timestamp_ms + 1000, 0);
//! let hlc3 = state.update(&remote);
//! assert!(hlc3 >= remote);
//! ```

mod error;
mod state;

pub use error::{HLCError, Result};
pub use state::NodeHLCState;

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

/// Hybrid Logical Clock timestamp combining physical time and logical counter
///
/// HLC provides a total ordering of events across distributed systems by combining:
/// - `timestamp_ms`: Physical wall-clock time in milliseconds since UNIX epoch
/// - `counter`: Logical counter for events within the same millisecond
///
/// # Ordering
///
/// HLCs are ordered first by timestamp, then by counter:
/// ```
/// # use raisin_hlc::HLC;
/// let hlc1 = HLC::new(1000, 0);
/// let hlc2 = HLC::new(1000, 1);
/// let hlc3 = HLC::new(1001, 0);
///
/// assert!(hlc1 < hlc2);
/// assert!(hlc2 < hlc3);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, schemars::JsonSchema)]
pub struct HLC {
    /// Physical wall-clock time in milliseconds since UNIX epoch
    pub timestamp_ms: u64,
    /// Logical counter for same-millisecond events
    pub counter: u64,
}

impl HLC {
    /// Creates a new HLC with the given timestamp and counter
    ///
    /// # Example
    ///
    /// ```
    /// # use raisin_hlc::HLC;
    /// let hlc = HLC::new(1705843009213693952, 42);
    /// assert_eq!(hlc.timestamp_ms, 1705843009213693952);
    /// assert_eq!(hlc.counter, 42);
    /// ```
    #[inline]
    pub const fn new(timestamp_ms: u64, counter: u64) -> Self {
        Self {
            timestamp_ms,
            counter,
        }
    }

    /// Creates an HLC from the current system time
    ///
    /// Counter is initialized to 0. This is primarily useful for initial state.
    ///
    /// # Example
    ///
    /// ```
    /// # use raisin_hlc::HLC;
    /// let hlc = HLC::now();
    /// assert!(hlc.timestamp_ms > 0);
    /// assert_eq!(hlc.counter, 0);
    /// ```
    pub fn now() -> Self {
        Self::new(current_time_ms(), 0)
    }

    /// Encodes HLC into 16-byte descending lexicographic order
    ///
    /// Uses bitwise NOT to achieve descending order - newer timestamps sort
    /// lexicographically before older ones. This is critical for RocksDB range
    /// scans where we want to iterate from newest to oldest.
    ///
    /// # Format
    ///
    /// - Bytes 0-7: NOT(timestamp_ms) in big-endian
    /// - Bytes 8-15: NOT(counter) in big-endian
    ///
    /// # Example
    ///
    /// ```
    /// # use raisin_hlc::HLC;
    /// let hlc1 = HLC::new(1000, 0);
    /// let hlc2 = HLC::new(2000, 0);
    ///
    /// let bytes1 = hlc1.encode_descending();
    /// let bytes2 = hlc2.encode_descending();
    ///
    /// // Newer timestamp sorts lexicographically first
    /// assert!(bytes2 < bytes1);
    /// ```
    #[inline]
    pub fn encode_descending(&self) -> [u8; 16] {
        let mut buf = [0u8; 16];

        // Encode timestamp_ms with bitwise NOT for descending order
        let timestamp_bytes = (!self.timestamp_ms).to_be_bytes();
        buf[0..8].copy_from_slice(&timestamp_bytes);

        // Encode counter with bitwise NOT for descending order
        let counter_bytes = (!self.counter).to_be_bytes();
        buf[8..16].copy_from_slice(&counter_bytes);

        buf
    }

    /// Decodes HLC from 16-byte descending lexicographic encoding
    ///
    /// Reverses the bitwise NOT operation applied during encoding.
    ///
    /// # Errors
    ///
    /// Returns `HLCError::InvalidEncoding` if the input is not exactly 16 bytes.
    ///
    /// # Example
    ///
    /// ```
    /// # use raisin_hlc::HLC;
    /// let original = HLC::new(1705843009213693952, 42);
    /// let encoded = original.encode_descending();
    /// let decoded = HLC::decode_descending(&encoded).unwrap();
    /// assert_eq!(original, decoded);
    /// ```
    pub fn decode_descending(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != 16 {
            return Err(HLCError::InvalidEncoding {
                expected: 16,
                actual: bytes.len(),
            });
        }

        // Decode timestamp_ms (reverse bitwise NOT)
        let mut timestamp_bytes = [0u8; 8];
        timestamp_bytes.copy_from_slice(&bytes[0..8]);
        let timestamp_ms = !u64::from_be_bytes(timestamp_bytes);

        // Decode counter (reverse bitwise NOT)
        let mut counter_bytes = [0u8; 8];
        counter_bytes.copy_from_slice(&bytes[8..16]);
        let counter = !u64::from_be_bytes(counter_bytes);

        Ok(Self::new(timestamp_ms, counter))
    }

    /// Returns the HLC as a u128 for compact numeric representation
    ///
    /// Upper 64 bits contain timestamp_ms, lower 64 bits contain counter.
    /// This is useful for compact storage and comparison.
    ///
    /// # Example
    ///
    /// ```
    /// # use raisin_hlc::HLC;
    /// let hlc = HLC::new(1000, 42);
    /// let numeric = hlc.as_u128();
    /// assert_eq!(numeric >> 64, 1000);
    /// assert_eq!(numeric & 0xFFFFFFFFFFFFFFFF, 42);
    /// ```
    #[inline]
    pub const fn as_u128(&self) -> u128 {
        ((self.timestamp_ms as u128) << 64) | (self.counter as u128)
    }

    /// Creates an HLC from a u128 numeric representation
    ///
    /// Inverse of `as_u128()`.
    ///
    /// # Example
    ///
    /// ```
    /// # use raisin_hlc::HLC;
    /// let original = HLC::new(1000, 42);
    /// let numeric = original.as_u128();
    /// let decoded = HLC::from_u128(numeric);
    /// assert_eq!(original, decoded);
    /// ```
    #[inline]
    pub const fn from_u128(value: u128) -> Self {
        let timestamp_ms = (value >> 64) as u64;
        let counter = (value & 0xFFFFFFFFFFFFFFFF) as u64;
        Self::new(timestamp_ms, counter)
    }
}

/// Ordering implementation: timestamp-first, then counter
///
/// This ensures HLCs form a total order across all nodes.
impl Ord for HLC {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        self.timestamp_ms
            .cmp(&other.timestamp_ms)
            .then_with(|| self.counter.cmp(&other.counter))
    }
}

impl PartialOrd for HLC {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Display format: "timestamp-counter"
///
/// Human-readable format suitable for APIs and logging.
///
/// # Example
///
/// ```
/// # use raisin_hlc::HLC;
/// let hlc = HLC::new(1705843009213693952, 42);
/// assert_eq!(hlc.to_string(), "1705843009213693952-42");
/// ```
impl fmt::Display for HLC {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.timestamp_ms, self.counter)
    }
}

/// Parse from "timestamp-counter" format
///
/// # Errors
///
/// Returns `HLCError::ParseError` if the format is invalid or components
/// cannot be parsed as u64.
///
/// # Example
///
/// ```
/// # use raisin_hlc::HLC;
/// let hlc: HLC = "1705843009213693952-42".parse().unwrap();
/// assert_eq!(hlc.timestamp_ms, 1705843009213693952);
/// assert_eq!(hlc.counter, 42);
/// ```
impl FromStr for HLC {
    type Err = HLCError;

    fn from_str(s: &str) -> Result<Self> {
        let parts: Vec<&str> = s.split('-').collect();
        if parts.len() != 2 {
            return Err(HLCError::ParseError {
                input: s.to_string(),
                reason: "expected format 'timestamp-counter'".to_string(),
            });
        }

        let timestamp_ms = parts[0].parse::<u64>().map_err(|e| HLCError::ParseError {
            input: s.to_string(),
            reason: format!("invalid timestamp: {}", e),
        })?;

        let counter = parts[1].parse::<u64>().map_err(|e| HLCError::ParseError {
            input: s.to_string(),
            reason: format!("invalid counter: {}", e),
        })?;

        Ok(Self::new(timestamp_ms, counter))
    }
}

/// Custom Serialize implementation - outputs HLC as string "timestamp-counter"
///
/// This provides a compact, human-readable representation in JSON APIs.
///
/// # Example
///
/// ```
/// # use raisin_hlc::HLC;
/// let hlc = HLC::new(1705584000000, 5);
/// let json = serde_json::to_string(&hlc).unwrap();
/// assert_eq!(json, "\"1705584000000-5\"");
/// ```
impl serde::Serialize for HLC {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&format!("{}-{}", self.timestamp_ms, self.counter))
    }
}

/// Custom Deserialize implementation - parses HLC from string "timestamp-counter"
///
/// # Example
///
/// ```
/// # use raisin_hlc::HLC;
/// let hlc: HLC = serde_json::from_str("\"1705584000000-5\"").unwrap();
/// assert_eq!(hlc.timestamp_ms, 1705584000000);
/// assert_eq!(hlc.counter, 5);
/// ```
impl<'de> serde::Deserialize<'de> for HLC {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Get current system time in milliseconds since UNIX epoch
#[inline]
fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time before UNIX epoch")
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hlc_creation() {
        let hlc = HLC::new(1000, 42);
        assert_eq!(hlc.timestamp_ms, 1000);
        assert_eq!(hlc.counter, 42);
    }

    #[test]
    fn test_hlc_now() {
        let hlc = HLC::now();
        assert!(hlc.timestamp_ms > 0);
        assert_eq!(hlc.counter, 0);
    }

    #[test]
    fn test_hlc_ordering() {
        let hlc1 = HLC::new(1000, 0);
        let hlc2 = HLC::new(1000, 1);
        let hlc3 = HLC::new(1001, 0);

        assert!(hlc1 < hlc2);
        assert!(hlc2 < hlc3);
        assert!(hlc1 < hlc3);

        // Test equality
        let hlc4 = HLC::new(1000, 0);
        assert_eq!(hlc1, hlc4);
        assert!(!(hlc1 < hlc4));
        assert!(!(hlc1 > hlc4));
    }

    #[test]
    fn test_encode_decode_roundtrip() {
        let test_cases = vec![
            HLC::new(0, 0),
            HLC::new(1, 0),
            HLC::new(0, 1),
            HLC::new(u64::MAX, u64::MAX),
            HLC::new(1705843009213693952, 42),
            HLC::new(1000, 999),
        ];

        for original in test_cases {
            let encoded = original.encode_descending();
            let decoded = HLC::decode_descending(&encoded).unwrap();
            assert_eq!(original, decoded, "roundtrip failed for {:?}", original);
        }
    }

    #[test]
    fn test_descending_order_encoding() {
        let hlc1 = HLC::new(1000, 0);
        let hlc2 = HLC::new(2000, 0);
        let hlc3 = HLC::new(2000, 1);

        let bytes1 = hlc1.encode_descending();
        let bytes2 = hlc2.encode_descending();
        let bytes3 = hlc3.encode_descending();

        // Newer timestamps should sort lexicographically first
        assert!(
            bytes2 < bytes1,
            "newer timestamp should be smaller in bytes"
        );
        assert!(bytes3 < bytes2, "higher counter should be smaller in bytes");
    }

    #[test]
    fn test_decode_invalid_length() {
        let result = HLC::decode_descending(&[0u8; 15]);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            HLCError::InvalidEncoding { .. }
        ));

        let result = HLC::decode_descending(&[0u8; 17]);
        assert!(result.is_err());
    }

    #[test]
    fn test_display_format() {
        let hlc = HLC::new(1705843009213693952, 42);
        assert_eq!(hlc.to_string(), "1705843009213693952-42");
    }

    #[test]
    fn test_from_str() {
        let hlc: HLC = "1705843009213693952-42".parse().unwrap();
        assert_eq!(hlc.timestamp_ms, 1705843009213693952);
        assert_eq!(hlc.counter, 42);
    }

    #[test]
    fn test_from_str_invalid() {
        assert!("invalid".parse::<HLC>().is_err());
        assert!("1000".parse::<HLC>().is_err());
        assert!("1000-".parse::<HLC>().is_err());
        assert!("-42".parse::<HLC>().is_err());
        assert!("1000-42-extra".parse::<HLC>().is_err());
        assert!("abc-def".parse::<HLC>().is_err());
    }

    #[test]
    fn test_as_u128_roundtrip() {
        let test_cases = vec![
            HLC::new(0, 0),
            HLC::new(1000, 42),
            HLC::new(u64::MAX, u64::MAX),
            HLC::new(1705843009213693952, 123456789),
        ];

        for original in test_cases {
            let numeric = original.as_u128();
            let decoded = HLC::from_u128(numeric);
            assert_eq!(original, decoded);
        }
    }

    #[test]
    fn test_serde_json() {
        let hlc = HLC::new(1705843009213693952, 42);

        // Test serialization to string format
        let json = serde_json::to_string(&hlc).unwrap();
        assert_eq!(json, "\"1705843009213693952-42\"");

        // Test deserialization from string format
        let decoded: HLC = serde_json::from_str(&json).unwrap();
        assert_eq!(hlc, decoded);

        // Test direct deserialization
        let hlc2: HLC = serde_json::from_str("\"1705584000000-5\"").unwrap();
        assert_eq!(hlc2.timestamp_ms, 1705584000000);
        assert_eq!(hlc2.counter, 5);
    }
}
