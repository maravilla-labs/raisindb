use hashbrown::HashMap;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// A vector clock for tracking causal dependencies in a distributed system.
///
/// Each cluster node (server instance) in the cluster maintains a counter. Vector clocks enable:
/// - Detecting causal relationships (happens-before)
/// - Identifying concurrent operations
/// - Providing a partial ordering of events
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VectorClock {
    /// Map from cluster_node_id to that node's logical clock value
    clock: HashMap<String, u64>,
}

/// Comparison result for vector clocks
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockOrdering {
    /// This clock happened before the other (this < other)
    Before,
    /// This clock happened after the other (this > other)
    After,
    /// The clocks are concurrent (neither < nor >)
    Concurrent,
    /// The clocks are equal
    Equal,
}

impl VectorClock {
    /// Create a new empty vector clock
    pub fn new() -> Self {
        Self {
            clock: HashMap::new(),
        }
    }

    /// Create a vector clock with an initial value for a cluster node
    ///
    /// # Arguments
    /// * `cluster_node_id` - Cluster node ID (server instance identifier)
    /// * `counter` - Initial counter value
    pub fn with_initial(cluster_node_id: String, counter: u64) -> Self {
        let mut clock = HashMap::new();
        clock.insert(cluster_node_id, counter);
        Self { clock }
    }

    /// Increment the counter for a specific cluster node and return the new value
    ///
    /// This should be called whenever a cluster node performs a local operation.
    ///
    /// # Arguments
    /// * `cluster_node_id` - Cluster node ID (server instance identifier)
    pub fn increment(&mut self, cluster_node_id: &str) -> u64 {
        let counter = self.clock.entry(cluster_node_id.to_string()).or_insert(0);
        *counter += 1;
        *counter
    }

    /// Get the current counter value for a cluster node
    ///
    /// # Arguments
    /// * `cluster_node_id` - Cluster node ID (server instance identifier)
    pub fn get(&self, cluster_node_id: &str) -> u64 {
        self.clock.get(cluster_node_id).copied().unwrap_or(0)
    }

    /// Set the counter for a specific cluster node
    ///
    /// # Arguments
    /// * `cluster_node_id` - Cluster node ID (server instance identifier)
    /// * `counter` - New counter value
    pub fn set(&mut self, cluster_node_id: &str, counter: u64) {
        if counter > 0 {
            self.clock.insert(cluster_node_id.to_string(), counter);
        } else {
            self.clock.remove(cluster_node_id);
        }
    }

    /// Merge this vector clock with another, taking the maximum for each cluster node
    ///
    /// This implements the merge operation: VC1 ∪ VC2 = max(VC1[i], VC2[i]) for all i
    pub fn merge(&mut self, other: &VectorClock) {
        for (cluster_node_id, &other_counter) in &other.clock {
            let counter = self.clock.entry(cluster_node_id.clone()).or_insert(0);
            *counter = (*counter).max(other_counter);
        }
    }

    /// Create a new vector clock that is the merge of two clocks
    pub fn merged(mut self, other: &VectorClock) -> Self {
        self.merge(other);
        self
    }

    /// Check if this clock happened before another clock
    ///
    /// VC1 < VC2 iff VC1[i] <= VC2[i] for all i AND VC1 != VC2
    pub fn happens_before(&self, other: &VectorClock) -> bool {
        matches!(self.compare(other), ClockOrdering::Before)
    }

    /// Check if this clock happened after another clock
    ///
    /// VC1 > VC2 iff VC2 < VC1
    pub fn happens_after(&self, other: &VectorClock) -> bool {
        matches!(self.compare(other), ClockOrdering::After)
    }

    /// Check if this clock is concurrent with another clock
    ///
    /// Two clocks are concurrent if neither happened before the other
    pub fn concurrent_with(&self, other: &VectorClock) -> bool {
        matches!(self.compare(other), ClockOrdering::Concurrent)
    }

    /// Compare two vector clocks to determine their causal relationship
    pub fn compare(&self, other: &VectorClock) -> ClockOrdering {
        let mut any_less = false;
        let mut any_greater = false;

        // Get all cluster node IDs from both clocks
        let all_nodes: std::collections::HashSet<_> =
            self.clock.keys().chain(other.clock.keys()).collect();

        for cluster_node_id in all_nodes {
            let self_val = self.get(cluster_node_id);
            let other_val = other.get(cluster_node_id);

            match self_val.cmp(&other_val) {
                Ordering::Less => any_less = true,
                Ordering::Greater => any_greater = true,
                Ordering::Equal => {}
            }
        }

        match (any_less, any_greater) {
            (false, false) => ClockOrdering::Equal,
            (true, false) => ClockOrdering::Before,
            (false, true) => ClockOrdering::After,
            (true, true) => ClockOrdering::Concurrent,
        }
    }

    /// Calculate the "distance" between two clocks
    ///
    /// This is useful for monitoring replication lag. The distance is the sum
    /// of differences for all cluster nodes where the other clock is ahead.
    pub fn distance(&self, other: &VectorClock) -> u64 {
        let mut total_distance = 0;

        for (cluster_node_id, &other_val) in &other.clock {
            let self_val = self.get(cluster_node_id);
            if other_val > self_val {
                total_distance += other_val - self_val;
            }
        }

        total_distance
    }

    /// Get all cluster node IDs tracked in this clock
    pub fn node_ids(&self) -> impl Iterator<Item = &String> {
        self.clock.keys()
    }

    /// Get the number of cluster nodes tracked
    pub fn len(&self) -> usize {
        self.clock.len()
    }

    /// Check if the clock is empty
    pub fn is_empty(&self) -> bool {
        self.clock.is_empty()
    }

    /// Get a reference to the internal clock map
    pub fn as_map(&self) -> &HashMap<String, u64> {
        &self.clock
    }

    /// Convert to a map of cluster node watermarks (highest sequence per cluster node)
    pub fn to_watermarks(&self) -> HashMap<String, u64> {
        self.clock.clone()
    }
}

impl Default for VectorClock {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for VectorClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{")?;
        let mut first = true;
        for (cluster_node_id, counter) in &self.clock {
            if !first {
                write!(f, ", ")?;
            }
            write!(f, "{}: {}", cluster_node_id, counter)?;
            first = false;
        }
        write!(f, "}}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_increment() {
        let mut vc = VectorClock::new();
        assert_eq!(vc.increment("node1"), 1);
        assert_eq!(vc.increment("node1"), 2);
        assert_eq!(vc.increment("node2"), 1);
        assert_eq!(vc.get("node1"), 2);
        assert_eq!(vc.get("node2"), 1);
    }

    #[test]
    fn test_merge() {
        let mut vc1 = VectorClock::new();
        vc1.increment("node1");
        vc1.increment("node1");
        vc1.increment("node2");

        let mut vc2 = VectorClock::new();
        vc2.increment("node1");
        vc2.increment("node2");
        vc2.increment("node2");
        vc2.increment("node3");

        vc1.merge(&vc2);

        assert_eq!(vc1.get("node1"), 2); // max(2, 1) = 2
        assert_eq!(vc1.get("node2"), 2); // max(1, 2) = 2
        assert_eq!(vc1.get("node3"), 1); // max(0, 1) = 1
    }

    #[test]
    fn test_happens_before() {
        let mut vc1 = VectorClock::new();
        vc1.increment("node1");

        let mut vc2 = VectorClock::new();
        vc2.increment("node1");
        vc2.increment("node1");

        assert!(vc1.happens_before(&vc2));
        assert!(!vc2.happens_before(&vc1));
        assert!(vc2.happens_after(&vc1));
    }

    #[test]
    fn test_concurrent() {
        let mut vc1 = VectorClock::new();
        vc1.increment("node1");
        vc1.increment("node1");

        let mut vc2 = VectorClock::new();
        vc2.increment("node2");

        assert!(vc1.concurrent_with(&vc2));
        assert!(vc2.concurrent_with(&vc1));
        assert!(!vc1.happens_before(&vc2));
        assert!(!vc2.happens_before(&vc1));
    }

    #[test]
    fn test_equal() {
        let mut vc1 = VectorClock::new();
        vc1.increment("node1");
        vc1.increment("node2");

        let mut vc2 = VectorClock::new();
        vc2.increment("node1");
        vc2.increment("node2");

        assert_eq!(vc1.compare(&vc2), ClockOrdering::Equal);
    }

    #[test]
    fn test_distance() {
        let mut vc1 = VectorClock::new();
        vc1.set("node1", 5);
        vc1.set("node2", 3);

        let mut vc2 = VectorClock::new();
        vc2.set("node1", 10);
        vc2.set("node2", 3);
        vc2.set("node3", 7);

        // vc1 is behind vc2 by: (10-5) + (7-0) = 12
        assert_eq!(vc1.distance(&vc2), 12);

        // vc2 is not behind vc1
        assert_eq!(vc2.distance(&vc1), 0);
    }

    #[test]
    fn test_serialization() {
        let mut vc = VectorClock::new();
        vc.increment("node1");
        vc.increment("node2");
        vc.increment("node1");

        let json = serde_json::to_string(&vc).unwrap();
        let vc2: VectorClock = serde_json::from_str(&json).unwrap();

        assert_eq!(vc, vc2);
    }
}
