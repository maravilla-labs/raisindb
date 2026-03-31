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

//! ProcessingRules V6 Migration: Nuclear delete of all corrupted entries
//!
//! This migration deletes ALL processing rules entries without attempting
//! to deserialize them. This is necessary because corrupted data with
//! `chunking: true/false` (boolean instead of struct) cannot be deserialized.
//! Users will need to recreate their rules via the UI.

use raisin_error::Result;
use rocksdb::{ColumnFamily, DB};
use std::sync::Arc;

// Changed to v6 for nuclear delete - delete ALL entries without deserializing
const MARKER_KEY: &[u8] = b"migrations/processing_rules_v6_complete";
const CF_PROCESSING_RULES: &str = "processing_rules";

/// Statistics from processing rules migration
#[derive(Debug, Clone, Default)]
pub struct ProcessingRulesMigrationStats {
    pub rules_deleted: usize,
    pub errors: usize,
}

/// Run the processing rules v6 migration (nuclear delete)
pub async fn migrate(db: Arc<DB>) -> Result<ProcessingRulesMigrationStats> {
    // Check if migration already completed
    if migration_completed(&db)? {
        tracing::info!("ProcessingRules V6 migration already completed, skipping");
        return Ok(ProcessingRulesMigrationStats::default());
    }

    tracing::info!("Starting ProcessingRules V6 migration (nuclear delete)...");
    let mut stats = ProcessingRulesMigrationStats::default();

    // Delete ALL processing rules entries
    match migrate_processing_rules(&db) {
        Ok(count) => {
            stats.rules_deleted = count;
            tracing::info!("Deleted {} processing rule entries", count);
        }
        Err(e) => {
            tracing::error!("Error during nuclear delete of processing rules: {}", e);
            stats.errors += 1;
        }
    }

    // Mark migration as complete
    mark_completed(&db)?;
    tracing::info!("ProcessingRules V6 migration marked as complete");

    Ok(stats)
}

/// Check if migration has already been completed
fn migration_completed(db: &Arc<DB>) -> Result<bool> {
    match db.get(MARKER_KEY) {
        Ok(Some(_)) => Ok(true),
        Ok(None) => Ok(false),
        Err(e) => Err(raisin_error::Error::storage(format!(
            "Failed to check migration status: {}",
            e
        ))),
    }
}

/// Mark migration as completed
fn mark_completed(db: &Arc<DB>) -> Result<()> {
    db.put(MARKER_KEY, b"1").map_err(|e| {
        raisin_error::Error::storage(format!("Failed to mark migration complete: {}", e))
    })?;
    Ok(())
}

/// Nuclear delete: Delete ALL entries in PROCESSING_RULES CF without deserializing
fn migrate_processing_rules(db: &Arc<DB>) -> Result<usize> {
    let cf = match get_cf(db, CF_PROCESSING_RULES) {
        Ok(cf) => cf,
        Err(_) => {
            // Column family doesn't exist yet - no data to migrate
            tracing::info!("PROCESSING_RULES column family not found, skipping migration");
            return Ok(0);
        }
    };

    // Collect all keys WITHOUT deserializing values
    let keys: Vec<Vec<u8>> = db
        .iterator_cf(cf, rocksdb::IteratorMode::Start)
        .filter_map(|item| item.ok().map(|(k, _)| k.to_vec()))
        .collect();

    let mut deleted = 0;

    // Delete ALL entries unconditionally
    for key in keys {
        let key_str = String::from_utf8_lossy(&key);
        tracing::info!("Deleting processing rules entry: {}", key_str);
        db.delete_cf(cf, &key)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        deleted += 1;
    }

    Ok(deleted)
}

/// Get column family handle
fn get_cf<'a>(db: &'a Arc<DB>, name: &str) -> Result<&'a ColumnFamily> {
    db.cf_handle(name)
        .ok_or_else(|| raisin_error::Error::storage(format!("Column family '{}' not found", name)))
}
