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

//! Locale resolution and fallback chain utilities.

use crate::translations::types::LocaleCode;
use std::collections::HashSet;

/// Generate a locale fallback chain.
///
/// Returns a vector of locales to try in order, from most specific to least.
/// This implements the standard locale fallback behavior:
/// - `en-US` -> `["en-US", "en"]`
/// - `zh-Hans-CN` -> `["zh-Hans-CN", "zh-Hans", "zh"]`
/// - `fr` -> `["fr"]`
///
/// # Arguments
///
/// * `locale` - The starting locale
///
/// # Returns
///
/// A vector of locales in fallback order (most specific first).
///
/// # Examples
///
/// ```rust
/// use raisin_models::translations::{LocaleCode, helpers};
///
/// let locale = LocaleCode::parse("en-US").unwrap();
/// let chain = helpers::locale_fallback_chain(&locale);
///
/// assert_eq!(chain.len(), 2);
/// assert_eq!(chain[0].as_str(), "en-US");
/// assert_eq!(chain[1].as_str(), "en");
///
/// let locale = LocaleCode::parse("fr").unwrap();
/// let chain = helpers::locale_fallback_chain(&locale);
/// assert_eq!(chain.len(), 1);
/// assert_eq!(chain[0].as_str(), "fr");
/// ```
pub fn locale_fallback_chain(locale: &LocaleCode) -> Vec<LocaleCode> {
    let mut chain = vec![locale.clone()];
    let mut current = locale.clone();

    while let Some(parent) = current.parent() {
        chain.push(parent.clone());
        current = parent;
    }

    chain
}

/// Find the best matching locale from available locales.
///
/// Uses fallback chain to find the most specific available locale.
/// Returns `None` if no matching locale is available.
///
/// # Arguments
///
/// * `requested` - The requested locale
/// * `available` - Set of available locales
///
/// # Returns
///
/// The best matching locale, or `None` if no match found.
///
/// # Examples
///
/// ```rust
/// use raisin_models::translations::{LocaleCode, helpers};
/// use std::collections::HashSet;
///
/// let requested = LocaleCode::parse("en-US").unwrap();
/// let mut available = HashSet::new();
/// available.insert(LocaleCode::parse("en").unwrap());
/// available.insert(LocaleCode::parse("fr").unwrap());
///
/// let matched = helpers::find_best_locale(&requested, &available);
/// assert_eq!(matched.unwrap().as_str(), "en");
/// ```
pub fn find_best_locale<'a>(
    requested: &LocaleCode,
    available: &'a HashSet<LocaleCode>,
) -> Option<&'a LocaleCode> {
    let chain = locale_fallback_chain(requested);

    for locale in &chain {
        if available.contains(locale) {
            return available.get(locale);
        }
    }

    None
}
