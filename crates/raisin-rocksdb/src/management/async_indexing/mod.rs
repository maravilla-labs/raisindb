//! Async index rebuilding for repositories
//!
//! This module provides repository-scoped index rebuilding operations:
//! - Rebuild path indexes (path -> node_id mappings)
//! - Rebuild property indexes (property value -> node_id mappings)
//! - Rebuild reference indexes (forward and reverse reference mappings)
//! - Orphaned index cleanup
//!
//! All operations are scoped to tenant/repository/branch/workspace for proper isolation.

mod helpers;
mod orphan_cleanup;
mod rebuild;

// Re-export the public API (unchanged from original single-file module)
pub use orphan_cleanup::{cleanup_orphaned_property_indexes, OrphanedIndexCleanupStats};
pub use rebuild::rebuild_indexes;
