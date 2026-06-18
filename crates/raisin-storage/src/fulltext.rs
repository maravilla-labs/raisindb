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
use std::collections::{HashMap, HashSet};

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

/// Which sub-fields of a nested element should have their value full-text indexed.
///
/// Built from a (resolved) ElementType's per-field `index` config: `include_fields`
/// lists exactly the fields whose values to index (those marked `Fulltext`). The
/// element's identity (its `element_type` shape_type) is always emitted regardless.
///
/// Semantics in the walker: a missing OR empty entry indexes NONE of the element's
/// field values (identity-only default). An unresolved element type therefore
/// contributes only its identity — matching the "identity-only by default" policy.
#[derive(Debug, Clone, Default)]
pub struct ElementFieldPlan {
    /// Field names (within the element's `content`) whose values to full-text index.
    pub include_fields: HashSet<String>,
}

/// Precomputed, storage-free plan describing how to full-text index one node.
///
/// Resolved once per node from cached per-type plans in the job handler, then passed
/// by reference into the indexing engine so the engine performs no storage lookups or
/// type resolution on the hot indexing path.
#[derive(Debug, Clone, Default)]
pub struct NodeIndexPlan {
    /// The node's type name. Always emitted as a shape identity.
    pub node_type: String,
    /// The node's archetype name, if any. Emitted as a shape identity.
    pub archetype: Option<String>,
    /// Top-level property names to index by value. `None` selects the legacy
    /// fallback (see `legacy_index_all_strings`); `Some` (possibly empty) indexes
    /// exactly the listed properties.
    pub top_level_props: Option<Vec<String>>,
    /// Per-element-type field plans for nested `Element`/`Composite` blocks,
    /// keyed by element_type name.
    pub element_plans: HashMap<String, ElementFieldPlan>,
    /// When true and `top_level_props` is `None`, index all top-level String
    /// properties (legacy behavior, used for untyped/unresolved nodes).
    pub legacy_index_all_strings: bool,
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

    /// Optional exact shape-type filter. When set, only nodes carrying this
    /// type identity (their `node_type`, `archetype`, or any nested
    /// `element_type`) match. The value must be the full `ns:TypeName`.
    pub shape_type: Option<String>,
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
    /// Index a node with all its language variants, driven by a precomputed plan.
    ///
    /// The engine creates one document per language (default language + translations),
    /// emitting the node's shape identities (node_type / archetype / nested element_types)
    /// and indexing field values exactly as the `plan` dictates. The engine performs no
    /// storage lookups or type resolution — all of that happened when the plan was built.
    fn do_index_node_with_plan(
        &self,
        job: &FullTextIndexJob,
        node: &Node,
        plan: &NodeIndexPlan,
    ) -> Result<()>;

    /// Index a node without a precomputed plan (legacy / back-compat).
    ///
    /// Delegates to [`IndexingEngine::do_index_node_with_plan`] with a fallback plan
    /// that indexes all top-level String properties and emits the node_type identity.
    fn do_index_node(&self, job: &FullTextIndexJob, node: &Node) -> Result<()> {
        let plan = NodeIndexPlan {
            node_type: node.node_type.clone(),
            legacy_index_all_strings: true,
            ..Default::default()
        };
        self.do_index_node_with_plan(job, node, &plan)
    }

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
