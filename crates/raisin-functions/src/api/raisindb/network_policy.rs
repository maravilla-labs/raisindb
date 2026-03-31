// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Network policy and URL matching for RaisinFunctionApi

use super::RaisinFunctionApi;

impl RaisinFunctionApi {
    /// Check if URL is allowed by network policy
    pub(crate) fn is_url_allowed(&self, url: &str) -> bool {
        tracing::trace!(
            url = url,
            http_enabled = self.network_policy.http_enabled,
            allowed_urls = ?self.network_policy.allowed_urls,
            "is_url_allowed - checking"
        );

        if !self.network_policy.http_enabled {
            tracing::trace!("is_url_allowed - BLOCKED: http_enabled is false");
            return false;
        }

        // If no allowed URLs specified, all are allowed (when http_enabled)
        if self.network_policy.allowed_urls.is_empty() {
            tracing::trace!("is_url_allowed - ALLOWED: no URL restrictions (empty list)");
            return true;
        }

        // Check against allowlist with glob matching
        for pattern in &self.network_policy.allowed_urls {
            let matches = Self::glob_match(pattern, url);
            tracing::trace!(
                pattern = pattern,
                url = url,
                matches = matches,
                "is_url_allowed - pattern check"
            );
            if matches {
                tracing::trace!(pattern = pattern, "is_url_allowed - ALLOWED by pattern");
                return true;
            }
        }

        tracing::trace!("is_url_allowed - BLOCKED: no pattern matched");
        false
    }

    /// Simple glob matching for URL patterns
    /// Supports:
    /// - `*` matches any characters except `/`
    /// - `**` matches any characters including `/`
    pub(crate) fn glob_match(pattern: &str, text: &str) -> bool {
        // Use placeholder to avoid double-replacement issue
        const DOUBLE_STAR_PLACEHOLDER: &str = "\x00DOUBLE_STAR\x00";

        // Step 1: Replace ** with placeholder
        let pattern = pattern.replace("**", DOUBLE_STAR_PLACEHOLDER);

        // Step 2: Escape regex special characters (except our placeholder and *)
        let mut escaped = String::with_capacity(pattern.len() * 2);
        for ch in pattern.chars() {
            match ch {
                '.' | '+' | '?' | '^' | '$' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '\\' => {
                    escaped.push('\\');
                    escaped.push(ch);
                }
                _ => escaped.push(ch),
            }
        }

        // Step 3: Replace * with [^/]* (matches anything except /)
        let pattern = escaped.replace('*', "[^/]*");

        // Step 4: Replace placeholder with .* (matches anything including /)
        let re_pattern = pattern.replace(DOUBLE_STAR_PLACEHOLDER, ".*");

        tracing::trace!(regex_pattern = re_pattern, "glob_match");

        regex::Regex::new(&format!("^{}$", re_pattern))
            .map(|re| re.is_match(text))
            .unwrap_or(false)
    }
}
