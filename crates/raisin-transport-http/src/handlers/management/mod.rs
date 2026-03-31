//! Management API handlers for three-tier architecture
//!
//! This module provides management endpoints organized into three tiers:
//! - Global: RocksDB operations affecting entire instance
//! - Tenant: Tenant-wide operations
//! - Database: Repository-specific index operations

pub mod database;
pub mod global;
pub mod tenant;

// Re-export handler functions
pub use database::*;
pub use global::*;
pub use tenant::*;
