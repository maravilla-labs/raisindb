// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Global dependency availability flags for runtime checks.

use std::collections::HashSet;
use std::sync::{LazyLock, RwLock};

/// Global flags tracking which dependencies are available at runtime.
///
/// This is used by API handlers to quickly check if a dependency
/// is available before attempting to use it.
///
/// # Example
///
/// ```rust,ignore
/// use raisin_deps::DEPENDENCY_FLAGS;
///
/// if !DEPENDENCY_FLAGS.is_available("tesseract") {
///     return Err(ApiError::DependencyNotAvailable("tesseract"));
/// }
/// ```
pub static DEPENDENCY_FLAGS: LazyLock<DependencyFlags> = LazyLock::new(DependencyFlags::new);

/// Thread-safe container for dependency availability flags.
#[derive(Debug)]
pub struct DependencyFlags {
    /// Set of unavailable dependencies
    unavailable: RwLock<HashSet<String>>,
    /// Set of dependencies that were skipped during setup
    skipped: RwLock<HashSet<String>>,
}

impl DependencyFlags {
    /// Create a new DependencyFlags instance.
    pub fn new() -> Self {
        Self {
            unavailable: RwLock::new(HashSet::new()),
            skipped: RwLock::new(HashSet::new()),
        }
    }

    /// Check if a dependency is available (installed and working).
    pub fn is_available(&self, name: &str) -> bool {
        let unavailable = self.unavailable.read().unwrap();
        !unavailable.contains(name)
    }

    /// Check if a dependency was explicitly skipped during setup.
    pub fn is_skipped(&self, name: &str) -> bool {
        let skipped = self.skipped.read().unwrap();
        skipped.contains(name)
    }

    /// Mark a dependency as unavailable.
    pub fn set_unavailable(&self, name: &str) {
        let mut unavailable = self.unavailable.write().unwrap();
        unavailable.insert(name.to_string());
    }

    /// Mark a dependency as available (remove from unavailable set).
    pub fn set_available(&self, name: &str) {
        let mut unavailable = self.unavailable.write().unwrap();
        unavailable.remove(name);

        // Also remove from skipped if it was there
        let mut skipped = self.skipped.write().unwrap();
        skipped.remove(name);
    }

    /// Mark a dependency as skipped (user chose not to install).
    pub fn set_skipped(&self, name: &str) {
        let mut skipped = self.skipped.write().unwrap();
        skipped.insert(name.to_string());

        // Also mark as unavailable
        let mut unavailable = self.unavailable.write().unwrap();
        unavailable.insert(name.to_string());
    }

    /// Get list of all unavailable dependencies.
    pub fn get_unavailable(&self) -> Vec<String> {
        let unavailable = self.unavailable.read().unwrap();
        unavailable.iter().cloned().collect()
    }

    /// Get list of all skipped dependencies.
    pub fn get_skipped(&self) -> Vec<String> {
        let skipped = self.skipped.read().unwrap();
        skipped.iter().cloned().collect()
    }

    /// Reset all flags (for testing).
    #[cfg(test)]
    pub fn reset(&self) {
        let mut unavailable = self.unavailable.write().unwrap();
        unavailable.clear();

        let mut skipped = self.skipped.write().unwrap();
        skipped.clear();
    }
}

impl Default for DependencyFlags {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dependency_flags() {
        let flags = DependencyFlags::new();

        // Initially all available
        assert!(flags.is_available("tesseract"));
        assert!(!flags.is_skipped("tesseract"));

        // Mark as unavailable
        flags.set_unavailable("tesseract");
        assert!(!flags.is_available("tesseract"));
        assert!(!flags.is_skipped("tesseract"));

        // Mark as skipped
        flags.set_skipped("ffmpeg");
        assert!(!flags.is_available("ffmpeg"));
        assert!(flags.is_skipped("ffmpeg"));

        // Mark as available again
        flags.set_available("tesseract");
        assert!(flags.is_available("tesseract"));
    }

    #[test]
    fn test_set_available_clears_skipped() {
        let flags = DependencyFlags::new();

        flags.set_skipped("tesseract");
        assert!(flags.is_skipped("tesseract"));
        assert!(!flags.is_available("tesseract"));

        flags.set_available("tesseract");
        assert!(!flags.is_skipped("tesseract"));
        assert!(flags.is_available("tesseract"));
    }
}
