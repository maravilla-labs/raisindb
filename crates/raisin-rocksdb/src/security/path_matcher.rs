//! Path pattern matching for permission rules.
//!
//! Supports glob-style patterns:
//! - `**` - matches any number of path segments (recursive)
//! - `*` - matches a single path segment
//! - Exact segment names match exactly
//!
//! # Examples
//!
//! - `content.articles.**` matches `/content/articles/`, `/content/articles/news/item1`
//! - `users.*.profile` matches `/users/123/profile`, `/users/abc/profile`
//! - `blog.posts` matches only `/blog/posts`

/// Compiled path pattern for efficient matching.
#[derive(Debug, Clone)]
pub struct PathMatcher {
    segments: Vec<PatternSegment>,
}

#[derive(Debug, Clone)]
enum PatternSegment {
    /// Matches exactly this segment name
    Exact(String),
    /// Matches any single segment (*)
    Wildcard,
    /// Matches zero or more segments (**)
    RecursiveWildcard,
}

impl PathMatcher {
    /// Create a new path matcher from a pattern string.
    ///
    /// Pattern format uses `.` as separator:
    /// - `content.articles` -> matches `/content/articles`
    /// - `content.articles.**` -> matches `/content/articles` and all descendants
    /// - `users.*.profile` -> matches `/users/{any}/profile`
    pub fn new(pattern: &str) -> Self {
        let segments = pattern
            .split('.')
            .filter(|s| !s.is_empty())
            .map(|s| match s {
                "**" => PatternSegment::RecursiveWildcard,
                "*" => PatternSegment::Wildcard,
                _ => PatternSegment::Exact(s.to_string()),
            })
            .collect();

        Self { segments }
    }

    /// Check if a node path matches this pattern.
    ///
    /// The path should be in `/segment1/segment2` format.
    pub fn matches(&self, path: &str) -> bool {
        let path_segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

        match_segments(&self.segments, &path_segments)
    }
}

/// Check if a path matches a pattern (convenience function).
///
/// # Arguments
///
/// * `pattern` - Permission path pattern (e.g., "content.articles.**")
/// * `path` - Node path (e.g., "/content/articles/news")
///
/// # Returns
///
/// true if the path matches the pattern
pub fn matches_path_pattern(pattern: &str, path: &str) -> bool {
    let matcher = PathMatcher::new(pattern);
    matcher.matches(path)
}

/// Recursive segment matching implementation.
fn match_segments(pattern: &[PatternSegment], path: &[&str]) -> bool {
    match (pattern.first(), path.first()) {
        // Both empty - match!
        (None, None) => true,

        // Pattern empty but path has more - no match
        (None, Some(_)) => false,

        // Pattern has ** - try all possible matches
        (Some(PatternSegment::RecursiveWildcard), _) => {
            // ** can match zero segments
            if match_segments(&pattern[1..], path) {
                return true;
            }
            // ** can match one or more segments
            if !path.is_empty() && match_segments(pattern, &path[1..]) {
                return true;
            }
            false
        }

        // Path empty but pattern still has segments
        (Some(_), None) => {
            // Only valid if remaining pattern is all **
            pattern
                .iter()
                .all(|p| matches!(p, PatternSegment::RecursiveWildcard))
        }

        // Both have segments - check current segment
        (Some(PatternSegment::Exact(expected)), Some(actual)) => {
            expected == *actual && match_segments(&pattern[1..], &path[1..])
        }

        // Wildcard matches any single segment
        (Some(PatternSegment::Wildcard), Some(_)) => match_segments(&pattern[1..], &path[1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        assert!(matches_path_pattern(
            "content.articles",
            "/content/articles"
        ));
        assert!(!matches_path_pattern(
            "content.articles",
            "/content/articles/news"
        ));
        assert!(!matches_path_pattern("content.articles", "/content"));
    }

    #[test]
    fn test_recursive_wildcard() {
        // ** at end matches all descendants
        assert!(matches_path_pattern(
            "content.articles.**",
            "/content/articles"
        ));
        assert!(matches_path_pattern(
            "content.articles.**",
            "/content/articles/news"
        ));
        assert!(matches_path_pattern(
            "content.articles.**",
            "/content/articles/news/item1"
        ));

        // ** doesn't match non-prefix paths
        assert!(!matches_path_pattern("content.articles.**", "/content"));
        assert!(!matches_path_pattern(
            "content.articles.**",
            "/other/articles"
        ));
    }

    #[test]
    fn test_single_wildcard() {
        // * matches exactly one segment
        assert!(matches_path_pattern(
            "users.*.profile",
            "/users/123/profile"
        ));
        assert!(matches_path_pattern(
            "users.*.profile",
            "/users/abc/profile"
        ));
        assert!(!matches_path_pattern(
            "users.*.profile",
            "/users/123/456/profile"
        ));
        assert!(!matches_path_pattern("users.*.profile", "/users/profile"));
    }

    #[test]
    fn test_root_pattern() {
        // ** at root matches everything
        assert!(matches_path_pattern("**", "/"));
        assert!(matches_path_pattern("**", "/anything"));
        assert!(matches_path_pattern("**", "/anything/deep/nested"));
    }

    #[test]
    fn test_mixed_patterns() {
        // Combination of exact, *, and **
        assert!(matches_path_pattern(
            "content.*.posts.**",
            "/content/blog/posts"
        ));
        assert!(matches_path_pattern(
            "content.*.posts.**",
            "/content/news/posts/item"
        ));
        assert!(!matches_path_pattern(
            "content.*.posts.**",
            "/content/posts"
        ));
    }

    #[test]
    fn test_empty_path() {
        assert!(matches_path_pattern("**", "/"));
        assert!(!matches_path_pattern("content", "/"));
    }
}
