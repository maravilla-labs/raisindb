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

//! Validated locale code (language tag) based on BCP 47.

use raisin_error::{Error, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Validated locale code (language tag).
///
/// A locale code identifies a language and optionally a region, following
/// the BCP 47 standard. This type provides validation and normalization
/// to ensure consistent locale handling throughout the system.
///
/// # Format
///
/// The basic format is: `language[-region]`
///
/// - `language`: 2-3 lowercase ASCII letters (ISO 639)
/// - `region`: 2 uppercase ASCII letters (ISO 3166-1 alpha-2) or 3 digits
///
/// # Supported Formats
///
/// - `en` - Language only
/// - `en-US` - Language with region (United States)
/// - `en-GB` - Language with region (Great Britain)
/// - `zh-CN` - Language with region (China)
/// - `pt-BR` - Language with region (Brazil)
/// - `zh-Hans` - Language with script (Simplified Chinese)
///
/// # Normalization
///
/// Locale codes are automatically normalized on creation:
/// - Language part is lowercased (`EN` -> `en`)
/// - Region part is uppercased (`us` -> `US`)
/// - Format is validated and rejected if invalid
///
/// # Locale Fallback
///
/// The system supports locale fallback through the [`parent`](Self::parent) method:
/// - `en-US` falls back to `en`
/// - `en` has no fallback (base language)
///
/// # Examples
///
/// ```rust
/// use raisin_models::translations::LocaleCode;
///
/// // Parse and validate
/// let locale = LocaleCode::parse("en-US").unwrap();
/// assert_eq!(locale.as_str(), "en-US");
/// assert_eq!(locale.language(), "en");
/// assert_eq!(locale.region(), Some("US"));
///
/// // Normalization
/// let locale = LocaleCode::parse("EN-us").unwrap();
/// assert_eq!(locale.as_str(), "en-US");
///
/// // Language-only
/// let locale = LocaleCode::parse("fr").unwrap();
/// assert_eq!(locale.region(), None);
///
/// // Locale fallback
/// let specific = LocaleCode::parse("en-US").unwrap();
/// let fallback = specific.parent().unwrap();
/// assert_eq!(fallback.as_str(), "en");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct LocaleCode(String);

impl LocaleCode {
    /// Parse and validate a locale code string.
    ///
    /// Normalizes the code to lowercase language and uppercase region/script.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Validation`] if:
    /// - Language code is not 2-3 lowercase letters
    /// - Region code is not 2 uppercase letters, 3 digits, or 4 letters
    /// - The format contains more than one hyphen
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::LocaleCode;
    ///
    /// // Valid codes
    /// assert!(LocaleCode::parse("en").is_ok());
    /// assert!(LocaleCode::parse("en-US").is_ok());
    /// assert!(LocaleCode::parse("zh-Hans").is_ok());
    ///
    /// // Invalid codes
    /// assert!(LocaleCode::parse("e").is_err());      // Too short
    /// assert!(LocaleCode::parse("english").is_err()); // Too long
    /// assert!(LocaleCode::parse("en-U").is_err());    // Invalid region
    /// ```
    pub fn parse(code: impl AsRef<str>) -> Result<Self> {
        let code = code.as_ref();

        // Split by hyphen
        let parts: Vec<&str> = code.split('-').collect();

        match parts.len() {
            1 => {
                // Language only (e.g., "en", "fr")
                let lang = parts[0].to_lowercase();
                if !Self::is_valid_language(&lang) {
                    return Err(Error::Validation(format!(
                        "Invalid language code: {}",
                        parts[0]
                    )));
                }
                Ok(LocaleCode(lang))
            }
            2 => {
                // Language-Region (e.g., "en-US", "zh-Hans")
                let lang = parts[0].to_lowercase();
                let region = parts[1].to_uppercase();

                if !Self::is_valid_language(&lang) {
                    return Err(Error::Validation(format!(
                        "Invalid language code: {}",
                        parts[0]
                    )));
                }
                if !Self::is_valid_region(&region) {
                    return Err(Error::Validation(format!(
                        "Invalid region code: {}",
                        parts[1]
                    )));
                }

                Ok(LocaleCode(format!("{}-{}", lang, region)))
            }
            _ => Err(Error::Validation(format!(
                "Invalid locale format: {}",
                code
            ))),
        }
    }

    /// Check if a language code is valid (2-3 lowercase letters).
    fn is_valid_language(lang: &str) -> bool {
        lang.len() >= 2 && lang.len() <= 3 && lang.chars().all(|c| c.is_ascii_lowercase())
    }

    /// Check if a region code is valid.
    ///
    /// Valid formats:
    /// - 2 uppercase letters (ISO 3166-1 alpha-2): US, GB, DE
    /// - 3 digits (UN M.49): 840, 826, 276
    /// - 4 letters (ISO 15924 script code): Hans, Latn
    fn is_valid_region(region: &str) -> bool {
        (region.len() == 2 && region.chars().all(|c| c.is_ascii_uppercase()))
            || (region.len() == 3 && region.chars().all(|c| c.is_ascii_digit()))
            || (region.len() == 4 && region.chars().all(|c| c.is_ascii_alphabetic()))
    }

    /// Get the locale code as a string slice.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::LocaleCode;
    ///
    /// let locale = LocaleCode::parse("en-US").unwrap();
    /// assert_eq!(locale.as_str(), "en-US");
    /// ```
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Get the language part of the locale (before hyphen).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::LocaleCode;
    ///
    /// let locale = LocaleCode::parse("en-US").unwrap();
    /// assert_eq!(locale.language(), "en");
    ///
    /// let locale = LocaleCode::parse("fr").unwrap();
    /// assert_eq!(locale.language(), "fr");
    /// ```
    #[inline]
    pub fn language(&self) -> &str {
        self.0.split('-').next().expect("split always yields at least one element")
    }

    /// Get the region part of the locale (after hyphen), if present.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::LocaleCode;
    ///
    /// let locale = LocaleCode::parse("en-US").unwrap();
    /// assert_eq!(locale.region(), Some("US"));
    ///
    /// let locale = LocaleCode::parse("en").unwrap();
    /// assert_eq!(locale.region(), None);
    /// ```
    #[inline]
    pub fn region(&self) -> Option<&str> {
        self.0.split('-').nth(1)
    }

    /// Get the parent locale (remove region).
    ///
    /// Returns `None` if this is already a language-only locale.
    /// This is useful for implementing locale fallback chains.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::LocaleCode;
    ///
    /// // Regional locale has parent
    /// let locale = LocaleCode::parse("en-US").unwrap();
    /// let parent = locale.parent().unwrap();
    /// assert_eq!(parent.as_str(), "en");
    ///
    /// // Language-only locale has no parent
    /// assert!(parent.parent().is_none());
    ///
    /// // Script-specific locale has parent
    /// let locale = LocaleCode::parse("zh-Hans").unwrap();
    /// let parent = locale.parent().unwrap();
    /// assert_eq!(parent.as_str(), "zh");
    /// ```
    pub fn parent(&self) -> Option<LocaleCode> {
        if self.region().is_some() {
            Some(LocaleCode(self.language().to_string()))
        } else {
            None
        }
    }

    /// Check if this locale is a language-only code (no region).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::LocaleCode;
    ///
    /// let locale = LocaleCode::parse("en").unwrap();
    /// assert!(locale.is_language_only());
    ///
    /// let locale = LocaleCode::parse("en-US").unwrap();
    /// assert!(!locale.is_language_only());
    /// ```
    #[inline]
    pub fn is_language_only(&self) -> bool {
        self.region().is_none()
    }

    /// Check if this locale matches another locale or is its parent.
    ///
    /// This is useful for locale fallback resolution:
    /// - `en` matches `en-US` (parent)
    /// - `en-US` matches `en-US` (exact)
    /// - `en-US` does not match `en-GB` (different region)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use raisin_models::translations::LocaleCode;
    ///
    /// let base = LocaleCode::parse("en").unwrap();
    /// let specific = LocaleCode::parse("en-US").unwrap();
    ///
    /// assert!(base.matches(&specific));
    /// assert!(specific.matches(&specific));
    /// assert!(!specific.matches(&base));
    /// ```
    pub fn matches(&self, other: &LocaleCode) -> bool {
        self == other || (self.is_language_only() && self.language() == other.language())
    }
}

impl fmt::Display for LocaleCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<LocaleCode> for String {
    fn from(code: LocaleCode) -> String {
        code.0
    }
}

impl TryFrom<String> for LocaleCode {
    type Error = Error;

    fn try_from(s: String) -> Result<Self> {
        LocaleCode::parse(s)
    }
}

impl TryFrom<&str> for LocaleCode {
    type Error = Error;

    fn try_from(s: &str) -> Result<Self> {
        LocaleCode::parse(s)
    }
}

impl AsRef<str> for LocaleCode {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
