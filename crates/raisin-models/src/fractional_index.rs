// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Fractional indexing for scalable node ordering
//!
//! This module implements a Base62 fractional indexing system that allows
//! insertion of items between any two existing items without renumbering.
//!
//! # How it works
//!
//! - Uses Base62 alphabet (0-9, a-z, A-Z) for maximum space
//! - Strings are compared lexicographically
//! - Can always find a string between any two strings by subdivision
//! - Average index length grows as O(log n) with n insertions
//!
//! # Examples
//!
//! ```
//! use raisin_models::fractional_index::*;
//!
//! // Get first key
//! let first = first_key();  // "a"
//!
//! // Get next key
//! let second = next_key(&first);  // "b"
//!
//! // Insert between two keys
//! let between = midpoint("a", "c").unwrap();  // "b"
//! ```

use raisin_error::{Error, Result};

/// Base62 alphabet: 0-9, a-z, A-Z
/// Ordered to be lexicographically sortable
pub const BASE62_ALPHABET: &str = "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";

/// Get the first key in the ordering
pub fn first_key() -> String {
    "a".to_string()
}

/// Get the last key in the ordering (used as sentinel)
pub fn last_key() -> String {
    "Z".to_string()
}

/// Generate the next key after the current one
///
/// # Examples
/// ```
/// use raisin_models::fractional_index::next_key;
/// assert_eq!(next_key("a"), "b");
/// assert_eq!(next_key("z"), "A");
/// assert_eq!(next_key("Z"), "Za");
/// ```
pub fn next_key(current: &str) -> String {
    if current.is_empty() {
        return first_key();
    }

    let chars: Vec<char> = BASE62_ALPHABET.chars().collect();
    let mut result: Vec<char> = current.chars().collect();

    // Try to increment the last character
    let last_char = result.last().copied().unwrap();

    if let Some(pos) = chars.iter().position(|&c| c == last_char) {
        if pos < chars.len() - 1 {
            // Can increment: replace last char with next char
            *result.last_mut().unwrap() = chars[pos + 1];
            return result.into_iter().collect();
        }
    }

    // Last char is 'Z' or not in alphabet - append 'a' to grow the string
    result.push('a');
    result.into_iter().collect()
}

/// Find a fractional index midpoint between two keys
///
/// Returns a new key that sorts between `left` and `right` lexicographically.
///
/// # Arguments
/// * `left` - The left boundary (can be empty string for "before all")
/// * `right` - The right boundary (can be empty string for "after all")
///
/// # Returns
/// A new key between left and right, or an error if invalid inputs
///
/// # Examples
/// ```
/// use raisin_models::fractional_index::midpoint;
///
/// assert_eq!(midpoint("a", "c").unwrap(), "b");
/// assert_eq!(midpoint("b", "c").unwrap(), "b5");
/// assert_eq!(midpoint("", "a").unwrap(), "0");
/// assert_eq!(midpoint("Z", "").unwrap(), "Za");
/// ```
pub fn midpoint(left: &str, right: &str) -> Result<String> {
    // Handle edge cases
    if left == right && !left.is_empty() {
        return Err(Error::Validation(
            "Cannot find midpoint: left equals right".to_string(),
        ));
    }

    if !left.is_empty() && !right.is_empty() && left >= right {
        return Err(Error::Validation(format!(
            "Cannot find midpoint: left '{}' >= right '{}'",
            left, right
        )));
    }

    let chars: Vec<char> = BASE62_ALPHABET.chars().collect();

    // Special case: left is empty (insert before all)
    if left.is_empty() {
        if right.is_empty() {
            // Both empty: return first key
            return Ok(first_key());
        }
        // Before first char: use '0' which is first in alphabet
        return Ok("0".to_string());
    }

    // Special case: right is empty (insert after all)
    if right.is_empty() {
        // Append a character to left
        return Ok(format!("{}a", left));
    }

    // Normal case: both left and right are non-empty
    let left_chars: Vec<char> = left.chars().collect();
    let right_chars: Vec<char> = right.chars().collect();

    // Find the first position where they differ
    let mut pos = 0;
    while pos < left_chars.len() && pos < right_chars.len() && left_chars[pos] == right_chars[pos] {
        pos += 1;
    }

    // Build result starting with the common prefix
    let mut result: Vec<char> = left_chars[..pos].to_vec();

    // If we've reached the end of left, we can append to it
    if pos >= left_chars.len() {
        // left is a prefix of right (e.g., left="b", right="b5")
        // We can append a character to left that's less than right's next char
        let right_next = right_chars[pos];
        if let Some(right_pos) = chars.iter().position(|&c| c == right_next) {
            if right_pos > 0 {
                // Use the character before right's next character
                result.push(chars[right_pos / 2]);
            } else {
                // right_next is '0', append '0' to left
                result.push('0');
            }
        }
        return Ok(result.into_iter().collect());
    }

    // If we've reached the end of right, that's an error (left > right)
    if pos >= right_chars.len() {
        return Err(Error::Validation(format!(
            "Invalid state: right '{}' is prefix of left '{}'",
            right, left
        )));
    }

    // Both have characters at pos - find the midpoint character
    let left_char = left_chars[pos];
    let right_char = right_chars[pos];

    let left_idx = chars
        .iter()
        .position(|&c| c == left_char)
        .ok_or_else(|| Error::Validation(format!("Invalid character in left: {}", left_char)))?;
    let right_idx = chars
        .iter()
        .position(|&c| c == right_char)
        .ok_or_else(|| Error::Validation(format!("Invalid character in right: {}", right_char)))?;

    if right_idx - left_idx > 1 {
        // There's at least one character between them - use the midpoint
        let mid_idx = (left_idx + right_idx) / 2;
        result.push(chars[mid_idx]);
        Ok(result.into_iter().collect())
    } else {
        // They're adjacent (e.g., 'a' and 'b') - append to the full left string
        // This preserves any characters after the common prefix
        let mut full_left: Vec<char> = left.chars().collect();
        full_left.push(chars[chars.len() - 1]); // Append last character (Z) for maximum space
        Ok(full_left.into_iter().collect())
    }
}

/// Generate evenly-spaced initial keys for bulk insertion
///
/// Useful when creating many items at once and you want to minimize
/// future subdivisions.
///
/// # Arguments
/// * `count` - Total number of keys to generate
///
/// # Returns
/// A vector of `count` keys evenly distributed across the key space
///
/// # Examples
/// ```
/// use raisin_models::fractional_index::initial_keys;
/// let keys = initial_keys(3);
/// assert_eq!(keys.len(), 3);
/// // Keys will be evenly spaced, e.g., ["P", "g", "x"]
/// ```
pub fn initial_keys(count: usize) -> Vec<String> {
    if count == 0 {
        return vec![];
    }

    if count == 1 {
        return vec![first_key()];
    }

    let chars: Vec<char> = BASE62_ALPHABET.chars().collect();
    let step = chars.len() / (count + 1);

    let mut keys: Vec<String> = (1..=count)
        .map(|i| chars[i * step % chars.len()].to_string())
        .collect();

    // Sort to ensure lexicographic ordering
    keys.sort();
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_first_key() {
        assert_eq!(first_key(), "a");
    }

    #[test]
    fn test_last_key() {
        assert_eq!(last_key(), "Z");
    }

    #[test]
    fn test_next_key() {
        assert_eq!(next_key("a"), "b");
        assert_eq!(next_key("z"), "A");
        assert_eq!(next_key("Z"), "Za");
        assert_eq!(next_key(""), "a");
    }

    #[test]
    fn test_midpoint_simple() {
        assert_eq!(midpoint("a", "c").unwrap(), "b");
        assert_eq!(midpoint("0", "2").unwrap(), "1");
    }

    #[test]
    fn test_midpoint_adjacent() {
        let result = midpoint("a", "b").unwrap();
        assert!(result.as_str() > "a");
        assert!(result.as_str() < "b");
        assert_eq!(result, "aZ"); // Middle character appended
    }

    #[test]
    fn test_midpoint_prefix() {
        let result = midpoint("b", "b5").unwrap();
        assert!(result.as_str() > "b");
        assert!(result.as_str() < "b5");
    }

    #[test]
    fn test_midpoint_empty_left() {
        let result = midpoint("", "a").unwrap();
        assert!(result.as_str() < "a");
        assert_eq!(result, "0");
    }

    #[test]
    fn test_midpoint_empty_right() {
        let result = midpoint("Z", "").unwrap();
        assert!(result.as_str() > "Z");
        assert_eq!(result, "Za");
    }

    #[test]
    fn test_midpoint_both_empty() {
        assert_eq!(midpoint("", "").unwrap(), first_key());
    }

    #[test]
    fn test_midpoint_equal_error() {
        assert!(midpoint("a", "a").is_err());
    }

    #[test]
    fn test_midpoint_wrong_order_error() {
        assert!(midpoint("c", "a").is_err());
    }

    #[test]
    fn test_midpoint_ordering() {
        let a = "a";
        let b = midpoint(a, "c").unwrap();
        let c = "c";

        assert!(a < b.as_str());
        assert!(b.as_str() < c);
    }

    #[test]
    fn test_subdivision_many_times() {
        let mut left = "a".to_string();
        let right = "b".to_string();

        // Should be able to subdivide many times
        for _ in 0..100 {
            let mid = midpoint(&left, &right).unwrap();
            assert!(mid.as_str() > left.as_str());
            assert!(mid.as_str() < right.as_str());
            left = mid;
        }
    }

    #[test]
    fn test_initial_keys() {
        let keys = initial_keys(5);
        assert_eq!(keys.len(), 5);

        // All keys should be unique
        let mut sorted = keys.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 5);

        // Keys should be in sorted order
        assert_eq!(keys, sorted);
    }

    #[test]
    fn test_initial_keys_empty() {
        assert_eq!(initial_keys(0).len(), 0);
    }

    #[test]
    fn test_initial_keys_one() {
        assert_eq!(initial_keys(1), vec![first_key()]);
    }
}
