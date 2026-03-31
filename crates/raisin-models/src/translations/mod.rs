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

//! Translation system for multi-language content support in RaisinDB.
//!
//! This module provides the core data structures and utilities for storing and
//! managing translations of node content across multiple locales.
//!
//! # Architecture Overview
//!
//! The translation system is built around several key concepts:
//!
//! - **[`LocaleOverlay`]**: Per-locale translation data stored separately from base node
//! - **[`JsonPointer`]**: RFC 6901 paths for granular field-level translations
//! - **[`LocaleCode`]**: Validated BCP 47 language tags (xx or xx-XX format)
//! - **[`TranslationMeta`]**: Revision metadata for translation commits
//!
//! # Module Organization
//!
//! - [`types`]: Core type definitions (LocaleOverlay, JsonPointer, LocaleCode)
//! - [`metadata`]: Translation revision metadata and tracking
//! - [`helpers`]: Utility functions for common translation operations
//!
//! # Design Principles
//!
//! ## 1. Base Language + Overlays
//!
//! Original content is stored in the repository's default language (base language).
//! Translations are stored as overlays that only contain the differences from
//! the base content. This approach:
//!
//! - Minimizes storage requirements
//! - Makes untranslated content fall back naturally to base language
//! - Allows partial translations without gaps
//!
//! ## 2. Revision Awareness
//!
//! Each translation change creates a revision, similar to Git commits. This enables:
//!
//! - Full audit trail of translation changes
//! - Ability to revert to previous translations
//! - Conflict resolution for concurrent updates
//! - Translation history and blame
//!
//! ## 3. Block Stability
//!
//! Block translations are tracked by UUID, not position. This means:
//!
//! - Reordering blocks doesn't break translations
//! - Moving content between documents preserves translations
//! - Content can be versioned independently of structure
//!
//! ## 4. Locale Fallback
//!
//! The system supports locale fallback chains:
//!
//! - `en-US` → `en` → base language
//! - `zh-Hans-CN` → `zh-Hans` → `zh` → base language
//!
//! This allows serving the most specific available translation.
//!
//! # Quick Start
//!
//! ## Creating a translation overlay
//!
//! ```rust
//! use raisin_models::translations::{LocaleOverlay, JsonPointer, LocaleCode};
//! use raisin_models::nodes::properties::PropertyValue;
//! use std::collections::HashMap;
//!
//! // Create a locale code
//! let locale = LocaleCode::parse("fr-FR").unwrap();
//!
//! // Build translation data
//! let mut translations = HashMap::new();
//! translations.insert(
//!     JsonPointer::new("/title"),
//!     PropertyValue::String("Bonjour le monde".to_string())
//! );
//! translations.insert(
//!     JsonPointer::new("/description"),
//!     PropertyValue::String("Ceci est un exemple".to_string())
//! );
//!
//! // Create overlay
//! let overlay = LocaleOverlay::properties(translations);
//! assert_eq!(overlay.len(), 2);
//! ```
//!
//! ## Working with translation metadata
//!
//! ```rust
//! use raisin_models::translations::{TranslationMeta, LocaleCode};
//!
//! let locale = LocaleCode::parse("es").unwrap();
//! let meta = TranslationMeta::new(
//!     locale,
//!     42,                                    // revision (HLC timestamp)
//!     Some(41),                              // parent revision
//!     "translator@example.com".to_string(),  // actor
//!     "Add Spanish translations".to_string() // message
//! );
//!
//! assert_eq!(meta.revision, 42);
//! assert!(!meta.is_system);
//! ```
//!
//! ## Using helper functions
//!
//! ```rust
//! use raisin_models::translations::{LocaleCode, helpers};
//!
//! // Get locale fallback chain
//! let locale = LocaleCode::parse("en-US").unwrap();
//! let chain = helpers::locale_fallback_chain(&locale);
//! assert_eq!(chain.len(), 2); // ["en-US", "en"]
//!
//! // Check if field is translatable
//! use raisin_models::translations::JsonPointer;
//! let ptr = JsonPointer::new("/title");
//! assert!(helpers::is_translatable_field(&ptr));
//! ```
//!
//! # Storage and Persistence
//!
//! Translation data is stored in RocksDB using MessagePack serialization.
//! The storage layer (in `raisin-rocksdb` crate) provides:
//!
//! - Efficient key-value access by node ID and locale
//! - Range queries for listing all translations
//! - Atomic updates with revision tracking
//! - Replication support for distributed deployments
//!
//! # Performance Considerations
//!
//! The translation system is designed for high performance:
//!
//! - **Overlay storage**: O(n) where n is number of translated fields
//! - **Locale lookup**: O(1) hash map access
//! - **Fallback resolution**: O(k) where k is fallback chain length (typically 1-2)
//! - **Merge operation**: O(n) where n is number of fields in overlay
//!
//! Memory usage is minimized by:
//! - Only storing differences from base content
//! - Using string interning for common field names
//! - Sharing unchanged data between locales
//!
//! # Thread Safety
//!
//! All types in this module are `Send + Sync` when appropriate:
//!
//! - [`LocaleCode`], [`JsonPointer`]: Immutable, safe to share
//! - [`LocaleOverlay`]: Can be cloned for concurrent access
//! - [`TranslationMeta`]: Immutable after creation
//!
//! # Examples
//!
//! ## Complete translation workflow
//!
//! ```rust
//! use raisin_models::translations::{
//!     LocaleCode, LocaleOverlay, JsonPointer, TranslationMeta, helpers
//! };
//! use raisin_models::nodes::properties::PropertyValue;
//! use std::collections::HashMap;
//!
//! // 1. Create translation for a specific locale
//! let locale = LocaleCode::parse("de-DE").unwrap();
//! let mut translations = HashMap::new();
//! translations.insert(
//!     JsonPointer::new("/title"),
//!     PropertyValue::String("Hallo Welt".to_string())
//! );
//! let overlay = LocaleOverlay::properties(translations);
//!
//! // 2. Create metadata for the translation
//! let meta = TranslationMeta::new(
//!     locale.clone(),
//!     100,
//!     None, // Initial translation
//!     "hans@example.com".to_string(),
//!     "Add German translation".to_string()
//! );
//!
//! // 3. Check translation completeness
//! let base_translations = HashMap::from([
//!     (JsonPointer::new("/title"), PropertyValue::String("Hello".to_string())),
//!     (JsonPointer::new("/description"), PropertyValue::String("Desc".to_string())),
//! ]);
//! let base_overlay = LocaleOverlay::properties(base_translations);
//!
//! let completeness = helpers::translation_completeness(&overlay, &base_overlay);
//! assert_eq!(completeness, 50.0); // 1 out of 2 fields translated
//! ```
//!
//! ## Handling missing translations with fallback
//!
//! ```rust
//! use raisin_models::translations::{LocaleCode, helpers};
//! use std::collections::HashSet;
//!
//! // User requests en-US but we only have en
//! let requested = LocaleCode::parse("en-US").unwrap();
//! let mut available = HashSet::new();
//! available.insert(LocaleCode::parse("en").unwrap());
//! available.insert(LocaleCode::parse("fr").unwrap());
//!
//! let best_match = helpers::find_best_locale(&requested, &available);
//! assert_eq!(best_match.unwrap().as_str(), "en"); // Falls back to en
//! ```
//!
//! # See Also
//!
//! - [`raisin-rocksdb::repositories::translations`]: Storage implementation
//! - [`raisin-server`]: API endpoints for translation management
//! - RFC 6901: JSON Pointer specification
//! - BCP 47: Language tags specification

// Re-export all public types from submodules
pub use hash_record::{MissingFieldInfo, StaleFieldInfo, StalenessReport, TranslationHashRecord};
pub use metadata::{TranslationMeta, TranslationMetaBuilder};
pub use types::{JsonPointer, LocaleCode, LocaleOverlay};

// Public modules
pub mod hash_record;
pub mod helpers;
pub mod metadata;
pub mod types;

// Tests module (private, only compiled for tests)
#[cfg(test)]
mod tests;
