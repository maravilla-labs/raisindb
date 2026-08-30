// SPDX-License-Identifier: BSL-1.1

//! HNSW index backed by usearch with incremental add/remove.
//!
//! This replaces the old instant-distance implementation which required
//! a full graph rebuild on every mutation. usearch supports incremental
//! insertions and deletions, and persists the full graph to disk.

use crate::types::{DistanceMetric, QuantizationType, SearchResult, MAX_FETCH_K};
use raisin_error::Result;
use raisin_hlc::HLC;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use usearch::{Index as UsearchIndex, IndexOptions, MetricKind, ScalarKind};

/// Tracks how the usearch index was loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IndexLoadState {
    /// Created in-memory (new index, not loaded from disk).
    InMemory,
    /// Memory-mapped from disk via `view()`. Read-only until promoted.
    Viewed,
    /// Fully loaded into RAM via `load()`. Supports mutations.
    Loaded,
}

/// Over-draw used by the post-filter FALLBACK only.
///
/// The index-side filtered walk needs no over-draw at all — it returns `k`
/// in-scope neighbours or says the index has no more. This multiplier exists
/// solely so the fallback is no worse than the behaviour it replaced.
const POST_FILTER_OVERDRAW: usize = 5;

/// Operator escape hatch: `RAISIN_HNSW_DISABLE_FILTERED_SEARCH=1` forces every
/// scoped search down the post-filter fallback.
///
/// Two reasons it exists. The filtered walk is deeper than an unfiltered one
/// when a scope is very selective (it only stops once the IN-SCOPE result set
/// is full), so an operator who would rather have fast wrong answers than slow
/// right ones needs a way back without a rollback. And it is the only way to
/// reach the fallback deliberately — otherwise that code runs only when usearch
/// errors, which is to say never, until the day it matters.
///
/// Read ONCE: this is consulted per query, and `std::env::var` on every vector
/// search would be a lock and an allocation in the hot path.
fn filtered_search_disabled() -> bool {
    static DISABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DISABLED.get_or_init(|| {
        matches!(
            std::env::var("RAISIN_HNSW_DISABLE_FILTERED_SEARCH").as_deref(),
            Ok("1") | Ok("true")
        )
    })
}

/// How a workspace restriction was applied to one vector search.
///
/// Carried out of the index so the caller's "returned fewer than asked" log can
/// say WHY, instead of leaving an operator to guess between index selectivity
/// and a permission drop — which is how this defect hid for so long.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeFilterMode {
    /// No workspace restriction was requested.
    Unrestricted,
    /// Applied inside the graph walk. A short result means the index genuinely
    /// holds no more in-scope neighbours.
    IndexSide,
    /// Applied AFTER an unfiltered walk, because the filtered walk was
    /// unavailable. A short result may be pure selectivity: the walk never
    /// visited this workspace's region of the graph.
    PostFilter,
}

/// One workspace-scoped search, with enough context to explain a short result.
#[derive(Debug)]
pub struct ScopedSearch {
    /// Matches, nearest first.
    pub results: Vec<SearchResult>,

    /// How the workspace restriction was applied.
    pub mode: ScopeFilterMode,

    /// Candidates the walk produced BEFORE the workspace restriction. Equal to
    /// `results.len()` for an index-side walk; larger for a post-filter.
    pub drawn: usize,

    /// Live vectors the index holds in the requested scope. The ceiling on what
    /// any search of this scope could return, and the number that turns "search
    /// came back short" into a diagnosis.
    pub in_scope_total: usize,
}

/// Build the workspace -> vector-count map from the metadata map.
///
/// Used on load, where the counts arrive derived rather than maintained.
fn count_workspaces(key_to_meta: &HashMap<u64, NodeMeta>) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for meta in key_to_meta.values() {
        *counts.entry(meta.workspace_id.clone()).or_insert(0) += 1;
    }
    counts
}

/// Metadata for a vector entry (stored in the JSON sidecar, not in usearch).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct NodeMeta {
    pub node_id: String,
    pub workspace_id: String,
    pub revision: HLC,
}

/// HNSW index backed by usearch with metadata tracking.
///
/// The usearch `Index` owns the graph and vectors. Node metadata (node_id,
/// workspace_id, revision) is maintained in HashMaps and persisted as a
/// JSON sidecar alongside the native usearch file.
pub struct HnswIndex {
    /// usearch index (owns the HNSW graph + vectors)
    index: UsearchIndex,

    /// node_id -> usearch key mapping
    node_to_key: HashMap<String, u64>,

    /// usearch key -> node metadata
    ///
    /// Never mutated directly: every write goes through [`HnswIndex::insert_meta`]
    /// / [`HnswIndex::remove_meta`], which keep `workspace_counts` in step. Two
    /// maps that can disagree about what the index holds is exactly the drift
    /// this codebase keeps getting bitten by.
    key_to_meta: HashMap<u64, NodeMeta>,

    /// workspace_id -> number of live vectors in that workspace.
    ///
    /// Derived from `key_to_meta`, maintained incrementally so a scoped search
    /// can answer "how many candidates could this scope possibly yield?" in O(1)
    /// instead of walking the whole metadata map. It is what turns a short
    /// result into a diagnosable one, and it lets an empty scope skip the graph
    /// walk entirely.
    workspace_counts: HashMap<String, usize>,

    /// Vector dimensions
    dimensions: usize,

    /// Distance metric
    distance_metric: DistanceMetric,

    /// Next available key for usearch
    next_key: u64,

    /// Vector quantization type
    quantization: QuantizationType,

    /// How the usearch index was loaded (InMemory, Viewed, or Loaded)
    load_state: IndexLoadState,

    /// Path to the .hnsw file on disk (needed for promotion from Viewed → Loaded)
    source_path: Option<PathBuf>,
}

impl HnswIndex {
    /// Create a new empty HNSW index with the default distance metric (Cosine).
    pub fn new(dimensions: usize) -> Self {
        Self::with_metric(dimensions, DistanceMetric::default())
    }

    /// Create a new empty HNSW index with a specific distance metric.
    pub fn with_metric(dimensions: usize, metric: DistanceMetric) -> Self {
        Self::with_params(dimensions, metric, crate::types::HnswParams::default())
    }

    /// Create a new empty HNSW index with specific distance metric and tuning parameters.
    pub fn with_params(
        dimensions: usize,
        metric: DistanceMetric,
        params: crate::types::HnswParams,
    ) -> Self {
        let options = IndexOptions {
            dimensions,
            metric: metric.to_usearch_metric(),
            quantization: params.quantization.to_scalar_kind(),
            connectivity: params.connectivity,
            expansion_add: params.expansion_add,
            expansion_search: params.expansion_search,
            multi: false,
        };
        let index = UsearchIndex::new(&options).expect("Failed to create usearch index");

        Self {
            index,
            node_to_key: HashMap::new(),
            key_to_meta: HashMap::new(),
            workspace_counts: HashMap::new(),
            dimensions,
            distance_metric: metric,
            quantization: params.quantization,
            next_key: 0,
            load_state: IndexLoadState::InMemory,
            source_path: None,
        }
    }

    /// Reconstruct an index from persisted files (called by persistence module).
    pub(crate) fn from_persisted(
        path: &Path,
        dimensions: usize,
        metric: DistanceMetric,
        quantization: QuantizationType,
        node_to_key: HashMap<String, u64>,
        key_to_meta: HashMap<u64, NodeMeta>,
        next_key: u64,
    ) -> Result<Self> {
        let options = IndexOptions {
            dimensions,
            metric: metric.to_usearch_metric(),
            quantization: quantization.to_scalar_kind(),
            connectivity: 0,
            expansion_add: 0,
            expansion_search: 0,
            multi: false,
        };
        let index = UsearchIndex::new(&options).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to create usearch index: {}", e))
        })?;

        let path_str = path.to_str().ok_or_else(|| {
            raisin_error::Error::storage("Index path contains invalid UTF-8".to_string())
        })?;
        index.load(path_str).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to load usearch index: {}", e))
        })?;

        Ok(Self {
            index,
            node_to_key,
            workspace_counts: count_workspaces(&key_to_meta),
            key_to_meta,
            dimensions,
            distance_metric: metric,
            quantization,
            next_key,
            load_state: IndexLoadState::Loaded,
            source_path: Some(path.to_path_buf()),
        })
    }

    /// Reconstruct an index from persisted files using memory-mapping (view).
    ///
    /// The usearch graph is memory-mapped and read-only. Mutations will
    /// transparently promote the index to fully loaded via `ensure_mutable()`.
    pub(crate) fn from_persisted_view(
        path: &Path,
        dimensions: usize,
        metric: DistanceMetric,
        quantization: QuantizationType,
        node_to_key: HashMap<String, u64>,
        key_to_meta: HashMap<u64, NodeMeta>,
        next_key: u64,
    ) -> Result<Self> {
        let options = IndexOptions {
            dimensions,
            metric: metric.to_usearch_metric(),
            quantization: quantization.to_scalar_kind(),
            connectivity: 0,
            expansion_add: 0,
            expansion_search: 0,
            multi: false,
        };
        let index = UsearchIndex::new(&options).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to create usearch index: {}", e))
        })?;

        let path_str = path.to_str().ok_or_else(|| {
            raisin_error::Error::storage("Index path contains invalid UTF-8".to_string())
        })?;
        index.view(path_str).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to view usearch index: {}", e))
        })?;

        Ok(Self {
            index,
            node_to_key,
            workspace_counts: count_workspaces(&key_to_meta),
            key_to_meta,
            dimensions,
            distance_metric: metric,
            quantization,
            next_key,
            load_state: IndexLoadState::Viewed,
            source_path: Some(path.to_path_buf()),
        })
    }

    /// Promote a viewed (mmap'd) index to a fully loaded index.
    ///
    /// This is called automatically before mutations. If the index is already
    /// mutable (InMemory or Loaded), this is a no-op.
    fn ensure_mutable(&mut self) -> Result<()> {
        if self.load_state != IndexLoadState::Viewed {
            return Ok(());
        }

        let path = self.source_path.as_ref().ok_or_else(|| {
            raisin_error::Error::storage(
                "Viewed index has no source path for promotion".to_string(),
            )
        })?;
        let path_str = path.to_str().ok_or_else(|| {
            raisin_error::Error::storage("Index path contains invalid UTF-8".to_string())
        })?;

        // Create a fresh usearch index and load the full file into RAM.
        // We cannot reuse the viewed index because reset()+load() leaves
        // usearch's internal thread pool in a bad state.
        let options = IndexOptions {
            dimensions: self.dimensions,
            metric: self.distance_metric.to_usearch_metric(),
            quantization: self.quantization.to_scalar_kind(),
            connectivity: 0,
            expansion_add: 0,
            expansion_search: 0,
            multi: false,
        };
        let new_index = UsearchIndex::new(&options).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to create usearch index: {}", e))
        })?;
        new_index.load(path_str).map_err(|e| {
            raisin_error::Error::storage(format!(
                "Failed to promote index from view to load: {}",
                e
            ))
        })?;

        tracing::info!(path = %path.display(), "Promoted HNSW index from viewed to loaded");
        self.index = new_index;
        self.load_state = IndexLoadState::Loaded;

        Ok(())
    }

    /// Get the distance metric used by this index.
    pub fn distance_metric(&self) -> DistanceMetric {
        self.distance_metric
    }

    /// Add a vector to the index. Updates in-place if node_id already exists.
    ///
    /// If the index is memory-mapped (viewed), it will be transparently
    /// promoted to a fully loaded index before the mutation.
    pub fn add(
        &mut self,
        node_id: String,
        workspace_id: String,
        revision: HLC,
        vector: Vec<f32>,
    ) -> Result<()> {
        self.ensure_mutable()?;

        if vector.len() != self.dimensions {
            return Err(raisin_error::Error::storage(format!(
                "Vector dimension mismatch: expected {}, got {}",
                self.dimensions,
                vector.len()
            )));
        }

        // If node exists, remove old entry first
        if let Some(&old_key) = self.node_to_key.get(&node_id) {
            self.index.remove(old_key).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to remove old vector: {}", e))
            })?;
            self.remove_meta(old_key);
        }

        let key = self.next_key;
        self.next_key += 1;

        // Reserve capacity if needed (usearch needs space before add)
        let current_cap = self.index.capacity();
        if self.index.size() >= current_cap {
            let new_cap = (current_cap + 1).max(current_cap * 2).max(16);
            self.index.reserve(new_cap).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to reserve capacity: {}", e))
            })?;
        }

        self.index
            .add(key, &vector)
            .map_err(|e| raisin_error::Error::storage(format!("Failed to add vector: {}", e)))?;

        self.node_to_key.insert(node_id.clone(), key);
        self.insert_meta(
            key,
            NodeMeta {
                node_id,
                workspace_id,
                revision,
            },
        );

        Ok(())
    }

    /// Record one key's metadata, keeping `workspace_counts` in step.
    ///
    /// The ONE place `key_to_meta` gains an entry. Replacing an existing key
    /// decrements the old workspace, so a node moved between workspaces cannot
    /// leave a phantom count behind.
    fn insert_meta(&mut self, key: u64, meta: NodeMeta) {
        *self
            .workspace_counts
            .entry(meta.workspace_id.clone())
            .or_insert(0) += 1;
        if let Some(previous) = self.key_to_meta.insert(key, meta) {
            self.decrement_workspace(&previous.workspace_id);
        }
    }

    /// Forget one key's metadata, keeping `workspace_counts` in step.
    ///
    /// The ONE place `key_to_meta` loses an entry.
    fn remove_meta(&mut self, key: u64) {
        if let Some(previous) = self.key_to_meta.remove(&key) {
            self.decrement_workspace(&previous.workspace_id);
        }
    }

    /// Drop one vector from a workspace's count, removing the entry at zero so
    /// the map cannot grow without bound across a long-lived index.
    fn decrement_workspace(&mut self, workspace_id: &str) {
        if let Some(count) = self.workspace_counts.get_mut(workspace_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.workspace_counts.remove(workspace_id);
            }
        }
    }

    /// How many live vectors the index holds across `workspaces`.
    ///
    /// An EMPTY slice means "every workspace" — the same convention the search
    /// API uses — and answers with the whole index size.
    pub fn vectors_in_workspaces(&self, workspaces: &[String]) -> usize {
        if workspaces.is_empty() {
            return self.key_to_meta.len();
        }
        workspaces
            .iter()
            .filter_map(|ws| self.workspace_counts.get(ws.as_str()))
            .sum()
    }

    /// Remove a vector from the index.
    ///
    /// If the index is memory-mapped (viewed), it will be transparently
    /// promoted to a fully loaded index before the mutation.
    pub fn remove(&mut self, node_id: &str) -> Result<()> {
        self.ensure_mutable()?;

        if let Some(&key) = self.node_to_key.get(node_id) {
            self.index.remove(key).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to remove vector: {}", e))
            })?;
            self.node_to_key.remove(node_id);
            self.remove_meta(key);
        }
        Ok(())
    }

    /// Is `node_id` present in the index?
    ///
    /// A pure read — no `ensure_mutable`, so it does not promote a viewed
    /// (memory-mapped) index. The id map is part of the persisted metadata, so a
    /// viewed index answers this correctly.
    pub fn contains(&self, node_id: &str) -> bool {
        self.node_to_key.contains_key(node_id)
    }

    /// Search for k nearest neighbors, with no workspace restriction.
    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<SearchResult>> {
        Ok(self.search_scoped(query, k, &[])?.results)
    }

    /// Search for k nearest neighbors within a set of workspaces.
    ///
    /// An EMPTY `workspaces` slice means "every workspace"; it is never "match
    /// nothing" — a caller that resolved to "nothing readable" must not reach
    /// this function at all.
    ///
    /// # The restriction is applied INSIDE the graph walk
    ///
    /// usearch takes a per-candidate predicate (`filtered_search`, present in
    /// the pinned 2.24). Out-of-scope nodes are still *expanded* through — so
    /// the graph stays connected and the walk keeps navigating — but they never
    /// occupy a result slot. The walk therefore continues until it has `k`
    /// IN-SCOPE neighbours or the reachable graph is exhausted.
    ///
    /// This replaces a post-filter over a fixed over-draw, which returned SHORT
    /// (frequently empty) whenever a narrow scope sat inside a large index: the
    /// unfiltered walk simply never visited that workspace's region of the
    /// graph, and no over-draw heuristic fixes that.
    ///
    /// # Cost
    ///
    /// A very selective scope makes the walk deep: the candidate queue only
    /// stops growing once the in-scope result set is full, so a scope holding
    /// far fewer than `k` vectors traverses most of the reachable graph. That is
    /// the price of a correct answer instead of an empty one, and the zero-count
    /// early return below removes the worst case (a scope with nothing in it).
    pub fn search_scoped(
        &self,
        query: &[f32],
        k: usize,
        workspaces: &[String],
    ) -> Result<ScopedSearch> {
        if query.len() != self.dimensions {
            return Err(raisin_error::Error::storage(format!(
                "Query dimension mismatch: expected {}, got {}",
                self.dimensions,
                query.len()
            )));
        }

        let in_scope_total = self.vectors_in_workspaces(workspaces);

        if self.node_to_key.is_empty() || in_scope_total == 0 {
            return Ok(ScopedSearch {
                results: Vec::new(),
                mode: ScopeFilterMode::IndexSide,
                drawn: 0,
                in_scope_total,
            });
        }

        if workspaces.is_empty() {
            let matches = self
                .index
                .search(query, k)
                .map_err(|e| raisin_error::Error::storage(format!("Search failed: {}", e)))?;
            let results = self.hydrate(&matches);
            let drawn = results.len();
            return Ok(ScopedSearch {
                results,
                mode: ScopeFilterMode::Unrestricted,
                drawn,
                in_scope_total,
            });
        }

        if filtered_search_disabled() {
            return self.post_filtered(query, k, workspaces, in_scope_total);
        }

        match self.filtered_matches(query, k, workspaces) {
            Ok(matches) => {
                let results = self.hydrate(&matches);
                let drawn = results.len();
                Ok(ScopedSearch {
                    results,
                    mode: ScopeFilterMode::IndexSide,
                    drawn,
                    in_scope_total,
                })
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "usearch filtered_search failed; falling back to an UNFILTERED walk with a \
                     post-filter, which can return short for a narrow scope"
                );
                self.post_filtered(query, k, workspaces, in_scope_total)
            }
        }
    }

    /// The fallback, and the only one: an unfiltered walk plus a post-filter.
    ///
    /// This is the algorithm the index-side walk replaced, kept for the two
    /// cases that cannot use the filtered walk — a `filtered_search` failure,
    /// and an operator who has turned it off. It is the SAME implementation for
    /// both, so the escape hatch exercises exactly the code the error path
    /// would take.
    ///
    /// It is retained rather than deleted because losing selectivity is better
    /// than losing search, but its shortfall is a different phenomenon from an
    /// index-side one and the caller reports it as such — see `ScopeFilterMode`.
    pub(crate) fn post_filtered(
        &self,
        query: &[f32],
        k: usize,
        workspaces: &[String],
        in_scope_total: usize,
    ) -> Result<ScopedSearch> {
        let overdrawn = k.saturating_mul(POST_FILTER_OVERDRAW).min(MAX_FETCH_K);
        let matches = self
            .index
            .search(query, overdrawn)
            .map_err(|e| raisin_error::Error::storage(format!("Search failed: {}", e)))?;
        let mut results = self.hydrate(&matches);
        let drawn = results.len();
        results.retain(|r| workspaces.iter().any(|ws| ws == &r.workspace_id));
        results.truncate(k);
        Ok(ScopedSearch {
            results,
            mode: ScopeFilterMode::PostFilter,
            drawn,
            in_scope_total,
        })
    }

    /// Run the workspace-filtered usearch walk.
    ///
    /// # Safety / soundness
    ///
    /// `filtered_search` passes the closure to C++ as a raw function pointer
    /// plus an opaque state pointer (usearch 2.24 `rust/lib.rs:718-740`), and
    /// C++ calls it back through a `noexcept` lambda
    /// (`index_dense.hpp:2079`). Three things make that sound here:
    ///
    /// 1. **Lifetime.** usearch takes the address of its own by-value `filter`
    ///    parameter, so the closure outlives every call: the C++ side has
    ///    returned before `filtered_search` does. The closure borrows
    ///    `&self.key_to_meta` and `workspaces`, both of which outlive this
    ///    function — `&self` is held by the caller's read guard for the whole
    ///    search.
    /// 2. **Threading.** A single-query usearch search runs on the calling
    ///    thread; even if it did not, the closure only performs shared reads of
    ///    a `HashMap` and a slice, which are `Sync`. It takes NO lock — a lock
    ///    here could deadlock against the one the caller already holds, and a
    ///    poisoned-lock `unwrap` would be a panic (see 3).
    /// 3. **Unwinding.** A panic crossing the `noexcept` C++ frame is undefined
    ///    behaviour, and the usearch trampoline has no `catch_unwind` of its
    ///    own. The body is panic-free by construction (two lookups, no
    ///    indexing, no allocation, no `unwrap`), and is wrapped in
    ///    `catch_unwind` anyway so that any future edit — or a panicking
    ///    hasher — fails CLOSED (candidate dropped) instead of unwinding into
    ///    C++.
    fn filtered_matches(
        &self,
        query: &[f32],
        k: usize,
        workspaces: &[String],
    ) -> std::result::Result<usearch::ffi::Matches, String> {
        let key_to_meta = &self.key_to_meta;
        let predicate = move |key: u64| -> bool {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                match key_to_meta.get(&key) {
                    Some(meta) => workspaces.iter().any(|ws| ws == &meta.workspace_id),
                    None => false,
                }
            }))
            .unwrap_or(false)
        };

        self.index
            .filtered_search(query, k, predicate)
            .map_err(|e| e.to_string())
    }

    /// Turn raw usearch matches into results, dropping any key the metadata map
    /// no longer knows about.
    fn hydrate(&self, matches: &usearch::ffi::Matches) -> Vec<SearchResult> {
        let mut results = Vec::with_capacity(matches.keys.len());
        for i in 0..matches.keys.len() {
            let key = matches.keys[i];
            let distance = matches.distances[i];
            if let Some(meta) = self.key_to_meta.get(&key) {
                results.push(SearchResult::new(
                    meta.node_id.clone(),
                    meta.workspace_id.clone(),
                    meta.revision,
                    distance,
                ));
            }
        }
        results
    }

    /// Get the number of vectors in the index.
    pub fn len(&self) -> usize {
        self.node_to_key.len()
    }

    /// Check if the index is empty.
    pub fn is_empty(&self) -> bool {
        self.node_to_key.is_empty()
    }

    /// Estimate memory usage in bytes.
    pub fn estimated_memory_bytes(&self) -> usize {
        // usearch reports its own memory usage
        let usearch_bytes = self.index.memory_usage();

        // HashMap overhead: ~64 bytes per entry for node_to_key, ~80 for key_to_meta
        let map_overhead = self.node_to_key.len() * 64 + self.key_to_meta.len() * 80;

        usearch_bytes + map_overhead
    }

    /// Save index to file (dual-file format: .hnsw + .hnsw.meta).
    ///
    /// No-op for viewed indexes since the on-disk file is already current.
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        if self.load_state == IndexLoadState::Viewed {
            tracing::debug!("Skipping save for viewed (read-only) index");
            return Ok(());
        }
        crate::persistence::save_to_file(self, path.as_ref())
    }

    /// Load index from file, auto-detecting old vs new format.
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        crate::persistence::load_from_file(path.as_ref())
    }

    /// View (mmap) an index from file. The usearch graph is memory-mapped
    /// and read-only. Mutations will transparently promote to fully loaded.
    pub fn view_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        crate::persistence::view_from_file(path.as_ref())
    }

    // --- Accessors for persistence module ---

    pub(crate) fn usearch_index(&self) -> &UsearchIndex {
        &self.index
    }

    pub(crate) fn node_to_key(&self) -> &HashMap<String, u64> {
        &self.node_to_key
    }

    pub(crate) fn key_to_meta(&self) -> &HashMap<u64, NodeMeta> {
        &self.key_to_meta
    }

    pub fn dimensions(&self) -> usize {
        self.dimensions
    }

    pub fn quantization(&self) -> QuantizationType {
        self.quantization
    }

    pub(crate) fn next_key(&self) -> u64 {
        self.next_key
    }

    pub(crate) fn is_viewed(&self) -> bool {
        self.load_state == IndexLoadState::Viewed
    }
}

impl DistanceMetric {
    /// Convert to usearch MetricKind.
    pub(crate) fn to_usearch_metric(self) -> MetricKind {
        match self {
            DistanceMetric::Cosine => MetricKind::Cos,
            DistanceMetric::L2 => MetricKind::L2sq,
            DistanceMetric::InnerProduct => MetricKind::IP,
            DistanceMetric::Hamming => MetricKind::Hamming,
        }
    }
}

impl QuantizationType {
    /// Convert to usearch ScalarKind.
    pub(crate) fn to_scalar_kind(self) -> ScalarKind {
        match self {
            QuantizationType::F32 => ScalarKind::F32,
            QuantizationType::F16 => ScalarKind::F16,
            QuantizationType::Int8 => ScalarKind::I8,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_vector(dims: usize, seed: f32) -> Vec<f32> {
        (0..dims).map(|i| (i as f32 + seed) / dims as f32).collect()
    }

    #[test]
    fn test_add_and_search() {
        let mut index = HnswIndex::new(128);

        index
            .add(
                "node1".to_string(),
                "workspace1".to_string(),
                HLC::new(1, 0),
                create_test_vector(128, 1.0),
            )
            .unwrap();
        index
            .add(
                "node2".to_string(),
                "workspace1".to_string(),
                HLC::new(2, 0),
                create_test_vector(128, 2.0),
            )
            .unwrap();
        index
            .add(
                "node3".to_string(),
                "workspace1".to_string(),
                HLC::new(3, 0),
                create_test_vector(128, 3.0),
            )
            .unwrap();

        assert_eq!(index.len(), 3);

        let query = create_test_vector(128, 1.1);
        let results = index.search(&query, 2).unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].node_id, "node1");
        assert_eq!(results[0].workspace_id, "workspace1");
    }

    #[test]
    fn test_remove() {
        let mut index = HnswIndex::new(128);

        index
            .add(
                "node1".to_string(),
                "workspace1".to_string(),
                HLC::new(1, 0),
                create_test_vector(128, 1.0),
            )
            .unwrap();
        index
            .add(
                "node2".to_string(),
                "workspace1".to_string(),
                HLC::new(2, 0),
                create_test_vector(128, 2.0),
            )
            .unwrap();

        assert_eq!(index.len(), 2);

        index.remove("node1").unwrap();
        assert_eq!(index.len(), 1);

        let query = create_test_vector(128, 1.0);
        let results = index.search(&query, 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].node_id, "node2");
    }

    #[test]
    fn test_persistence() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index_path = temp_dir.path().join("test.hnsw");

        {
            let mut index = HnswIndex::new(128);
            index
                .add(
                    "node1".to_string(),
                    "workspace1".to_string(),
                    HLC::new(1, 0),
                    create_test_vector(128, 1.0),
                )
                .unwrap();
            index
                .add(
                    "node2".to_string(),
                    "workspace1".to_string(),
                    HLC::new(2, 0),
                    create_test_vector(128, 2.0),
                )
                .unwrap();

            index.save_to_file(&index_path).unwrap();
        }

        {
            let index = HnswIndex::load_from_file(&index_path).unwrap();
            assert_eq!(index.len(), 2);
            assert_eq!(index.dimensions, 128);

            let query = create_test_vector(128, 1.1);
            let results = index.search(&query, 2).unwrap();
            assert_eq!(results[0].node_id, "node1");
        }
    }

    #[test]
    fn test_dimension_validation() {
        let mut index = HnswIndex::new(128);

        let result = index.add(
            "node1".to_string(),
            "workspace1".to_string(),
            HLC::new(1, 0),
            vec![1.0, 2.0, 3.0],
        );
        assert!(result.is_err());

        let result = index.add(
            "node1".to_string(),
            "workspace1".to_string(),
            HLC::new(1, 0),
            create_test_vector(128, 1.0),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_default_metric_is_cosine() {
        let index = HnswIndex::new(128);
        assert_eq!(index.distance_metric(), DistanceMetric::Cosine);
    }

    #[test]
    fn test_with_metric_constructor() {
        let index = HnswIndex::with_metric(128, DistanceMetric::L2);
        assert_eq!(index.distance_metric(), DistanceMetric::L2);

        let index = HnswIndex::with_metric(128, DistanceMetric::InnerProduct);
        assert_eq!(index.distance_metric(), DistanceMetric::InnerProduct);
    }

    fn create_normalized_vector(dims: usize, seed: f32) -> Vec<f32> {
        let raw: Vec<f32> = (0..dims).map(|i| (i as f32 + seed) / dims as f32).collect();
        let magnitude = raw.iter().map(|x| x * x).sum::<f32>().sqrt();
        raw.iter().map(|x| x / magnitude).collect()
    }

    #[test]
    fn test_l2_distance_metric() {
        let mut index = HnswIndex::with_metric(4, DistanceMetric::L2);

        index
            .add(
                "origin".to_string(),
                "ws".to_string(),
                HLC::new(1, 0),
                vec![0.0, 0.0, 0.0, 0.0],
            )
            .unwrap();
        index
            .add(
                "far".to_string(),
                "ws".to_string(),
                HLC::new(2, 0),
                vec![10.0, 10.0, 10.0, 10.0],
            )
            .unwrap();

        let results = index.search(&[0.1, 0.1, 0.1, 0.1], 2).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].node_id, "origin");
        assert_eq!(results[1].node_id, "far");

        // usearch L2sq returns squared distance, so distance to origin =
        // 4 * 0.01 = 0.04 (not sqrt'd)
        assert!(results[0].distance < 1.0);
        assert!(results[1].distance > 10.0);
    }

    #[test]
    fn test_cosine_with_normalized_vectors() {
        let mut index = HnswIndex::with_metric(4, DistanceMetric::Cosine);

        let v1 = create_normalized_vector(4, 1.0);
        let v2 = create_normalized_vector(4, 100.0);

        index
            .add(
                "a".to_string(),
                "ws".to_string(),
                HLC::new(1, 0),
                v1.clone(),
            )
            .unwrap();
        index
            .add("b".to_string(), "ws".to_string(), HLC::new(2, 0), v2)
            .unwrap();

        let results = index.search(&v1, 2).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].node_id, "a");
        assert!(results[0].distance.abs() < 0.01);
    }

    #[test]
    fn test_metric_persists_through_save_load() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index_path = temp_dir.path().join("test_metric.hnsw");

        {
            let mut index = HnswIndex::with_metric(4, DistanceMetric::L2);
            index
                .add(
                    "node1".to_string(),
                    "ws".to_string(),
                    HLC::new(1, 0),
                    vec![1.0, 2.0, 3.0, 4.0],
                )
                .unwrap();
            index.save_to_file(&index_path).unwrap();
        }

        {
            let index = HnswIndex::load_from_file(&index_path).unwrap();
            assert_eq!(index.distance_metric(), DistanceMetric::L2);
            assert_eq!(index.len(), 1);
        }
    }

    #[test]
    fn test_distance_metric_requires_normalization() {
        assert!(DistanceMetric::Cosine.requires_normalization());
        assert!(DistanceMetric::InnerProduct.requires_normalization());
        assert!(!DistanceMetric::L2.requires_normalization());
        assert!(!DistanceMetric::Hamming.requires_normalization());
    }

    #[test]
    fn test_update_existing_node() {
        let mut index = HnswIndex::new(4);

        index
            .add(
                "node1".to_string(),
                "ws".to_string(),
                HLC::new(1, 0),
                vec![1.0, 0.0, 0.0, 0.0],
            )
            .unwrap();
        assert_eq!(index.len(), 1);

        // Update with new vector
        index
            .add(
                "node1".to_string(),
                "ws".to_string(),
                HLC::new(2, 0),
                vec![0.0, 1.0, 0.0, 0.0],
            )
            .unwrap();
        assert_eq!(index.len(), 1); // Still 1, not 2

        let results = index.search(&[0.0, 1.0, 0.0, 0.0], 1).unwrap();
        assert_eq!(results[0].node_id, "node1");
    }

    #[test]
    fn test_empty_index_search() {
        let index = HnswIndex::new(4);
        let results = index.search(&[1.0, 0.0, 0.0, 0.0], 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_remove_nonexistent() {
        let mut index = HnswIndex::new(4);
        // Should not error
        index.remove("nonexistent").unwrap();
    }

    #[test]
    fn test_view_and_search() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index_path = temp_dir.path().join("view_test.hnsw");

        // Create and save an index
        {
            let mut index = HnswIndex::new(128);
            index
                .add(
                    "node1".to_string(),
                    "ws".to_string(),
                    HLC::new(1, 0),
                    create_test_vector(128, 1.0),
                )
                .unwrap();
            index
                .add(
                    "node2".to_string(),
                    "ws".to_string(),
                    HLC::new(2, 0),
                    create_test_vector(128, 2.0),
                )
                .unwrap();
            index.save_to_file(&index_path).unwrap();
        }

        // View (mmap) and search
        {
            let index = HnswIndex::view_from_file(&index_path).unwrap();
            assert!(index.is_viewed());
            assert_eq!(index.len(), 2);

            let query = create_test_vector(128, 1.1);
            let results = index.search(&query, 2).unwrap();
            assert_eq!(results.len(), 2);
            assert_eq!(results[0].node_id, "node1");
        }
    }

    #[test]
    fn test_view_then_add_promotes() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index_path = temp_dir.path().join("promote_test.hnsw");

        {
            let mut index = HnswIndex::new(128);
            index
                .add(
                    "node1".to_string(),
                    "ws".to_string(),
                    HLC::new(1, 0),
                    create_test_vector(128, 1.0),
                )
                .unwrap();
            index.save_to_file(&index_path).unwrap();
        }

        {
            let mut index = HnswIndex::view_from_file(&index_path).unwrap();
            assert!(index.is_viewed());

            // Adding should transparently promote to loaded
            index
                .add(
                    "node2".to_string(),
                    "ws".to_string(),
                    HLC::new(2, 0),
                    create_test_vector(128, 2.0),
                )
                .unwrap();
            assert!(!index.is_viewed());
            assert_eq!(index.len(), 2);

            // Both vectors should be searchable
            let results = index.search(&create_test_vector(128, 1.1), 2).unwrap();
            assert_eq!(results.len(), 2);
            assert_eq!(results[0].node_id, "node1");
        }
    }

    #[test]
    fn test_view_then_remove_promotes() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index_path = temp_dir.path().join("remove_promote_test.hnsw");

        {
            let mut index = HnswIndex::new(128);
            index
                .add(
                    "node1".to_string(),
                    "ws".to_string(),
                    HLC::new(1, 0),
                    create_test_vector(128, 1.0),
                )
                .unwrap();
            index
                .add(
                    "node2".to_string(),
                    "ws".to_string(),
                    HLC::new(2, 0),
                    create_test_vector(128, 2.0),
                )
                .unwrap();
            index.save_to_file(&index_path).unwrap();
        }

        {
            let mut index = HnswIndex::view_from_file(&index_path).unwrap();
            assert!(index.is_viewed());

            index.remove("node1").unwrap();
            assert!(!index.is_viewed());
            assert_eq!(index.len(), 1);

            let results = index.search(&create_test_vector(128, 1.0), 10).unwrap();
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].node_id, "node2");
        }
    }

    #[test]
    fn test_view_save_is_noop() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index_path = temp_dir.path().join("view_save_test.hnsw");

        {
            let mut index = HnswIndex::new(128);
            index
                .add(
                    "node1".to_string(),
                    "ws".to_string(),
                    HLC::new(1, 0),
                    create_test_vector(128, 1.0),
                )
                .unwrap();
            index.save_to_file(&index_path).unwrap();
        }

        {
            let index = HnswIndex::view_from_file(&index_path).unwrap();
            // Saving a viewed index should be a no-op (no panic, no error)
            index.save_to_file(&index_path).unwrap();
            assert!(index.is_viewed());
        }
    }

    #[test]
    fn test_view_estimated_memory_less_than_loaded() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index_path = temp_dir.path().join("mem_test.hnsw");

        {
            let mut index = HnswIndex::new(128);
            for i in 0..100 {
                index
                    .add(
                        format!("node{}", i),
                        "ws".to_string(),
                        HLC::new(i as u64, 0),
                        create_test_vector(128, i as f32),
                    )
                    .unwrap();
            }
            index.save_to_file(&index_path).unwrap();
        }

        let loaded_mem = {
            let index = HnswIndex::load_from_file(&index_path).unwrap();
            index.estimated_memory_bytes()
        };

        let viewed_mem = {
            let index = HnswIndex::view_from_file(&index_path).unwrap();
            index.estimated_memory_bytes()
        };

        // Viewed index should report less memory than fully loaded
        assert!(
            viewed_mem < loaded_mem,
            "viewed ({}) should use less memory than loaded ({})",
            viewed_mem,
            loaded_mem
        );
    }

    /// The workspace counts drive the scoped search's zero-check and its
    /// diagnostics, so a count that drifts from `key_to_meta` is either a
    /// silently empty search or a lie in the log. Every mutation path is
    /// exercised here: insert, in-place update that MOVES a node between
    /// workspaces, and remove.
    #[test]
    fn test_workspace_counts_never_drift_from_the_metadata_map() {
        let mut index = HnswIndex::new(4);

        index
            .add(
                "n1".to_string(),
                "wsA".to_string(),
                HLC::new(1, 0),
                vec![1.0, 0.0, 0.0, 0.0],
            )
            .unwrap();
        index
            .add(
                "n2".to_string(),
                "wsA".to_string(),
                HLC::new(2, 0),
                vec![0.0, 1.0, 0.0, 0.0],
            )
            .unwrap();

        assert_eq!(index.vectors_in_workspaces(&["wsA".to_string()]), 2);
        assert_eq!(index.vectors_in_workspaces(&[]), 2, "empty = whole index");

        // Re-adding an existing node under a different workspace must MOVE the
        // count, not double it.
        index
            .add(
                "n1".to_string(),
                "wsB".to_string(),
                HLC::new(3, 0),
                vec![1.0, 0.0, 0.0, 0.0],
            )
            .unwrap();

        assert_eq!(index.vectors_in_workspaces(&["wsA".to_string()]), 1);
        assert_eq!(index.vectors_in_workspaces(&["wsB".to_string()]), 1);
        assert_eq!(index.vectors_in_workspaces(&[]), 2);

        index.remove("n1").unwrap();
        assert_eq!(index.vectors_in_workspaces(&["wsB".to_string()]), 0);
        assert_eq!(index.vectors_in_workspaces(&[]), 1);

        // A workspace that never existed is zero, not a panic.
        assert_eq!(index.vectors_in_workspaces(&["nope".to_string()]), 0);
    }

    /// An empty scope must be answered as EMPTY, never as "no filter".
    #[test]
    fn test_scoped_search_of_an_absent_workspace_is_empty() {
        let mut index = HnswIndex::new(4);
        index
            .add(
                "n1".to_string(),
                "wsA".to_string(),
                HLC::new(1, 0),
                vec![1.0, 0.0, 0.0, 0.0],
            )
            .unwrap();

        let scoped = index
            .search_scoped(&[1.0, 0.0, 0.0, 0.0], 5, &["wsB".to_string()])
            .unwrap();

        assert!(scoped.results.is_empty());
        assert_eq!(scoped.in_scope_total, 0);
        assert_eq!(scoped.mode, ScopeFilterMode::IndexSide);
    }
}
