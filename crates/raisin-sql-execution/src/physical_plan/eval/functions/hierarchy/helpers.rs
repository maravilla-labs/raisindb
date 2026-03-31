//! Helper functions for hierarchical path operations
//!
//! This module provides shared utility functions for working with hierarchical paths.
//! These functions are used internally by hierarchy functions but are not exposed as
//! SQL functions directly.

/// Returns the ancestor of the path at the specified absolute depth from root.
///
/// This function truncates the path to a specific depth level, counting from the root.
/// For example, for path "/a/b/c/d" with depth 2, it returns "/a/b".
///
/// # Arguments
/// * `path` - The full path to process
/// * `depth` - The absolute depth level from root (0 = root, 1 = first level, etc.)
///
/// # Returns
/// * The path truncated to the specified depth
/// * Empty string if the requested depth exceeds the path depth
///
/// # Examples
/// ```
/// # use raisin_sql::physical_plan::eval::functions::hierarchy::helpers::get_ancestor;
/// assert_eq!(get_ancestor("/a/b/c/d", 2), "/a/b");
/// assert_eq!(get_ancestor("/a/b/c/d", 1), "/a");
/// assert_eq!(get_ancestor("/a/b/c/d", 0), "");
/// assert_eq!(get_ancestor("/a/b/c/d", 10), ""); // depth too high
/// ```
///
/// # Performance
/// Uses iterator-based approach to minimize allocations
pub fn get_ancestor(path: &str, depth: i32) -> String {
    if depth <= 0 {
        return String::new();
    }

    // Iterator-based approach: find the nth '/' and take substring
    // This avoids multiple string slicing operations
    path.char_indices()
        .filter(|(_, ch)| *ch == '/')
        .nth(depth as usize)
        .map_or_else(
            || {
                // Not enough slashes - check if we have exactly depth segments
                let segment_count = path.split('/').filter(|s| !s.is_empty()).count();
                if segment_count == depth as usize {
                    path.to_string()
                } else {
                    String::new()
                }
            },
            |(idx, _)| path[..idx].to_string(),
        )
}

/// Returns the parent of the path by going N levels up from the current position.
///
/// This function works backwards from the end of the path, removing N levels.
/// For example, for path "/a/b/c/d" with levels=1, it returns "/a/b/c" (immediate parent).
///
/// # Arguments
/// * `path` - The full path to process
/// * `levels` - Number of levels to go up (1 = immediate parent, 2 = grandparent, etc.)
///
/// # Returns
/// * The parent path after going up the specified number of levels
/// * Empty string if levels exceeds the path depth
/// * "/" if going up would reach the root
///
/// # Examples
/// ```
/// # use raisin_sql::physical_plan::eval::functions::hierarchy::helpers::get_parent_at_level;
/// assert_eq!(get_parent_at_level("/a/b/c/d", 1), "/a/b/c");
/// assert_eq!(get_parent_at_level("/a/b/c/d", 2), "/a/b");
/// assert_eq!(get_parent_at_level("/a", 1), "/");
/// assert_eq!(get_parent_at_level("/a/b", 10), ""); // levels too high
/// ```
///
/// # Performance
/// Uses iterator-based reverse search
pub fn get_parent_at_level(path: &str, levels: usize) -> String {
    // Special case: root has no parent
    if path == "/" {
        return String::new();
    }

    // Iterator-based approach: find the Nth '/' from the end
    path.char_indices()
        .rev()
        .filter(|(_, ch)| *ch == '/')
        .nth(levels.saturating_sub(1))
        .map_or_else(String::new, |(idx, _)| {
            if idx == 0 {
                "/".to_string()
            } else {
                path[..idx].to_string()
            }
        })
}
