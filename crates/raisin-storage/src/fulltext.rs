// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Full-text search indexing traits and types.
//!
//! This module defines the abstraction layer for full-text search indexing in RaisinDB.
//! The architecture separates job persistence (FullTextJobStore) from indexing logic
//! (IndexingEngine), allowing different storage backends to implement the job queue
//! while keeping the Tantivy-based indexing engine in raisin-indexer.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use serde::{Deserialize, Serialize};

/// Job types for full-text indexing operations
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum JobKind {
    /// Add or update a node in the index
    AddNode,
    /// Remove a node from the index
    DeleteNode,
    /// Handle branch creation by copying index
    BranchCreated,
}

/// A full-text indexing job
///
/// Jobs are lightweight and contain only metadata. The actual Node data
/// is fetched from storage when the job is processed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FullTextIndexJob {
    /// Unique identifier for this job
    pub job_id: String,

    /// Type of indexing operation
    pub kind: JobKind,

    /// Tenant identifier
    pub tenant_id: String,

    /// Repository identifier
    pub repo_id: String,

    /// Workspace identifier
    pub workspace_id: String,

    /// Branch name
    pub branch: String,

    /// Node revision (for exact version fetching)
    pub revision: HLC,

    /// Node ID (for AddNode and DeleteNode operations)
    pub node_id: Option<String>,

    /// Source branch name (for BranchCreated operation)
    pub source_branch: Option<String>,

    /// Default language for the repository (immutable after creation)
    pub default_language: String,

    /// Supported languages for translations
    pub supported_languages: Vec<String>,

    /// Property names to index for full-text search (schema-driven)
    /// If None, falls back to indexing all String properties
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub properties_to_index: Option<Vec<String>>,
}

/// Query parameters for full-text search
#[derive(Debug, Clone)]
pub struct FullTextSearchQuery {
    /// Tenant identifier
    pub tenant_id: String,

    /// Repository identifier
    pub repo_id: String,

    /// Workspace identifiers
    /// None = search all workspaces (cross-workspace)
    /// Some(vec![ws]) = single workspace
    /// Some(vec![ws1, ws2, ...]) = multiple specific workspaces
    pub workspace_ids: Option<Vec<String>>,

    /// Branch name
    pub branch: String,

    /// Language code (e.g., "en", "de", "fr")
    pub language: String,

    /// Search query string (Tantivy query syntax)
    pub query: String,

    /// Maximum number of results to return
    pub limit: usize,

    /// Optional revision for point-in-time search
    /// None = HEAD/latest, Some(revision) = search at specific revision
    pub revision: Option<HLC>,
}

/// Search result with relevance score
#[derive(Debug, Clone)]
pub struct FullTextSearchResult {
    /// Node ID
    pub node_id: String,

    /// Workspace ID (required for cross-workspace searches)
    pub workspace_id: String,

    /// Relevance score (higher is better)
    pub score: f32,

    /// Node name (optional, for hybrid search)
    pub name: Option<String>,

    /// Node type (optional, for hybrid search)
    pub node_type: Option<String>,

    /// Node path (from Tantivy stored field)
    pub path: Option<String>,

    /// Revision (optional, for hybrid search)
    pub revision: Option<HLC>,
}

/// Persistent job queue for full-text indexing.
///
/// This trait is implemented by storage backends (e.g., RocksDB) to provide
/// a crash-safe, persistent queue for indexing jobs.
pub trait FullTextJobStore: Send + Sync {
    /// Enqueue a new indexing job
    ///
    /// The job will be added to the persistent queue and processed asynchronously
    /// by the IndexerWorker.
    fn enqueue(&self, job: &FullTextIndexJob) -> Result<()>;

    /// Dequeue up to `count` pending jobs
    ///
    /// Returns jobs that are ready to be processed. Jobs are marked as
    /// "processing" to prevent duplicate processing.
    fn dequeue(&self, count: usize) -> Result<Vec<FullTextIndexJob>>;

    /// Mark jobs as successfully completed
    ///
    /// Removes the jobs from the queue.
    fn complete(&self, job_ids: &[String]) -> Result<()>;

    /// Mark a job as failed
    ///
    /// Records the error and may schedule retry based on implementation policy.
    fn fail(&self, job_id: &str, error: &str) -> Result<()>;
}

/// Full-text indexing engine interface.
///
/// This trait is implemented by the Tantivy-based indexing engine in raisin-indexer.
/// It handles the actual indexing, searching, and index management operations.
pub trait IndexingEngine: Send + Sync {
    /// Index a node with all its language variants
    ///
    /// The engine will create one document per language (default language + translations).
    /// It uses the NodeTypeSchema to determine which properties should be indexed.
    fn do_index_node(&self, job: &FullTextIndexJob, node: &Node) -> Result<()>;

    /// Remove a node from the index
    ///
    /// Deletes all language variants of the node.
    fn do_delete_node(&self, job: &FullTextIndexJob) -> Result<()>;

    /// Handle branch creation
    ///
    /// Copies the index from the source branch to the new branch.
    fn do_branch_created(&self, job: &FullTextIndexJob) -> Result<()>;

    /// Search the index
    ///
    /// Returns a list of node IDs ranked by relevance.
    fn search(&self, query: &FullTextSearchQuery) -> Result<Vec<FullTextSearchResult>>;
}
