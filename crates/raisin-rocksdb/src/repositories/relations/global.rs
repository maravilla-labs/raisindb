//! Global relation index scanning
//!
//! This module implements scan_relations_global for cross-workspace
//! Cypher queries using the global relation index.

use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use raisin_models::nodes::FullRelation;
use rocksdb::DB;
use std::collections::HashSet;
use std::sync::Arc;

use crate::keys::{relation_global_prefix, relation_global_type_prefix};

use super::helpers::{deserialize_full_relation, get_relation_cf, is_tombstone, parse_global_key};

/// Scan all relations in the global index, optionally filtered by relation type
pub(super) async fn scan_relations_global(
    db: &Arc<DB>,
    max_revision: &HLC,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    relation_type_filter: Option<&str>,
) -> Result<Vec<(String, String, String, String, FullRelation)>> {
    tracing::info!("🔵 RocksDB scan_relations_global");
    tracing::debug!(
        "   tenant={}, repo={}, branch={}, type_filter={:?}",
        tenant_id,
        repo_id,
        branch,
        relation_type_filter
    );
    tracing::debug!("   max_revision={:?}", max_revision);

    // Choose prefix based on whether we're filtering by type
    let prefix = match relation_type_filter {
        Some(rel_type) => {
            tracing::debug!("   Using type-specific prefix for: {}", rel_type);
            relation_global_type_prefix(tenant_id, repo_id, branch, rel_type)
        }
        None => {
            tracing::debug!("   Using global prefix (all types)");
            relation_global_prefix(tenant_id, repo_id, branch)
        }
    };

    tracing::debug!("   Prefix scan: {} bytes", prefix.len());

    // Get relation column family handle
    let cf_relation = get_relation_cf(db)?;

    let mut relations = Vec::new();
    let mut seen_relations = HashSet::new();
    let mut stats = ScanStats::default();

    // Scan global index
    let iter = db.prefix_iterator_cf(cf_relation, &prefix);
    for item in iter {
        stats.scanned_keys += 1;

        let (key, value) =
            item.map_err(|e| Error::storage(format!("Failed to iterate relations: {}", e)))?;

        // Check if key still matches prefix
        if !key.starts_with(&prefix) {
            break;
        }

        // Parse the key to extract components
        let components = match parse_global_key(&key) {
            Ok(c) => c,
            Err(e) => {
                stats.skipped_invalid += 1;
                tracing::warn!("   Invalid key format: {}", e);
                continue;
            }
        };

        // Skip if revision is newer than max_revision
        if &components.revision > max_revision {
            stats.skipped_revision += 1;
            continue;
        }

        // Skip tombstones (empty values)
        if is_tombstone(&value) {
            stats.skipped_tombstone += 1;
            continue;
        }

        // Create unique key for this relationship to detect duplicates
        // We only want the newest revision of each relationship
        let rel_key = format!(
            "{}:{}:{}:{}:{}",
            components.relation_type,
            components.source_workspace,
            components.source_id,
            components.target_workspace,
            components.target_id
        );

        // Skip if we've already seen this relationship (we only want newest revision)
        if seen_relations.contains(&rel_key) {
            stats.skipped_duplicate += 1;
            continue;
        }

        // Deserialize the FullRelation
        let full_relation = deserialize_full_relation(&value)?;

        // Return FullRelation for graph-only semantics (includes source and target node types)
        relations.push((
            components.source_workspace.clone(),
            components.source_id.clone(),
            components.target_workspace.clone(),
            components.target_id.clone(),
            full_relation,
        ));

        seen_relations.insert(rel_key);
    }

    stats.log_results(relations.len());

    Ok(relations)
}

/// Statistics for global relation scanning
#[derive(Default)]
struct ScanStats {
    scanned_keys: usize,
    skipped_revision: usize,
    skipped_tombstone: usize,
    skipped_duplicate: usize,
    skipped_invalid: usize,
}

impl ScanStats {
    fn log_results(&self, result_count: usize) {
        tracing::info!(
            "   ✓ Scan complete: {} total keys scanned",
            self.scanned_keys
        );
        tracing::debug!(
            "   Skipped: {} rev, {} tombstone, {} duplicate, {} invalid",
            self.skipped_revision,
            self.skipped_tombstone,
            self.skipped_duplicate,
            self.skipped_invalid
        );
        tracing::info!("   ✓ Returning {} unique relationships", result_count);

        if result_count == 0 && self.scanned_keys > 0 {
            tracing::warn!(
                "   ⚠️  Scanned {} keys but found 0 valid relationships!",
                self.scanned_keys
            );
        } else if result_count == 0 {
            tracing::warn!("   ⚠️  No keys found with this prefix!");
        }
    }
}
