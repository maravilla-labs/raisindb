// SPDX-License-Identifier: BSL-1.1

//! Path matching with wildcard support.
//!
//! Supports: `*` (single level), `**` (recursive), `%` (SQL LIKE style - any chars)

/// Path matching with wildcard support
pub(crate) fn path_matches(path: &str, pattern: &str) -> bool {
    // Handle exact match
    if path == pattern {
        tracing::debug!("Path match: exact - path={}, pattern={}", path, pattern);
        return true;
    }

    // Handle SQL-style % wildcard (treat like shell glob *)
    let normalized_pattern = if pattern.contains('%') {
        let converted = pattern.replace('%', "*");
        tracing::debug!(
            "Converting SQL-style wildcard: {} -> {}",
            pattern,
            converted
        );
        converted
    } else {
        pattern.to_string()
    };

    // Handle patterns with wildcards (* or **)
    if normalized_pattern.contains('*') {
        let matches = glob_match(&normalized_pattern, path);
        tracing::debug!(
            "Path match: glob pattern - path={}, pattern={}, normalized={}, matches={}",
            path,
            pattern,
            normalized_pattern,
            matches
        );
        return matches;
    }

    tracing::debug!("Path match: no match - path={}, pattern={}", path, pattern);
    false
}

/// Simple glob matching for path patterns
/// Supports * (match any chars until next /) and ** (match any chars including /)
fn glob_match(pattern: &str, path: &str) -> bool {
    let pattern_parts: Vec<&str> = pattern.split('/').collect();
    let path_parts: Vec<&str> = path.split('/').collect();

    let mut p_idx = 0;
    let mut path_idx = 0;

    while p_idx < pattern_parts.len() && path_idx < path_parts.len() {
        let p_part = pattern_parts[p_idx];

        if p_part == "**" {
            if p_idx == pattern_parts.len() - 1 {
                return true;
            }

            for i in path_idx..=path_parts.len() {
                if glob_match_from(&pattern_parts[p_idx + 1..], &path_parts[i..]) {
                    return true;
                }
            }
            return false;
        } else if p_part == "*" || p_part == path_parts[path_idx] {
            path_idx += 1;
            p_idx += 1;
        } else {
            return false;
        }
    }

    p_idx == pattern_parts.len() && path_idx == path_parts.len()
}

/// Helper function for glob_match to match from specific positions
fn glob_match_from(pattern_parts: &[&str], path_parts: &[&str]) -> bool {
    let mut p_idx = 0;
    let mut path_idx = 0;

    while p_idx < pattern_parts.len() && path_idx < path_parts.len() {
        let p_part = pattern_parts[p_idx];

        if p_part == "**" {
            if p_idx == pattern_parts.len() - 1 {
                return true;
            }
            for i in path_idx..=path_parts.len() {
                if glob_match_from(&pattern_parts[p_idx + 1..], &path_parts[i..]) {
                    return true;
                }
            }
            return false;
        } else if p_part == "*" || p_part == path_parts[path_idx] {
            path_idx += 1;
            p_idx += 1;
        } else {
            return false;
        }
    }

    p_idx == pattern_parts.len() && path_idx == path_parts.len()
}
