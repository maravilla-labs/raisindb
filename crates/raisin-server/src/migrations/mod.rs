// SPDX-License-Identifier: UNLICENSED
//
// Copyright (C) 2019-2025 SOLUTAS GmbH, All Rights Reserved.
//
// Paradieshofstrasse 117, 4054 Basel, Switzerland
// http://www.solutas.ch | info@solutas.ch
//
// This file is part of RaisinDB.
//
// Unauthorized copying of this file, via any medium is strictly prohibited
// Proprietary and confidential

//! Database migrations for schema evolution
//!
//! This module coordinates all database migrations that need to run on server startup.
//! Each migration runs only once and tracks completion status.

mod fix_orphaned_unique_indexes_v1;
mod fix_processing_rules_v1;
mod fix_schema_v1;
mod purge_orphaned_jobs_v1;

use raisin_error::Result;
use rocksdb::DB;
use std::sync::Arc;

/// Run all pending migrations in order
///
/// This should be called once during server startup, after opening the database
/// but before starting to serve requests.
///
/// # Arguments
/// * `db` - Arc reference to the RocksDB instance
///
/// # Returns
/// Ok(()) if all migrations completed successfully, Err if any migration failed
pub async fn run_migrations(db: Arc<DB>) -> Result<()> {
    tracing::info!("🔄 Starting database migrations...");

    // Run schema v1 fix migration
    match fix_schema_v1::migrate(db.clone()).await {
        Ok(stats) => {
            tracing::info!(
                "✅ Schema V1 migration completed: {} nodes fixed, {} revisions fixed, {} errors",
                stats.nodes_fixed,
                stats.revisions_fixed,
                stats.errors
            );
        }
        Err(e) => {
            tracing::error!("❌ Schema V1 migration failed: {}", e);
            return Err(e);
        }
    }

    // Run processing rules v6 migration (nuclear delete)
    match fix_processing_rules_v1::migrate(db.clone()).await {
        Ok(stats) => {
            tracing::info!(
                "✅ ProcessingRules V6 migration completed: {} rules deleted, {} errors",
                stats.rules_deleted,
                stats.errors
            );
        }
        Err(e) => {
            tracing::error!("❌ ProcessingRules V6 migration failed: {}", e);
            return Err(e);
        }
    }

    // Run orphaned unique index cleanup migration
    match fix_orphaned_unique_indexes_v1::migrate(db.clone()).await {
        Ok(stats) => {
            tracing::info!(
                "✅ Orphaned unique index V1 migration completed: {} scanned, {} orphans found, {} tombstones written, {} errors",
                stats.entries_scanned,
                stats.orphans_found,
                stats.tombstones_written,
                stats.errors
            );
        }
        Err(e) => {
            tracing::error!("❌ Orphaned unique index V1 migration failed: {}", e);
            return Err(e);
        }
    }

    // Purge orphaned jobs (runs every startup, idempotent)
    match purge_orphaned_jobs_v1::migrate(db.clone()).await {
        Ok(stats) => {
            if stats.orphans_purged > 0 {
                tracing::info!(
                    "✅ Orphaned jobs purge: scanned {} entries, purged {} orphans, {} errors",
                    stats.entries_scanned,
                    stats.orphans_purged,
                    stats.errors
                );
            } else {
                tracing::debug!(
                    "Orphaned jobs purge: scanned {} entries, no orphans found",
                    stats.entries_scanned
                );
            }
        }
        Err(e) => {
            tracing::error!("❌ Orphaned jobs purge failed: {}", e);
            // Non-fatal: don't block startup
        }
    }

    tracing::info!("✅ All migrations completed successfully");
    Ok(())
}

/// Statistics from a migration run
#[derive(Debug, Clone, Default)]
pub struct MigrationStats {
    pub nodes_fixed: usize,
    pub revisions_fixed: usize,
    pub errors: usize,
}
