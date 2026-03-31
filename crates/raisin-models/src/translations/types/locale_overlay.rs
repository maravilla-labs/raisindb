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

//! Locale-specific overlay for node translations.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::JsonPointer;
use crate::nodes::properties::PropertyValue;

/// Locale-specific overlay for node translations.
///
/// Stores only the differences from the base node content, enabling
/// efficient storage and fast resolution. This overlay pattern allows:
///
/// - **Partial translations**: Only translated fields are stored
/// - **Small storage footprint**: No duplication of unchanged fields
/// - **Fast merging**: Base node + overlay = translated node
/// - **Visibility control**: Hide nodes per locale with tombstone
///
/// # Overlay Types
///
/// ## Properties Overlay
///
/// Contains a map of JsonPointer paths to translated PropertyValue.
/// Only translatable fields (strings, text, etc.) should be included.
///
/// ## Hidden Tombstone
///
/// Indicates that a node should not be visible in this locale.
/// Useful for locale-specific content filtering.
///
/// # Storage Format
///
/// Overlays are serialized using MessagePack and stored in RocksDB
/// with keys that combine node ID and locale code for efficient lookups.
///
/// # Performance Characteristics
///
/// - **Memory**: O(n) where n is the number of translated fields
/// - **Lookup**: O(1) hash map access per field
/// - **Merge**: O(n) iteration over overlay fields
///
/// # Examples
///
/// ```rust
/// use raisin_models::translations::{LocaleOverlay, JsonPointer};
/// use raisin_models::nodes::properties::PropertyValue;
/// use std::collections::HashMap;
///
/// // Create a properties overlay
/// let mut data = HashMap::new();
/// data.insert(
///     JsonPointer::new("/title"),
///     PropertyValue::String("Hola".to_string())
/// );
/// let overlay = LocaleOverlay::properties(data);
///
/// // Create a hidden tombstone
/// let hidden = LocaleOverlay::hidden();
/// assert!(hidden.is_hidden());
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LocaleOverlay {
    /// Property-level translation overlay.
    ///
    /// Maps JsonPointer paths to translated PropertyValue.
    /// Only translatable fields can be included.
    Properties {
        /// Map of JsonPointer paths to translated values
        data: HashMap<JsonPointer, PropertyValue>,
    },

    /// Tombstone indicating node is hidden in this locale.
    ///
    /// When resolved, the node will not be returned for queries
    /// in this locale. Useful for locale-specific content visibility.
    Hidden,
}

impl LocaleOverlay {
    /// Create a new Properties overlay with the given translations.
    ///
    /// # Arguments
    ///
    /// * `data` - Map of JSON pointer paths to translated property values
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::{LocaleOverlay, JsonPointer};
    /// use raisin_models::nodes::properties::PropertyValue;
    /// use std::collections::HashMap;
    ///
    /// let mut translations = HashMap::new();
    /// translations.insert(
    ///     JsonPointer::new("/description"),
    ///     PropertyValue::String("Descripcion en espanol".to_string())
    /// );
    ///
    /// let overlay = LocaleOverlay::properties(translations);
    /// assert!(!overlay.is_hidden());
    /// assert_eq!(overlay.len(), 1);
    /// ```
    pub fn properties(data: HashMap<JsonPointer, PropertyValue>) -> Self {
        LocaleOverlay::Properties { data }
    }

    /// Create a Hidden tombstone overlay.
    ///
    /// This marks a node as hidden in a specific locale, meaning it should
    /// not appear in query results for that locale.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::LocaleOverlay;
    ///
    /// let overlay = LocaleOverlay::hidden();
    /// assert!(overlay.is_hidden());
    /// assert_eq!(overlay.len(), 0);
    /// ```
    pub fn hidden() -> Self {
        LocaleOverlay::Hidden
    }

    /// Check if this overlay is a Hidden tombstone.
    ///
    /// # Returns
    ///
    /// `true` if this overlay marks the node as hidden, `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::LocaleOverlay;
    /// use std::collections::HashMap;
    ///
    /// let hidden = LocaleOverlay::hidden();
    /// assert!(hidden.is_hidden());
    ///
    /// let properties = LocaleOverlay::properties(HashMap::new());
    /// assert!(!properties.is_hidden());
    /// ```
    #[inline]
    pub fn is_hidden(&self) -> bool {
        matches!(self, LocaleOverlay::Hidden)
    }

    /// Get a reference to the properties map if this is a Properties overlay.
    ///
    /// # Returns
    ///
    /// `Some(&HashMap)` if this is a Properties overlay, `None` if Hidden.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::{LocaleOverlay, JsonPointer};
    /// use raisin_models::nodes::properties::PropertyValue;
    /// use std::collections::HashMap;
    ///
    /// let mut data = HashMap::new();
    /// data.insert(
    ///     JsonPointer::new("/title"),
    ///     PropertyValue::String("Title".to_string())
    /// );
    /// let overlay = LocaleOverlay::properties(data);
    ///
    /// let properties = overlay.properties_ref().unwrap();
    /// assert_eq!(properties.len(), 1);
    /// ```
    pub fn properties_ref(&self) -> Option<&HashMap<JsonPointer, PropertyValue>> {
        match self {
            LocaleOverlay::Properties { data } => Some(data),
            LocaleOverlay::Hidden => None,
        }
    }

    /// Get a mutable reference to the properties map if this is a Properties overlay.
    ///
    /// # Returns
    ///
    /// `Some(&mut HashMap)` if this is a Properties overlay, `None` if Hidden.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::{LocaleOverlay, JsonPointer};
    /// use raisin_models::nodes::properties::PropertyValue;
    /// use std::collections::HashMap;
    ///
    /// let mut overlay = LocaleOverlay::properties(HashMap::new());
    ///
    /// if let Some(props) = overlay.properties_mut() {
    ///     props.insert(
    ///         JsonPointer::new("/new_field"),
    ///         PropertyValue::String("New value".to_string())
    ///     );
    /// }
    ///
    /// assert_eq!(overlay.len(), 1);
    /// ```
    pub fn properties_mut(&mut self) -> Option<&mut HashMap<JsonPointer, PropertyValue>> {
        match self {
            LocaleOverlay::Properties { data } => Some(data),
            LocaleOverlay::Hidden => None,
        }
    }

    /// Get the number of translated fields.
    ///
    /// Returns 0 for Hidden overlays.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::{LocaleOverlay, JsonPointer};
    /// use raisin_models::nodes::properties::PropertyValue;
    /// use std::collections::HashMap;
    ///
    /// let mut data = HashMap::new();
    /// data.insert(
    ///     JsonPointer::new("/title"),
    ///     PropertyValue::String("Title".to_string())
    /// );
    /// data.insert(
    ///     JsonPointer::new("/description"),
    ///     PropertyValue::String("Description".to_string())
    /// );
    /// let overlay = LocaleOverlay::properties(data);
    ///
    /// assert_eq!(overlay.len(), 2);
    /// ```
    #[inline]
    pub fn len(&self) -> usize {
        match self {
            LocaleOverlay::Properties { data } => data.len(),
            LocaleOverlay::Hidden => 0,
        }
    }

    /// Check if the overlay is empty (no translations).
    ///
    /// Returns `true` for Hidden overlays or Properties overlays with no fields.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::LocaleOverlay;
    /// use std::collections::HashMap;
    ///
    /// let empty = LocaleOverlay::properties(HashMap::new());
    /// assert!(empty.is_empty());
    ///
    /// let hidden = LocaleOverlay::hidden();
    /// assert!(hidden.is_empty());
    /// ```
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
