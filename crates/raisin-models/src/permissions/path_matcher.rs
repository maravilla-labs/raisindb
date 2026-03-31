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

//! Glob-based path pattern matching for permissions.
//!
//! This module provides efficient path pattern matching using glob-style wildcards.
//! Patterns use slash-separated paths with wildcards:
//!
//! - `*` matches any characters except `/` (single segment wildcard)
//! - `**` matches any characters including `/` (recursive wildcard)
//! - `?` matches any single character except `/`
//!
//! Examples:
//! - `/00**` matches `/00`, `/000`, `/001`, `/00abc`, `/00/child` (prefix match)
//! - `/articles/*` matches `/articles/news` but not `/articles` or `/articles/a/b`
//! - `/articles/**` matches `/articles`, `/articles/news`, `/articles/news/2024`
//! - `/**/blog/**` matches `/blog`, `/foo/blog/post`, `/a/b/blog/x/y`
//! - `/users/*/profile` matches `/users/123/profile` but not `/users/123/456/profile`
//! - `/**` matches any path

use regex::Regex;
use std::fmt;

/// A pre-compiled glob-based path pattern matcher.
///
/// Compiles a pattern string once for efficient reuse across many matches.
/// This avoids repeated pattern compilation during permission checks.
#[derive(Debug, Clone)]
pub struct PathMatcher {
    /// The original pattern string
    pattern: String,
    /// Pre-compiled regex pattern (None if pattern is invalid)
    regex: Option<Regex>,
    /// Pre-computed specificity score (higher = more specific)
    specificity: usize,
}

impl PathMatcher {
    /// Create a new path matcher from a pattern string.
    ///
    /// Pattern uses slash-separated segments with glob wildcards:
    /// - `*` matches any characters except `/`
    /// - `**` matches any characters including `/`
    /// - `?` matches any single character except `/`
    pub fn new(pattern: &str) -> Self {
        let regex_pattern = Self::pattern_to_regex(pattern);
        let regex = Regex::new(&regex_pattern).ok();
        let specificity = Self::calculate_specificity(pattern);

        Self {
            pattern: pattern.to_string(),
            regex,
            specificity,
        }
    }

    /// Convert a glob pattern to a regex pattern.
    ///
    /// Glob to regex conversion:
    /// - `**` → `.*` (match any characters including `/`)
    /// - `*` → `[^/]*` (match any characters except `/`)
    /// - `?` → `[^/]` (match single character except `/`)
    /// - `/**` at end → `(/.*)?` (optional slash and anything)
    /// - `/**/` in middle → `(/.*/|/)` (zero or more path segments)
    /// - Other special chars are escaped
    fn pattern_to_regex(pattern: &str) -> String {
        let normalized = pattern.trim_start_matches('/');

        if normalized.is_empty() {
            // Empty pattern matches everything
            return "^.*$".to_string();
        }

        // Pre-process to handle special glob patterns:
        // Use placeholders that won't conflict with normal patterns
        let mut processed = normalized.to_string();

        // Handle **/ at start (after leading / stripped) - replace with placeholder
        // This allows matching zero or more leading path segments
        if processed.starts_with("**/") {
            processed = "\x00LEAD\x00".to_string() + &processed[3..];
        }

        // Handle /**/ in middle - replace with placeholder
        // This allows matching zero segments between parts
        while processed.contains("/**/") {
            processed = processed.replace("/**/", "\x00RECURSIVE\x00");
        }

        // Handle /** at end - replace with placeholder
        if processed.ends_with("/**") {
            processed = processed[..processed.len() - 3].to_string() + "\x00TRAIL\x00";
        }

        let mut regex = String::from("^");
        let mut chars = processed.chars().peekable();

        while let Some(c) = chars.next() {
            // Check for special placeholders
            if c == '\x00' {
                // Read until next \x00
                let mut marker = String::new();
                while let Some(&next) = chars.peek() {
                    if next == '\x00' {
                        chars.next(); // consume closing \x00
                        break;
                    }
                    marker.push(chars.next().unwrap());
                }

                match marker.as_str() {
                    "LEAD" => {
                        // (.*/)?  allows optional leading path segments
                        regex.push_str("(.*/)?");
                    }
                    "RECURSIVE" => {
                        // (/.*/)? allows zero or more path segments between
                        regex.push_str("(/.*/|/)");
                    }
                    "TRAIL" => {
                        // (/.*)?  allows optional trailing path
                        regex.push_str("(/.*)?");
                    }
                    _ => {}
                }
                continue;
            }

            match c {
                '*' => {
                    if chars.peek() == Some(&'*') {
                        // ** - match any characters including /
                        chars.next();
                        regex.push_str(".*");
                    } else {
                        // * - match any characters except /
                        regex.push_str("[^/]*");
                    }
                }
                '?' => {
                    // ? - match single character except /
                    regex.push_str("[^/]");
                }
                // Escape regex special characters
                '.' | '+' | '^' | '$' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '\\' => {
                    regex.push('\\');
                    regex.push(c);
                }
                _ => {
                    regex.push(c);
                }
            }
        }

        regex.push('$');
        regex
    }

    /// Check if a path matches this pattern.
    ///
    /// The path uses slash-separated segments (e.g., `/content/articles/news`).
    pub fn matches(&self, path: &str) -> bool {
        let normalized_path = path.trim_start_matches('/');

        match &self.regex {
            Some(regex) => regex.is_match(normalized_path),
            None => false,
        }
    }

    /// Calculate specificity score for a pattern.
    ///
    /// Scoring:
    /// - Exact segment (no wildcards): 100 points
    /// - Single wildcard `*`: 10 points
    /// - Prefix with `**` (e.g., `foo**`): 5 points
    /// - Recursive wildcard `**`: 1 point
    /// - Length bonus: segments.len() * 5
    ///
    /// More specific patterns (fewer wildcards) score higher.
    fn calculate_specificity(pattern: &str) -> usize {
        let mut score = 0;
        let segments: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();

        for segment in &segments {
            if *segment == "**" {
                // Recursive wildcard - least specific
                score += 1;
            } else if segment.contains("**") {
                // Contains recursive wildcard (e.g., prefix**)
                score += 5;
            } else if segment.contains('*') || segment.contains('?') {
                // Contains single wildcard
                score += 10;
            } else {
                // Exact segment - most specific
                score += 100;
            }
        }

        // Longer patterns are more specific
        score += segments.len() * 5;
        score
    }

    /// Get the specificity score for this pattern.
    ///
    /// Higher scores indicate more specific patterns.
    /// When multiple permissions match, the most specific one wins.
    pub fn specificity(&self) -> usize {
        self.specificity
    }

    /// Get the original pattern string.
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    /// Check if this is an unrestricted pattern (matches everything).
    pub fn is_unrestricted(&self) -> bool {
        self.pattern == "/**" || self.pattern == "**"
    }
}

impl fmt::Display for PathMatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.pattern)
    }
}

impl Default for PathMatcher {
    /// Default matcher that matches all paths (`/**`).
    fn default() -> Self {
        Self::new("/**")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========== Prefix Pattern Tests (user's original use case) ==========

    #[test]
    fn test_prefix_double_star() {
        let matcher = PathMatcher::new("/00**");
        assert!(matcher.matches("/00"));
        assert!(matcher.matches("/000"));
        assert!(matcher.matches("/001"));
        assert!(matcher.matches("/00abc"));
        assert!(matcher.matches("/00/child"));
        assert!(!matcher.matches("/10"));
        assert!(!matcher.matches("/a00"));
    }

    #[test]
    fn test_prefix_with_wildcard() {
        let matcher = PathMatcher::new("/art*");
        assert!(matcher.matches("/art"));
        assert!(matcher.matches("/articles"));
        assert!(matcher.matches("/artwork"));
        assert!(!matcher.matches("/art/news")); // * doesn't match /
        assert!(!matcher.matches("/bart"));
    }

    // ========== Single Wildcard (*) Tests ==========

    #[test]
    fn test_single_star_segment() {
        let matcher = PathMatcher::new("/articles/*");
        assert!(matcher.matches("/articles/news"));
        assert!(matcher.matches("/articles/sports"));
        assert!(!matcher.matches("/articles")); // Missing segment
        assert!(!matcher.matches("/articles/news/2024")); // Extra segment
    }

    #[test]
    fn test_single_star_middle() {
        let matcher = PathMatcher::new("/users/*/profile");
        assert!(matcher.matches("/users/123/profile"));
        assert!(matcher.matches("/users/alice/profile"));
        assert!(!matcher.matches("/users/profile")); // Missing segment
        assert!(!matcher.matches("/users/123/456/profile")); // Too many segments
    }

    // ========== Recursive Wildcard (**) Tests ==========

    #[test]
    fn test_recursive_double_star() {
        let matcher = PathMatcher::new("/articles/**");
        assert!(matcher.matches("/articles"));
        assert!(matcher.matches("/articles/news"));
        assert!(matcher.matches("/articles/news/2024"));
        assert!(matcher.matches("/articles/news/2024/item1"));
        assert!(!matcher.matches("/users"));
    }

    #[test]
    fn test_recursive_at_start() {
        let matcher = PathMatcher::new("/**");
        assert!(matcher.matches("/"));
        assert!(matcher.matches("/anything"));
        assert!(matcher.matches("/deep/nested/path"));
        assert!(matcher.is_unrestricted());
    }

    // ========== Middle Wildcard Tests ==========

    #[test]
    fn test_blog_anywhere() {
        let matcher = PathMatcher::new("/**/blog/**");
        assert!(matcher.matches("/blog"));
        assert!(matcher.matches("/blog/post"));
        assert!(matcher.matches("/foo/blog/post"));
        assert!(matcher.matches("/a/b/blog/x/y"));
        assert!(!matcher.matches("/articles"));
        assert!(!matcher.matches("/foo/blag/post"));
    }

    #[test]
    fn test_foo_bar_with_middle() {
        let matcher = PathMatcher::new("/foo/**/bar");
        assert!(matcher.matches("/foo/bar"));
        assert!(matcher.matches("/foo/x/bar"));
        assert!(matcher.matches("/foo/x/y/z/bar"));
        assert!(!matcher.matches("/foo"));
        assert!(!matcher.matches("/bar"));
        assert!(!matcher.matches("/foo/bar/baz"));
    }

    // ========== Exact Path Match ==========

    #[test]
    fn test_exact_path() {
        let matcher = PathMatcher::new("/users/profile");
        assert!(matcher.matches("/users/profile"));
        assert!(!matcher.matches("/users"));
        assert!(!matcher.matches("/users/profile/photo"));
    }

    // ========== Specificity Tests ==========

    #[test]
    fn test_specificity_ordering() {
        let exact = PathMatcher::new("/articles/news");
        let single = PathMatcher::new("/articles/*");
        let recursive = PathMatcher::new("/articles/**");
        let prefix = PathMatcher::new("/art**");
        let all = PathMatcher::new("/**");

        // More specific patterns should have higher scores
        assert!(exact.specificity() > single.specificity());
        assert!(single.specificity() > recursive.specificity());
        assert!(recursive.specificity() > all.specificity());
        assert!(prefix.specificity() > all.specificity());
    }

    // ========== Edge Cases ==========

    #[test]
    fn test_empty_pattern_matches_all() {
        let matcher = PathMatcher::new("");
        assert!(matcher.matches(""));
        assert!(matcher.matches("/"));
        assert!(matcher.matches("/content"));
        assert!(matcher.matches("/any/path"));
    }

    #[test]
    fn test_root_path() {
        let matcher = PathMatcher::new("/");
        // "/" normalizes to empty which becomes "**"
        assert!(matcher.matches("/"));
        assert!(matcher.matches("/content"));
    }

    #[test]
    fn test_path_without_leading_slash() {
        let matcher = PathMatcher::new("/articles/**");
        // Should match both with and without leading slash
        assert!(matcher.matches("articles/news"));
        assert!(matcher.matches("/articles/news"));
    }

    #[test]
    fn test_display() {
        let matcher = PathMatcher::new("/content/**/items");
        assert_eq!(format!("{}", matcher), "/content/**/items");
    }

    #[test]
    fn test_default() {
        let matcher = PathMatcher::default();
        assert!(matcher.matches("/any/path"));
        assert!(matcher.is_unrestricted());
    }

    // ========== Question Mark Wildcard ==========

    #[test]
    fn test_question_mark_wildcard() {
        let matcher = PathMatcher::new("/item?");
        assert!(matcher.matches("/item1"));
        assert!(matcher.matches("/itema"));
        assert!(!matcher.matches("/item"));
        assert!(!matcher.matches("/item12"));
    }

    // ========== Real World Patterns ==========

    #[test]
    fn test_content_tree_pattern() {
        // Match all content under pages
        let matcher = PathMatcher::new("/pages/**");
        assert!(matcher.matches("/pages"));
        assert!(matcher.matches("/pages/home"));
        assert!(matcher.matches("/pages/about/team"));
        assert!(!matcher.matches("/settings"));
    }

    #[test]
    fn test_user_documents_pattern() {
        // Match any user's documents
        let matcher = PathMatcher::new("/users/*/documents/**");
        assert!(matcher.matches("/users/alice/documents"));
        assert!(matcher.matches("/users/bob/documents/report.pdf"));
        assert!(!matcher.matches("/users/documents"));
        assert!(!matcher.matches("/users/alice/settings"));
    }

    // ========== Regex Conversion Tests ==========

    #[test]
    fn test_pattern_to_regex() {
        // Test prefix pattern (inline **)
        let regex = PathMatcher::pattern_to_regex("/00**");
        assert_eq!(regex, "^00.*$");

        // Test single star
        let regex = PathMatcher::pattern_to_regex("/articles/*");
        assert_eq!(regex, "^articles/[^/]*$");

        // Test trailing /** (special case - optional trailing)
        let regex = PathMatcher::pattern_to_regex("/articles/**");
        assert_eq!(regex, "^articles(/.*)?$");

        // Test mixed
        let regex = PathMatcher::pattern_to_regex("/users/*/profile");
        assert_eq!(regex, "^users/[^/]*/profile$");

        // Test /**/ in middle (special case - zero or more segments)
        let regex = PathMatcher::pattern_to_regex("/foo/**/bar");
        assert_eq!(regex, "^foo(/.*/|/)bar$");
    }
}
