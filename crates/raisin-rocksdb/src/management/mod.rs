//! Management operations for RocksDBStorage
//!
//! This module provides production-grade operational features:
//! - Integrity checking and verification
//! - Index rebuilding and maintenance
//! - Backup and restore
//! - Health monitoring and metrics
//! - Background jobs for automated maintenance
//!
//! # Module Organization
//!
//! - `metrics` - Health checks and metrics collection
//! - `integrity` - Integrity checking
//! - `async_indexing` - Index operations
//! - `compaction` - Revision compaction
//! - `backup` - Backup/restore
//! - `background` - Background job orchestration
//! - `helpers` - Tenant/repo/branch/workspace enumeration

// Trait implementations
mod background_jobs_impl;
mod management_ops;

// Internal modules
mod helpers;
mod metrics;

// Public submodules
pub mod async_indexing;
pub mod backup;
pub mod compaction;
pub mod integrity;

// Background job orchestration
pub mod background;

// One-time migration module (temporary)
pub mod format_migration;

// Vector index management
pub mod vector;

// Re-exports for direct imports
pub use background::{
    BackgroundJobStats, BackgroundJobs as BackgroundJobsImpl, BackgroundJobsConfig,
};
pub use helpers::{list_branches, list_repositories, list_tenants, list_workspaces};
pub use vector::{
    DimensionMismatch, HnswManagement, RebuildStats as VectorRebuildStats, VerificationReport,
};
