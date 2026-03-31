// SPDX-License-Identifier: BSL-1.1

//! Path filtering and glob matching for sync configuration.

use super::{ConflictStrategy, SyncConfig, SyncDirection, SyncMode};

impl SyncConfig {
    /// Get the effective sync mode for a path
    pub fn get_mode_for_path(&self, path: &str) -> SyncMode {
        // Find last matching filter (last match wins)
        for filter in self.filters.iter().rev() {
            if path.starts_with(&filter.root) {
                if let Some(mode) = filter.mode {
                    return mode;
                }
            }
        }
        self.defaults.mode
    }

    /// Get the effective sync direction for a path
    pub fn get_direction_for_path(&self, path: &str) -> SyncDirection {
        for filter in self.filters.iter().rev() {
            if path.starts_with(&filter.root) {
                if let Some(direction) = filter.direction {
                    return direction;
                }
            }
        }
        SyncDirection::Bidirectional
    }

    /// Get the effective conflict strategy for a path
    pub fn get_conflict_strategy_for_path(&self, path: &str) -> ConflictStrategy {
        // First check explicit conflict overrides
        if let Some(override_config) = self.conflicts.get(path) {
            return override_config.strategy;
        }

        // Then check filters
        for filter in self.filters.iter().rev() {
            if path.starts_with(&filter.root) {
                if let Some(strategy) = filter.on_conflict {
                    return strategy;
                }
            }
        }

        self.defaults.on_conflict
    }

    /// Check if a path should be synced based on include/exclude patterns
    pub fn should_sync_path(&self, path: &str) -> bool {
        for filter in self.filters.iter().rev() {
            if path.starts_with(&filter.root) {
                // Check direction first
                if let Some(SyncDirection::LocalOnly) = filter.direction {
                    return false;
                }

                let relative_path = path.strip_prefix(&filter.root).unwrap_or(path);
                let relative_path = relative_path.trim_start_matches('/');

                // If include patterns exist, path must match at least one
                if !filter.include.is_empty() {
                    let matches_include = filter
                        .include
                        .iter()
                        .any(|pattern| glob_matches(pattern, relative_path));
                    if !matches_include {
                        return false;
                    }
                }

                // Path must not match any exclude pattern
                if filter
                    .exclude
                    .iter()
                    .any(|pattern| glob_matches(pattern, relative_path))
                {
                    return false;
                }

                return true;
            }
        }

        // No matching filter, allow by default
        true
    }
}

/// Simple glob pattern matching (supports *, **, ?)
pub(crate) fn glob_matches(pattern: &str, path: &str) -> bool {
    // Convert glob pattern to regex-like matching
    let pattern_parts: Vec<&str> = pattern.split('/').collect();
    let path_parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    glob_matches_parts(&pattern_parts, &path_parts)
}

fn glob_matches_parts(pattern_parts: &[&str], path_parts: &[&str]) -> bool {
    if pattern_parts.is_empty() {
        return path_parts.is_empty();
    }

    let pattern = pattern_parts[0];

    if pattern == "**" {
        // ** matches zero or more path segments
        if pattern_parts.len() == 1 {
            return true; // ** at end matches everything
        }

        // Try matching ** with 0, 1, 2, ... path segments
        for i in 0..=path_parts.len() {
            if glob_matches_parts(&pattern_parts[1..], &path_parts[i..]) {
                return true;
            }
        }
        return false;
    }

    if path_parts.is_empty() {
        return false;
    }

    // Match single segment
    if segment_matches(pattern, path_parts[0]) {
        glob_matches_parts(&pattern_parts[1..], &path_parts[1..])
    } else {
        false
    }
}

fn segment_matches(pattern: &str, segment: &str) -> bool {
    if pattern == "*" {
        return true;
    }

    let mut pattern_chars = pattern.chars().peekable();
    let mut segment_chars = segment.chars().peekable();

    while let Some(p) = pattern_chars.next() {
        match p {
            '*' => {
                // * matches any sequence
                if pattern_chars.peek().is_none() {
                    return true;
                }
                // Try matching * with 0, 1, 2, ... characters
                let remaining_pattern: String = pattern_chars.collect();
                let remaining_segment: String = segment_chars.collect();
                for i in 0..=remaining_segment.len() {
                    if segment_matches(&remaining_pattern, &remaining_segment[i..]) {
                        return true;
                    }
                }
                return false;
            }
            '?' => {
                if segment_chars.next().is_none() {
                    return false;
                }
            }
            c => {
                if segment_chars.next() != Some(c) {
                    return false;
                }
            }
        }
    }

    segment_chars.peek().is_none()
}
