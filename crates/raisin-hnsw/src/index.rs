// SPDX-License-Identifier: BSL-1.1

//! HNSW index backed by usearch with incremental add/remove.
//!
//! This replaces the old instant-distance implementation which required
//! a full graph rebuild on every mutation. usearch supports incremental
//! insertions and deletions, and persists the full graph to disk.

use crate::types::{DistanceMetric, SearchResult};
use raisin_error::Result;
use raisin_hlc::HLC;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use usearch::{Index as UsearchIndex, IndexOptions, MetricKind, ScalarKind};

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
    key_to_meta: HashMap<u64, NodeMeta>,

    /// Vector dimensions
    dimensions: usize,

    /// Distance metric
    distance_metric: DistanceMetric,

    /// Next available key for usearch
    next_key: u64,
}

impl HnswIndex {
    /// Create a new empty HNSW index with the default distance metric (Cosine).
    pub fn new(dimensions: usize) -> Self {
        Self::with_metric(dimensions, DistanceMetric::default())
    }

    /// Create a new empty HNSW index with a specific distance metric.
    pub fn with_metric(dimensions: usize, metric: DistanceMetric) -> Self {
        let options = IndexOptions {
            dimensions,
            metric: metric.to_usearch_metric(),
            quantization: ScalarKind::F32,
            connectivity: 0,
            expansion_add: 0,
            expansion_search: 0,
            multi: false,
        };
        let index = UsearchIndex::new(&options).expect("Failed to create usearch index");

        Self {
            index,
            node_to_key: HashMap::new(),
            key_to_meta: HashMap::new(),
            dimensions,
            distance_metric: metric,
            next_key: 0,
        }
    }

    /// Reconstruct an index from persisted files (called by persistence module).
    pub(crate) fn from_persisted(
        path: &Path,
        dimensions: usize,
        metric: DistanceMetric,
        node_to_key: HashMap<String, u64>,
        key_to_meta: HashMap<u64, NodeMeta>,
        next_key: u64,
    ) -> Result<Self> {
        let options = IndexOptions {
            dimensions,
            metric: metric.to_usearch_metric(),
            quantization: ScalarKind::F32,
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
            key_to_meta,
            dimensions,
            distance_metric: metric,
            next_key,
        })
    }

    /// Get the distance metric used by this index.
    pub fn distance_metric(&self) -> DistanceMetric {
        self.distance_metric
    }

    /// Add a vector to the index. Updates in-place if node_id already exists.
    pub fn add(
        &mut self,
        node_id: String,
        workspace_id: String,
        revision: HLC,
        vector: Vec<f32>,
    ) -> Result<()> {
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
            self.key_to_meta.remove(&old_key);
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
        self.key_to_meta.insert(
            key,
            NodeMeta {
                node_id,
                workspace_id,
                revision,
            },
        );

        Ok(())
    }

    /// Remove a vector from the index.
    pub fn remove(&mut self, node_id: &str) -> Result<()> {
        if let Some(&key) = self.node_to_key.get(node_id) {
            self.index.remove(key).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to remove vector: {}", e))
            })?;
            self.node_to_key.remove(node_id);
            self.key_to_meta.remove(&key);
        }
        Ok(())
    }

    /// Search for k nearest neighbors.
    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<SearchResult>> {
        if query.len() != self.dimensions {
            return Err(raisin_error::Error::storage(format!(
                "Query dimension mismatch: expected {}, got {}",
                self.dimensions,
                query.len()
            )));
        }

        if self.node_to_key.is_empty() {
            return Ok(Vec::new());
        }

        let matches = self
            .index
            .search(query, k)
            .map_err(|e| raisin_error::Error::storage(format!("Search failed: {}", e)))?;

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

        Ok(results)
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
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        crate::persistence::save_to_file(self, path.as_ref())
    }

    /// Load index from file, auto-detecting old vs new format.
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        crate::persistence::load_from_file(path.as_ref())
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

    pub(crate) fn dimensions(&self) -> usize {
        self.dimensions
    }

    pub(crate) fn next_key(&self) -> u64 {
        self.next_key
    }
}

impl DistanceMetric {
    /// Convert to usearch MetricKind.
    pub(crate) fn to_usearch_metric(self) -> MetricKind {
        match self {
            DistanceMetric::Cosine => MetricKind::Cos,
            DistanceMetric::L2 => MetricKind::L2sq,
            DistanceMetric::InnerProduct => MetricKind::IP,
            // Manhattan: no native L1 in usearch. Falls back to L2sq which
            // preserves ordering for most use cases but is not a true L1 metric.
            DistanceMetric::Manhattan => {
                tracing::warn!(
                    "Manhattan (L1) distance is not natively supported by usearch; \
                     falling back to L2sq (squared Euclidean)"
                );
                MetricKind::L2sq
            }
            DistanceMetric::Hamming => MetricKind::Hamming,
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
        assert!(!DistanceMetric::Manhattan.requires_normalization());
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
}
