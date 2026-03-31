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

//! Orphaned Unique Index Migration V1: Cleanup unique index entries for deleted nodes
//!
//! This migration fixes a bug where cascade delete operations did not write tombstones
//! to the UNIQUE_INDEX column family. This caused "Property must be unique" validation
//! errors when trying to create new nodes with values that were previously owned by
//! deleted nodes.
//!
//! The migration scans all UNIQUE_INDEX entries and writes tombstones for any entries
//! where the referenced node no longer exists in the NODES column family.
//!
//! ## Key Format
//! UNIQUE_INDEX: {tenant}\0{repo}\0{branch}\0{workspace}\0uniq\0{node_type}\0{property_name}\0{value_hash}\0{~revision}
//! Value: {node_id} or "T" (tombstone)

use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::{ColumnFamily, WriteBatch, DB};
use std::collections::HashSet;
use std::sync::Arc;

const MARKER_KEY: &[u8] = b"migrations/orphaned_unique_indexes_v1_complete";
const CF_UNIQUE_INDEX: &str = "unique_index";
const CF_NODES: &str = "nodes";
const TOMBSTONE: &[u8] = b"T";

/// Statistics from orphaned unique index migration
#[derive(Debug, Clone, Default)]
pub struct OrphanedUniqueIndexMigrationStats {
    pub entries_scanned: usize,
    pub orphans_found: usize,
    pub tombstones_written: usize,
    pub errors: usize,
}

/// Run the orphaned unique index cleanup migration
pub async fn migrate(db: Arc<DB>) -> Result<OrphanedUniqueIndexMigrationStats> {
    // Check if migration already completed
    if migration_completed(&db)? {
        tracing::info!("Orphaned unique index V1 migration already completed, skipping");
        return Ok(OrphanedUniqueIndexMigrationStats::default());
    }

    tracing::info!("Starting orphaned unique index V1 migration...");
    let mut stats = OrphanedUniqueIndexMigrationStats::default();

    // Run the cleanup
    match cleanup_orphaned_unique_indexes(&db) {
        Ok((scanned, orphans, written)) => {
            stats.entries_scanned = scanned;
            stats.orphans_found = orphans;
            stats.tombstones_written = written;
            tracing::info!(
                "Scanned {} unique index entries, found {} orphans, wrote {} tombstones",
                scanned,
                orphans,
                written
            );
        }
        Err(e) => {
            tracing::error!("Error cleaning up orphaned unique indexes: {}", e);
            stats.errors += 1;
        }
    }

    // Mark migration as complete
    mark_completed(&db)?;
    tracing::info!("Orphaned unique index V1 migration marked as complete");

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

/// Clean up orphaned unique index entries
///
/// Returns (entries_scanned, orphans_found, tombstones_written)
fn cleanup_orphaned_unique_indexes(db: &Arc<DB>) -> Result<(usize, usize, usize)> {
    let cf_unique = match get_cf(db, CF_UNIQUE_INDEX) {
        Ok(cf) => cf,
        Err(_) => {
            tracing::info!("UNIQUE_INDEX column family not found, skipping migration");
            return Ok((0, 0, 0));
        }
    };

    let cf_nodes = match get_cf(db, CF_NODES) {
        Ok(cf) => cf,
        Err(_) => {
            tracing::info!("NODES column family not found, skipping migration");
            return Ok((0, 0, 0));
        }
    };

    // Track unique value prefixes we've already seen to avoid duplicate tombstones
    // (multiple revisions of the same unique value)
    let mut processed_value_prefixes: HashSet<Vec<u8>> = HashSet::new();

    // Collect orphaned entries for batch tombstone writing
    let mut orphaned_entries: Vec<OrphanedEntry> = Vec::new();
    let mut entries_scanned = 0;

    // Iterate through all UNIQUE_INDEX entries
    let iter = db.iterator_cf(cf_unique, rocksdb::IteratorMode::Start);

    for item in iter {
        let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        entries_scanned += 1;

        // Skip tombstone entries
        if is_tombstone(&value) {
            continue;
        }

        // Parse the key to extract context and value prefix
        let parsed = match parse_unique_index_key(&key) {
            Some(p) => p,
            None => {
                tracing::debug!("Skipping unparseable unique index key");
                continue;
            }
        };

        // Skip if we've already processed this value prefix (newer revision takes precedence)
        if processed_value_prefixes.contains(&parsed.value_prefix) {
            continue;
        }

        // Extract the node_id that supposedly owns this unique value
        let owning_node_id = String::from_utf8_lossy(&value).to_string();

        // Check if this node still exists
        if !node_exists(db, cf_nodes, &parsed, &owning_node_id)? {
            tracing::debug!(
                "Found orphaned unique index: node_type={}, property={}, value_hash={}, node_id={}",
                parsed.node_type,
                parsed.property_name,
                parsed.value_hash,
                owning_node_id
            );

            orphaned_entries.push(OrphanedEntry {
                tenant_id: parsed.tenant_id.clone(),
                repo_id: parsed.repo_id.clone(),
                branch: parsed.branch.clone(),
                workspace: parsed.workspace.clone(),
                node_type: parsed.node_type.clone(),
                property_name: parsed.property_name.clone(),
                value_hash: parsed.value_hash.clone(),
            });
        }

        // Mark this value prefix as processed
        processed_value_prefixes.insert(parsed.value_prefix);
    }

    let orphans_found = orphaned_entries.len();

    // Write tombstones for all orphaned entries
    let tombstones_written = write_orphan_tombstones(db, cf_unique, &orphaned_entries)?;

    Ok((entries_scanned, orphans_found, tombstones_written))
}

/// Parsed components from a UNIQUE_INDEX key
struct ParsedUniqueIndexKey {
    tenant_id: String,
    repo_id: String,
    branch: String,
    workspace: String,
    node_type: String,
    property_name: String,
    value_hash: String,
    /// The key prefix up to (but not including) the revision - used for deduplication
    value_prefix: Vec<u8>,
}

/// Parse a UNIQUE_INDEX key into its components
///
/// Key format: {tenant}\0{repo}\0{branch}\0{workspace}\0uniq\0{node_type}\0{property_name}\0{value_hash}\0{~revision}
fn parse_unique_index_key(key: &[u8]) -> Option<ParsedUniqueIndexKey> {
    let parts: Vec<&[u8]> = key.split(|&b| b == 0).collect();

    // Expected: tenant, repo, branch, workspace, "uniq", node_type, property_name, value_hash, revision
    if parts.len() < 9 {
        return None;
    }

    // Verify the "uniq" tag
    let tag = String::from_utf8_lossy(parts[4]);
    if tag != "uniq" {
        return None;
    }

    let tenant_id = String::from_utf8_lossy(parts[0]).to_string();
    let repo_id = String::from_utf8_lossy(parts[1]).to_string();
    let branch = String::from_utf8_lossy(parts[2]).to_string();
    let workspace = String::from_utf8_lossy(parts[3]).to_string();
    let node_type = String::from_utf8_lossy(parts[5]).to_string();
    let property_name = String::from_utf8_lossy(parts[6]).to_string();
    let value_hash = String::from_utf8_lossy(parts[7]).to_string();

    // Build value prefix (everything except the revision)
    // This is used to deduplicate - we only need to check the latest revision
    let mut value_prefix = Vec::new();
    for (i, part) in parts.iter().enumerate().take(8) {
        if i > 0 {
            value_prefix.push(0);
        }
        value_prefix.extend_from_slice(part);
    }

    Some(ParsedUniqueIndexKey {
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_type,
        property_name,
        value_hash,
        value_prefix,
    })
}

/// Check if a node exists in the NODES column family
fn node_exists(
    db: &Arc<DB>,
    cf_nodes: &ColumnFamily,
    parsed: &ParsedUniqueIndexKey,
    node_id: &str,
) -> Result<bool> {
    // Build the prefix for this node's entries
    // NODES key format: {tenant}\0{repo}\0{branch}\0{workspace}\0nodes\0{node_id}\0{~revision}
    let prefix = format!(
        "{}\0{}\0{}\0{}\0nodes\0{}\0",
        parsed.tenant_id, parsed.repo_id, parsed.branch, parsed.workspace, node_id
    );

    let mut iter = db.prefix_iterator_cf(cf_nodes, prefix.as_bytes());

    if let Some(item) = iter.next() {
        let (key, value) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        // Verify we're still in our prefix
        if !key.starts_with(prefix.as_bytes()) {
            return Ok(false);
        }

        // If the latest entry is NOT a tombstone, the node exists
        if !is_tombstone(&value) {
            return Ok(true);
        }

        // If tombstone is the latest, node is deleted
        return Ok(false);
    }

    // No entries found - node doesn't exist
    Ok(false)
}

/// Entry representing an orphaned unique index that needs a tombstone
struct OrphanedEntry {
    tenant_id: String,
    repo_id: String,
    branch: String,
    workspace: String,
    node_type: String,
    property_name: String,
    value_hash: String,
}

/// Write tombstones for all orphaned entries
fn write_orphan_tombstones(
    db: &Arc<DB>,
    cf_unique: &ColumnFamily,
    orphaned_entries: &[OrphanedEntry],
) -> Result<usize> {
    if orphaned_entries.is_empty() {
        return Ok(0);
    }

    let mut batch = WriteBatch::default();
    let revision = HLC::now();

    for entry in orphaned_entries {
        let key = build_unique_index_key(
            &entry.tenant_id,
            &entry.repo_id,
            &entry.branch,
            &entry.workspace,
            &entry.node_type,
            &entry.property_name,
            &entry.value_hash,
            &revision,
        );

        batch.put_cf(cf_unique, key, TOMBSTONE);

        tracing::info!(
            "Writing tombstone for orphaned unique index: {}:{}.{} = {}",
            entry.node_type,
            entry.property_name,
            entry.value_hash,
            entry.tenant_id
        );
    }

    let count = orphaned_entries.len();

    db.write(batch)
        .map_err(|e| raisin_error::Error::storage(format!("Failed to write tombstones: {}", e)))?;

    Ok(count)
}

/// Build a UNIQUE_INDEX key
///
/// Key format: {tenant}\0{repo}\0{branch}\0{workspace}\0uniq\0{node_type}\0{property_name}\0{value_hash}\0{~revision}
fn build_unique_index_key(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_type: &str,
    property_name: &str,
    value_hash: &str,
    revision: &HLC,
) -> Vec<u8> {
    let mut key = Vec::new();

    key.extend_from_slice(tenant_id.as_bytes());
    key.push(0);
    key.extend_from_slice(repo_id.as_bytes());
    key.push(0);
    key.extend_from_slice(branch.as_bytes());
    key.push(0);
    key.extend_from_slice(workspace.as_bytes());
    key.push(0);
    key.extend_from_slice(b"uniq");
    key.push(0);
    key.extend_from_slice(node_type.as_bytes());
    key.push(0);
    key.extend_from_slice(property_name.as_bytes());
    key.push(0);
    key.extend_from_slice(value_hash.as_bytes());
    key.push(0);
    key.extend_from_slice(&encode_descending_revision(revision));

    key
}

/// Encode HLC as descending bytes for newest-first iteration
fn encode_descending_revision(hlc: &HLC) -> Vec<u8> {
    hlc.encode_descending().to_vec()
}

/// Check if a value is a tombstone marker
#[inline]
fn is_tombstone(value: &[u8]) -> bool {
    value == TOMBSTONE || value.is_empty()
}

/// Get column family handle
fn get_cf<'a>(db: &'a Arc<DB>, name: &str) -> Result<&'a ColumnFamily> {
    db.cf_handle(name)
        .ok_or_else(|| raisin_error::Error::storage(format!("Column family '{}' not found", name)))
}
