//! Persistent graph projection serialization type

use serde::{Deserialize, Serialize};

/// A graph projection serialized for persistent storage in the GRAPH_PROJECTION column family.
///
/// Stores the edge list and node ID mapping. The CSR (Compressed Sparse Row) structure
/// is rebuilt in memory from this data -- CSR has internal pointers that don't serialize well,
/// but rebuilding from edge list is O(E log E) and sub-millisecond for 100K edges.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedProjection {
    /// All node IDs in the projection (position = integer ID in CSR)
    pub nodes: Vec<String>,
    /// Directed edges as (source_index, target_index) into the nodes vec
    pub edges: Vec<(u32, u32)>,
    /// Optional edge weights (same length as edges). None = unweighted graph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weights: Option<Vec<f64>>,
    /// HLC revision string when this projection was built
    pub revision: String,
    /// Unix timestamp (millis) when this projection was built
    pub built_at: u64,
    /// Whether this projection is stale (needs rebuild)
    #[serde(default)]
    pub stale: bool,
}

impl PersistedProjection {
    /// Create from a GraphProjection and revision string
    pub fn from_projection(
        projection: &raisin_graph_algorithms::GraphProjection,
        revision: String,
    ) -> Self {
        let node_count = projection.node_count();
        let graph = projection.graph();

        // Collect node IDs in order
        let mut nodes = Vec::with_capacity(node_count);
        for i in 0..node_count {
            if let Some(id) = projection.get_node_id(i as u32) {
                nodes.push(id.clone());
            }
        }

        // Collect edges from CSR
        let mut edges = Vec::with_capacity(graph.edge_count());
        for u in 0..node_count {
            if u < graph.node_count() {
                for &v in graph.neighbors_slice(u as u32) {
                    edges.push((u as u32, v));
                }
            }
        }

        // Collect weights if present
        let weights = projection.edge_weights().map(|w| w.to_vec());

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            nodes,
            edges,
            weights,
            revision,
            built_at: now,
            stale: false,
        }
    }

    /// Convert back to a GraphProjection
    pub fn to_projection(&self) -> raisin_graph_algorithms::GraphProjection {
        // Rebuild node ID mapping
        let nodes = self.nodes.clone();

        // Convert edges back to (String, String) format
        if let Some(ref weights) = self.weights {
            let weighted_edges: Vec<(String, String, f64)> = self
                .edges
                .iter()
                .zip(weights.iter())
                .filter_map(|(&(src, tgt), &w)| {
                    let src_id = self.nodes.get(src as usize)?;
                    let tgt_id = self.nodes.get(tgt as usize)?;
                    Some((src_id.clone(), tgt_id.clone(), w))
                })
                .collect();
            raisin_graph_algorithms::GraphProjection::from_parts_weighted(nodes, weighted_edges)
        } else {
            let edges: Vec<(String, String)> = self
                .edges
                .iter()
                .filter_map(|&(src, tgt)| {
                    let src_id = self.nodes.get(src as usize)?;
                    let tgt_id = self.nodes.get(tgt as usize)?;
                    Some((src_id.clone(), tgt_id.clone()))
                })
                .collect();
            raisin_graph_algorithms::GraphProjection::from_parts(nodes, edges)
        }
    }

    /// Check if this projection is stale
    pub fn is_stale(&self) -> bool {
        self.stale
    }

    /// Mark as stale
    pub fn mark_stale(&mut self) {
        self.stale = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_persisted_projection_roundtrip() {
        let nodes = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let edges = vec![
            ("a".to_string(), "b".to_string()),
            ("b".to_string(), "c".to_string()),
        ];
        let projection = raisin_graph_algorithms::GraphProjection::from_parts(nodes, edges);

        let persisted = PersistedProjection::from_projection(&projection, "rev1".to_string());
        assert_eq!(persisted.nodes.len(), 3);
        assert_eq!(persisted.edges.len(), 2);
        assert!(persisted.weights.is_none());
        assert!(!persisted.stale);

        let restored = persisted.to_projection();
        assert_eq!(restored.node_count(), 3);
        assert_eq!(restored.edge_count(), 2);
    }

    #[test]
    fn test_persisted_projection_weighted_roundtrip() {
        let nodes = vec!["a".to_string(), "b".to_string()];
        let edges = vec![("a".to_string(), "b".to_string(), 2.5)];
        let projection =
            raisin_graph_algorithms::GraphProjection::from_parts_weighted(nodes, edges);

        let persisted = PersistedProjection::from_projection(&projection, "rev2".to_string());
        assert!(persisted.weights.is_some());
        assert_eq!(persisted.weights.as_ref().unwrap().len(), 1);

        let restored = persisted.to_projection();
        assert_eq!(restored.node_count(), 2);
        assert!(restored.has_weights());
    }

    #[test]
    fn test_persisted_projection_serialization() {
        let persisted = PersistedProjection {
            nodes: vec!["x".to_string(), "y".to_string()],
            edges: vec![(0, 1)],
            weights: None,
            revision: "hlc123".to_string(),
            built_at: 1000,
            stale: false,
        };

        // MessagePack roundtrip (named encoding, matching production usage)
        let bytes = rmp_serde::to_vec_named(&persisted).expect("serialize");
        let restored: PersistedProjection = rmp_serde::from_slice(&bytes).expect("deserialize");

        assert_eq!(restored.nodes, persisted.nodes);
        assert_eq!(restored.edges, persisted.edges);
        assert_eq!(restored.revision, persisted.revision);
    }
}
