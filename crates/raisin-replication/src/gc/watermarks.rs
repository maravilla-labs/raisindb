//! Peer watermark tracking for garbage collection

use hashbrown::HashMap;
use serde::{Deserialize, Serialize};

/// Tracks the highest sequence number acknowledged by each peer
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PeerWatermarks {
    /// Map of {node_id -> highest acknowledged op_seq}
    watermarks: HashMap<String, u64>,
}

impl PeerWatermarks {
    /// Create a new watermark tracker
    pub fn new() -> Self {
        Self {
            watermarks: HashMap::new(),
        }
    }

    /// Update a peer's watermark to the highest acknowledged sequence
    pub fn update(&mut self, node_id: String, op_seq: u64) {
        self.watermarks
            .entry(node_id)
            .and_modify(|existing| {
                if op_seq > *existing {
                    *existing = op_seq;
                }
            })
            .or_insert(op_seq);
    }

    /// Get the minimum watermark across all peers (safe-to-delete threshold)
    ///
    /// This is the highest sequence number that ALL peers have acknowledged.
    /// Operations with op_seq <= this value are safe to delete.
    pub fn min_watermark(&self) -> u64 {
        self.watermarks.values().copied().min().unwrap_or(0)
    }

    /// Get the watermark for a specific peer
    pub fn get_watermark(&self, node_id: &str) -> u64 {
        self.watermarks.get(node_id).copied().unwrap_or(0)
    }

    /// Get all known peers
    pub fn peers(&self) -> Vec<String> {
        self.watermarks.keys().cloned().collect()
    }

    /// Check if a peer is known
    pub fn has_peer(&self, node_id: &str) -> bool {
        self.watermarks.contains_key(node_id)
    }

    /// Remove a peer (when it's permanently offline)
    pub fn remove_peer(&mut self, node_id: &str) {
        self.watermarks.remove(node_id);
    }
}
