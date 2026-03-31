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

//! JSON Pointer (RFC 6901) for addressing specific fields within a node.

use raisin_error::{Error, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

/// JSON Pointer (RFC 6901) for addressing specific fields within a node.
///
/// A JSON Pointer is a string syntax for identifying a specific value within
/// a JSON document. It's used in the translation system to identify which
/// fields are translated in a [`super::LocaleOverlay`].
///
/// # Format
///
/// A JSON Pointer is a Unicode string containing a sequence of zero or more
/// reference tokens, each prefixed by a '/' character.
///
/// - `/field` - Top-level field
/// - `/nested/field` - Nested object field
/// - `/array/0` - Array element by index
/// - `/blocks/abc-123/content/text` - Block field by UUID
///
/// # Special Characters
///
/// According to RFC 6901:
/// - `~0` encodes `~`
/// - `~1` encodes `/`
///
/// # Block References
///
/// For content blocks, use UUIDs instead of array indices for stability:
/// - Good: `/blocks/550e8400-e29b-41d4-a716-446655440000/content/text`
/// - Bad: `/blocks/0/content/text` (changes if blocks are reordered)
///
/// # Performance
///
/// JsonPointer is a thin wrapper around `String` with validation.
/// - **Memory**: O(n) where n is the path length
/// - **Comparison**: O(n) string comparison
/// - **Hashing**: O(n) string hashing (cached by HashMap)
///
/// # Examples
///
/// ```rust
/// use raisin_models::translations::JsonPointer;
///
/// // Create a pointer
/// let ptr = JsonPointer::new("/properties/title");
/// assert_eq!(ptr.as_str(), "/properties/title");
///
/// // Parse with validation
/// let ptr = JsonPointer::parse("/array/0/field").unwrap();
/// assert_eq!(ptr.segments(), &["array", "0", "field"]);
///
/// // Navigate the tree
/// let parent = ptr.parent().unwrap();
/// assert_eq!(parent.as_str(), "/array/0");
///
/// // Check prefixes
/// let child = JsonPointer::new("/properties/blocks/0/text");
/// let parent = JsonPointer::new("/properties/blocks");
/// assert!(child.starts_with(&parent));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct JsonPointer(String);

impl JsonPointer {
    /// Create a new JsonPointer from a string path.
    ///
    /// # Panics
    ///
    /// Panics if the path does not start with '/'. Use [`parse`](Self::parse)
    /// for fallible construction.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::JsonPointer;
    ///
    /// let ptr = JsonPointer::new("/title");
    /// assert_eq!(ptr.as_str(), "/title");
    /// ```
    ///
    /// ```should_panic
    /// use raisin_models::translations::JsonPointer;
    ///
    /// // This will panic
    /// let ptr = JsonPointer::new("invalid");
    /// ```
    #[inline]
    pub fn new(path: impl Into<String>) -> Self {
        let path = path.into();
        assert!(path.starts_with('/'), "JsonPointer must start with '/'");
        JsonPointer(path)
    }

    /// Parse and validate a JsonPointer string.
    ///
    /// Returns an error if the path is invalid (doesn't start with '/').
    ///
    /// # Errors
    ///
    /// Returns [`Error::Validation`] if:
    /// - The path doesn't start with '/'
    /// - The path contains invalid escape sequences
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::JsonPointer;
    ///
    /// let ptr = JsonPointer::parse("/valid/path").unwrap();
    /// assert_eq!(ptr.as_str(), "/valid/path");
    ///
    /// let invalid = JsonPointer::parse("no-slash");
    /// assert!(invalid.is_err());
    /// ```
    pub fn parse(path: impl Into<String>) -> Result<Self> {
        let path = path.into();
        if !path.starts_with('/') {
            return Err(Error::Validation(format!(
                "JsonPointer must start with '/': {}",
                path
            )));
        }
        Ok(JsonPointer(path))
    }

    /// Get the pointer as a string slice.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::JsonPointer;
    ///
    /// let ptr = JsonPointer::new("/title");
    /// assert_eq!(ptr.as_str(), "/title");
    /// ```
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Get the path segments (split by '/').
    ///
    /// The first empty segment (before the leading '/') is skipped.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::JsonPointer;
    ///
    /// let ptr = JsonPointer::new("/properties/nested/field");
    /// assert_eq!(ptr.segments(), vec!["properties", "nested", "field"]);
    ///
    /// let root = JsonPointer::new("/single");
    /// assert_eq!(root.segments(), vec!["single"]);
    /// ```
    pub fn segments(&self) -> Vec<&str> {
        self.0
            .split('/')
            .skip(1) // Skip empty string before first '/'
            .collect()
    }

    /// Get the parent pointer (remove last segment).
    ///
    /// Returns `None` if this pointer has no parent (single segment).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::JsonPointer;
    ///
    /// let ptr = JsonPointer::new("/properties/nested/field");
    /// let parent = ptr.parent().unwrap();
    /// assert_eq!(parent.as_str(), "/properties/nested");
    ///
    /// let root = JsonPointer::new("/single");
    /// assert!(root.parent().is_none());
    /// ```
    pub fn parent(&self) -> Option<JsonPointer> {
        let segments = self.segments();
        if segments.is_empty() || segments.len() == 1 {
            return None;
        }
        let parent_path = segments[..segments.len() - 1]
            .iter()
            .map(|s| format!("/{}", s))
            .collect::<String>();
        Some(JsonPointer(parent_path))
    }

    /// Check if this pointer starts with another pointer (prefix check).
    ///
    /// This is useful for filtering fields that belong to a specific
    /// section of the document.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::JsonPointer;
    ///
    /// let ptr = JsonPointer::new("/properties/blocks/0/text");
    /// let prefix = JsonPointer::new("/properties/blocks");
    /// assert!(ptr.starts_with(&prefix));
    ///
    /// let other = JsonPointer::new("/other/path");
    /// assert!(!ptr.starts_with(&other));
    /// ```
    #[inline]
    pub fn starts_with(&self, prefix: &JsonPointer) -> bool {
        self.0.starts_with(&prefix.0)
    }

    /// Append a segment to this pointer.
    ///
    /// Creates a new pointer with the given segment appended.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::JsonPointer;
    ///
    /// let ptr = JsonPointer::new("/properties");
    /// let child = ptr.append("title");
    /// assert_eq!(child.as_str(), "/properties/title");
    /// ```
    #[inline]
    pub fn append(&self, segment: &str) -> JsonPointer {
        JsonPointer(format!("{}/{}", self.0, segment))
    }

    /// Check if this pointer references an array index.
    ///
    /// Returns `true` if the last segment is a valid non-negative integer.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::JsonPointer;
    ///
    /// let array_ptr = JsonPointer::new("/array/0");
    /// assert!(array_ptr.is_array_index());
    ///
    /// let object_ptr = JsonPointer::new("/array/field");
    /// assert!(!object_ptr.is_array_index());
    /// ```
    pub fn is_array_index(&self) -> bool {
        self.segments()
            .last()
            .map(|s| s.parse::<usize>().is_ok())
            .unwrap_or(false)
    }

    /// Get the last segment of this pointer.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::JsonPointer;
    ///
    /// let ptr = JsonPointer::new("/properties/nested/field");
    /// assert_eq!(ptr.last_segment(), Some("field"));
    ///
    /// let root = JsonPointer::new("/");
    /// assert_eq!(root.last_segment(), None);
    /// ```
    pub fn last_segment(&self) -> Option<&str> {
        self.segments().last().copied()
    }
}

impl fmt::Display for JsonPointer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for JsonPointer {
    fn from(s: String) -> Self {
        JsonPointer::new(s)
    }
}

impl From<&str> for JsonPointer {
    fn from(s: &str) -> Self {
        JsonPointer::new(s)
    }
}

impl AsRef<str> for JsonPointer {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
