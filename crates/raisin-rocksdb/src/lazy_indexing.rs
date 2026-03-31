//! Lazy Indexing System for RaisinDB
//!
//! This module provides lazy/on-demand indexing for secondary indexes during cluster replication.
//!
//! ## Problem
//! During cluster catch-up scenarios, replaying thousands of operations with full indexing
//! can be slow. Many secondary indexes (PROPERTY_INDEX, PATH_INDEX) may not be needed
//! immediately.
//!
//! ## Solution
//! - Skip expensive secondary indexes during bulk replication
//! - Track which revisions have been indexed
//! - Build indexes on-demand when queries need them
//! - Provide background rebuilding for consistency
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────┐
//! │  Replication Replay │
//! │  (Bulk Mode)        │
//! └──────────┬──────────┘
//!            │
//!            ├─> Write to NODES (fast)
//!            ├─> Write to ORDERED_CHILDREN (fast)
//!            └─> Skip PROPERTY_INDEX (lazy)
//!
//! ┌─────────────────────┐
//! │  Query Execution    │
//! └──────────┬──────────┘
//!            │
//!            ├─> Check IndexStatus
//!            ├─> Build missing indexes if needed
//!            └─> Execute query
//! ```
use crate::repositories::hash_property_value;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::DB;
use std::sync::Arc;

/// Column family for tracking index build status
pub const INDEX_STATUS_CF: &str = "index_status";

/// Tracks the status of lazy indexes
#[derive(Clone)]
pub struct LazyIndexManager {
    db: Arc<DB>,
}

impl LazyIndexManager {
    pub fn new(db: Arc<DB>) -> Self {
        Self { db }
    }

    /// Check if property index is up-to-date for a given tenant/repo/branch
    ///
    /// Returns the last indexed revision, or None if never indexed
    pub fn get_property_index_status(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
    ) -> Result<Option<HLC>> {
        let key = format!(
            "prop_index\0{}\0{}\0{}\0{}",
            tenant_id, repo_id, branch, workspace
        );

        let cf_index_status = cf_handle(&self.db, cf::INDEX_STATUS)?;

        match self.db.get_cf(cf_index_status, key.as_bytes()) {
            Ok(Some(value)) => {
                // Parse HLC from stored string
                let hlc_str = String::from_utf8(value).map_err(|e| {
                    raisin_error::Error::storage(format!("Invalid UTF-8 in index status: {}", e))
                })?;

                let hlc = hlc_str.parse::<HLC>().map_err(|e| {
                    raisin_error::Error::storage(format!("Failed to parse HLC: {}", e))
                })?;

                Ok(Some(hlc))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(raisin_error::Error::storage(format!(
                "Failed to read index status: {}",
                e
            ))),
        }
    }

    /// Mark property index as built up to a specific revision
    pub fn set_property_index_status(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        revision: &HLC,
    ) -> Result<()> {
        let key = format!(
            "prop_index\0{}\0{}\0{}\0{}",
            tenant_id, repo_id, branch, workspace
        );

        let value = revision.to_string();

        let cf_index_status = cf_handle(&self.db, cf::INDEX_STATUS)?;

        self.db
            .put_cf(cf_index_status, key.as_bytes(), value.as_bytes())
            .map_err(|e| {
                raisin_error::Error::storage(format!("Failed to write index status: {}", e))
            })?;

        tracing::debug!(
            "Property index marked as built up to revision {} for {}/{}/{}/{}",
            revision,
            tenant_id,
            repo_id,
            branch,
            workspace
        );

        Ok(())
    }

    /// Build property index for all nodes in a tenant/repo/branch/workspace
    ///
    /// This scans the NODES column family and builds PROPERTY_INDEX entries.
    /// Should be called:
    /// - After bulk replication catch-up
    /// - When a query needs property index but it's out of date
    /// - As a background maintenance task
    pub async fn build_property_index(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
    ) -> Result<BuildResult> {
        tracing::info!(
            "🔨 Building property index for {}/{}/{}/{}",
            tenant_id,
            repo_id,
            branch,
            workspace
        );

        let start = std::time::Instant::now();
        let mut nodes_processed = 0;
        let mut properties_indexed = 0;

        // Scan all nodes in this scope
        let prefix = keys::KeyBuilder::new()
            .push(tenant_id)
            .push(repo_id)
            .push(branch)
            .push(workspace)
            .push("nodes")
            .build_prefix();

        let cf_nodes = cf_handle(&self.db, cf::NODES)?;
        let cf_prop_index = cf_handle(&self.db, cf::PROPERTY_INDEX)?;

        let iter = self.db.iterator_cf(
            cf_nodes,
            rocksdb::IteratorMode::From(&prefix, rocksdb::Direction::Forward),
        );

        for result in iter {
            let (key, value) = result.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Check if still in our scope
            if !key.starts_with(&prefix) {
                break;
            }

            // Deserialize node
            let node: Node = match rmp_serde::from_slice(&value) {
                Ok(n) => n,
                Err(e) => {
                    tracing::warn!("Failed to deserialize node: {}", e);
                    continue;
                }
            };

            // Extract revision from key (last 16 bytes)
            // Key format: {prefix}\0{node_id}\0{~revision_16_bytes}
            if key.len() < 16 {
                continue;
            }

            let hlc_bytes = &key[key.len() - 16..];
            let revision = match HLC::decode_descending(hlc_bytes) {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("Failed to decode HLC from key: {}", e);
                    continue;
                }
            };

            // Index each property
            for (prop_name, prop_value) in &node.properties {
                let value_hash = hash_property_value(prop_value);

                let prop_key = keys::property_index_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    prop_name,
                    &value_hash,
                    &revision,
                    &node.id,
                    false, // is_published
                );

                // Store empty value (key is the index)
                self.db
                    .put_cf(cf_prop_index, prop_key, b"")
                    .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

                properties_indexed += 1;
            }

            nodes_processed += 1;

            if nodes_processed % 1000 == 0 {
                tracing::debug!("   Indexed {} nodes...", nodes_processed);
            }
        }

        let elapsed = start.elapsed();

        tracing::info!(
            "✅ Property index built: {} nodes, {} properties in {:?}",
            nodes_processed,
            properties_indexed,
            elapsed
        );

        Ok(BuildResult {
            nodes_processed,
            properties_indexed,
            elapsed,
        })
    }

    /// Check if property index needs building and build it if necessary
    ///
    /// This is called before query execution to ensure indexes are available.
    /// Builds the index synchronously if needed.
    pub async fn ensure_property_index(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
    ) -> Result<()> {
        let status = self.get_property_index_status(tenant_id, repo_id, branch, workspace)?;

        // If never indexed, build it now (synchronous)
        if status.is_none() {
            tracing::info!("Property index not found, building on-demand...");
            self.build_property_index(tenant_id, repo_id, branch, workspace)
                .await?;
        }

        Ok(())
    }

    /// Queue a background job to build property index
    ///
    /// This uses the existing job system (JobRegistry + JobDataStore) to
    /// schedule index building as a background task. Should be called after
    /// bulk replication catch-up.
    ///
    /// IMPORTANT: Uses JobRegistry.register_job() + JobDataStore.put()
    /// as per project conventions in CLAUDE.md
    pub async fn queue_property_index_build(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
    ) -> Result<String> {
        // TODO: Implement when wired into RocksDBStorage
        // This will use:
        //   JobRegistry.register_job("build_property_index", ...)
        //   JobDataStore.put(job_id, job_context)

        let job_id = format!(
            "build_prop_index_{}_{}_{}_{}",
            tenant_id, repo_id, branch, workspace
        );

        tracing::info!("📋 Queued property index build job: {}", job_id);

        Ok(job_id)
    }
}

/// Result of building an index
#[derive(Debug, Clone)]
pub struct BuildResult {
    pub nodes_processed: usize,
    pub properties_indexed: usize,
    pub elapsed: std::time::Duration,
}
