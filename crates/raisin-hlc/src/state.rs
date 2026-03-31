// SPDX-License-Identifier: BSL-1.1

//! Lock-free HLC state management for distributed nodes

use crate::{current_time_ms, HLCError, Result, HLC};
use std::sync::atomic::{AtomicU64, Ordering};

/// Maximum allowed clock skew before warning (5 seconds)
const MAX_CLOCK_SKEW_MS: u64 = 5_000;

/// Lock-free HLC state for a single node
///
/// This structure maintains the HLC state for a node using atomic operations,
/// ensuring thread-safe, lock-free timestamp generation. The implementation
/// uses compare-and-swap loops to guarantee monotonicity without locks.
///
/// # Performance
///
/// - `tick()`: Target <100ns per call (lock-free CAS loop)
/// - `update()`: Target <200ns per call (includes clock skew detection)
///
/// # Thread Safety
///
/// All operations are lock-free and thread-safe. Multiple threads can call
/// `tick()` and `update()` concurrently without synchronization.
///
/// # Example
///
/// ```
/// use raisin_hlc::NodeHLCState;
///
/// let state = NodeHLCState::new("node-1".to_string());
///
/// // Generate timestamps concurrently
/// let hlc1 = state.tick();
/// let hlc2 = state.tick();
/// assert!(hlc2 > hlc1);
///
/// // Update from remote node
/// let remote = state.tick();
/// let hlc3 = state.update(&remote);
/// assert!(hlc3 >= remote);
/// ```
pub struct NodeHLCState {
    /// Last observed timestamp in milliseconds
    last_timestamp: AtomicU64,
    /// Logical counter for same-millisecond events
    last_counter: AtomicU64,
    /// Unique identifier for this node
    node_id: String,
}

impl NodeHLCState {
    /// Creates a new HLC state for a node
    ///
    /// Initializes the state with the current wall clock time and counter 0.
    ///
    /// # Arguments
    ///
    /// * `node_id` - Unique identifier for this node in the cluster
    ///
    /// # Example
    ///
    /// ```
    /// use raisin_hlc::NodeHLCState;
    ///
    /// let state = NodeHLCState::new("node-1".to_string());
    /// ```
    pub fn new(node_id: String) -> Self {
        let now = current_time_ms();
        Self {
            last_timestamp: AtomicU64::new(now),
            last_counter: AtomicU64::new(0),
            node_id,
        }
    }

    /// Creates HLC state initialized with a specific timestamp
    ///
    /// Useful for restoring state from persistence.
    ///
    /// # Arguments
    ///
    /// * `node_id` - Unique identifier for this node
    /// * `initial_hlc` - Initial HLC value to restore
    ///
    /// # Example
    ///
    /// ```
    /// use raisin_hlc::{NodeHLCState, HLC};
    ///
    /// let initial = HLC::new(1000, 42);
    /// let state = NodeHLCState::with_initial("node-1".to_string(), initial);
    /// ```
    pub fn with_initial(node_id: String, initial_hlc: HLC) -> Self {
        Self {
            last_timestamp: AtomicU64::new(initial_hlc.timestamp_ms),
            last_counter: AtomicU64::new(initial_hlc.counter),
            node_id,
        }
    }

    /// Returns the node ID
    #[inline]
    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// Generates the next HLC timestamp for a local event
    ///
    /// This is the primary operation for generating timestamps. It guarantees:
    /// - Monotonicity: each call returns a strictly greater HLC
    /// - Causality: the HLC advances with wall clock time
    /// - Thread safety: lock-free via compare-and-swap
    ///
    /// # Algorithm
    ///
    /// 1. Read current wall clock time
    /// 2. Compare with last HLC timestamp:
    ///    - If wall clock > last timestamp: use wall clock, reset counter to 0
    ///    - If wall clock = last timestamp: keep timestamp, increment counter
    ///    - If wall clock < last timestamp: keep last timestamp, increment counter
    /// 3. Update atomically using CAS, retry on contention
    ///
    /// # Performance
    ///
    /// Target: <100ns per call (typically completes in 1-2 CAS iterations)
    ///
    /// # Example
    ///
    /// ```
    /// use raisin_hlc::NodeHLCState;
    ///
    /// let state = NodeHLCState::new("node-1".to_string());
    /// let hlc1 = state.tick();
    /// let hlc2 = state.tick();
    /// assert!(hlc2 > hlc1);
    /// ```
    pub fn tick(&self) -> HLC {
        loop {
            let wall_clock = current_time_ms();
            let last_ts = self.last_timestamp.load(Ordering::Acquire);
            let last_cnt = self.last_counter.load(Ordering::Acquire);

            let (new_ts, new_cnt) = if wall_clock > last_ts {
                // Wall clock advanced: use wall clock time, reset counter
                (wall_clock, 0)
            } else {
                // Wall clock same or behind: keep last timestamp, increment counter
                (last_ts, last_cnt + 1)
            };

            // Atomic compare-and-swap for timestamp
            if self
                .last_timestamp
                .compare_exchange(last_ts, new_ts, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                // Timestamp updated successfully, now update counter
                if self
                    .last_counter
                    .compare_exchange(last_cnt, new_cnt, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    return HLC::new(new_ts, new_cnt);
                }
            }

            // CAS failed (contention), retry
            // Note: This is expected under high concurrency and is part of the lock-free design
        }
    }

    /// Updates local HLC state from a remote HLC timestamp
    ///
    /// This operation is called when receiving operations from other nodes during
    /// replication. It ensures the local clock advances to be consistent with the
    /// remote clock while maintaining monotonicity.
    ///
    /// # Algorithm
    ///
    /// 1. Read current wall clock and local HLC state
    /// 2. Compute new timestamp as max(wall_clock, local_timestamp, remote_timestamp)
    /// 3. If new timestamp equals remote timestamp: use remote.counter + 1
    ///    Else if new timestamp equals local timestamp: use local.counter + 1
    ///    Otherwise: reset counter to 0
    /// 4. Update atomically and return the new HLC
    ///
    /// # Clock Skew Detection
    ///
    /// If the remote timestamp is significantly ahead of the wall clock (>5s),
    /// this indicates potential clock skew and logs a warning.
    ///
    /// # Performance
    ///
    /// Target: <200ns per call
    ///
    /// # Example
    ///
    /// ```
    /// use raisin_hlc::{NodeHLCState, HLC};
    ///
    /// let state = NodeHLCState::new("node-1".to_string());
    /// let remote = HLC::new(1000, 42);
    /// let updated = state.update(&remote);
    /// assert!(updated >= remote);
    /// ```
    pub fn update(&self, remote: &HLC) -> HLC {
        loop {
            let wall_clock = current_time_ms();
            let last_ts = self.last_timestamp.load(Ordering::Acquire);
            let last_cnt = self.last_counter.load(Ordering::Acquire);

            // Check for clock skew (remote significantly ahead of wall clock)
            if remote.timestamp_ms > wall_clock + MAX_CLOCK_SKEW_MS {
                let delta = remote.timestamp_ms - wall_clock;
                eprintln!(
                    "WARNING: Clock skew detected on node {}: remote timestamp {}ms ahead of wall clock (delta: {}ms)",
                    self.node_id, remote.timestamp_ms, delta
                );
            }

            // Compute new HLC: max of all timestamps
            let max_ts = wall_clock.max(last_ts).max(remote.timestamp_ms);

            let new_cnt = if max_ts == remote.timestamp_ms {
                // Use remote counter + 1 when remote timestamp is the max
                remote.counter + 1
            } else if max_ts == last_ts {
                // Use local counter + 1 when local timestamp is the max
                last_cnt + 1
            } else {
                // Wall clock advanced, reset counter
                0
            };

            // Atomic update
            if self
                .last_timestamp
                .compare_exchange(last_ts, max_ts, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
                && self
                    .last_counter
                    .compare_exchange(last_cnt, new_cnt, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                return HLC::new(max_ts, new_cnt);
            }

            // CAS failed, retry
        }
    }

    /// Returns the current HLC state without advancing it
    ///
    /// This is primarily for debugging and monitoring. For generating timestamps,
    /// use `tick()` instead.
    ///
    /// # Note
    ///
    /// The returned HLC may already be outdated by the time it's examined due to
    /// concurrent operations.
    ///
    /// # Example
    ///
    /// ```
    /// use raisin_hlc::NodeHLCState;
    ///
    /// let state = NodeHLCState::new("node-1".to_string());
    /// state.tick();
    /// let current = state.current();
    /// ```
    pub fn current(&self) -> HLC {
        let timestamp = self.last_timestamp.load(Ordering::Acquire);
        let counter = self.last_counter.load(Ordering::Acquire);
        HLC::new(timestamp, counter)
    }

    /// Validates the HLC state for consistency
    ///
    /// Checks for clock skew by comparing the stored timestamp against the
    /// current wall clock.
    ///
    /// # Errors
    ///
    /// Returns `HLCError::ClockSkew` if the stored timestamp is significantly
    /// behind the wall clock (indicating system time jumped forward).
    ///
    /// # Example
    ///
    /// ```
    /// use raisin_hlc::NodeHLCState;
    ///
    /// let state = NodeHLCState::new("node-1".to_string());
    /// assert!(state.validate().is_ok());
    /// ```
    pub fn validate(&self) -> Result<()> {
        let wall_clock = current_time_ms();
        let last_ts = self.last_timestamp.load(Ordering::Acquire);

        // Check if wall clock jumped significantly forward (e.g., after suspend/resume)
        if wall_clock > last_ts + MAX_CLOCK_SKEW_MS {
            return Err(HLCError::ClockSkew {
                wall_clock_ms: wall_clock,
                hlc_timestamp_ms: last_ts,
                delta_ms: wall_clock - last_ts,
            });
        }

        Ok(())
    }
}

// Implement Send + Sync manually to document thread-safety guarantees
unsafe impl Send for NodeHLCState {}
unsafe impl Sync for NodeHLCState {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_tick_monotonicity() {
        let state = NodeHLCState::new("test-node".to_string());

        let hlc1 = state.tick();
        let hlc2 = state.tick();
        let hlc3 = state.tick();

        assert!(hlc2 > hlc1);
        assert!(hlc3 > hlc2);
    }

    #[test]
    fn test_tick_same_millisecond() {
        let state = NodeHLCState::new("test-node".to_string());

        // Generate multiple ticks rapidly (likely same millisecond)
        let hlc1 = state.tick();
        let hlc2 = state.tick();
        let hlc3 = state.tick();

        // All should be monotonically increasing
        assert!(hlc2 > hlc1);
        assert!(hlc3 > hlc2);

        // If they're in the same millisecond, counters should increment
        if hlc1.timestamp_ms == hlc2.timestamp_ms {
            assert_eq!(hlc2.counter, hlc1.counter + 1);
        }
    }

    #[test]
    fn test_update_from_remote() {
        let state = NodeHLCState::new("test-node".to_string());

        let local1 = state.tick();

        // Simulate remote timestamp in the future
        let remote = HLC::new(local1.timestamp_ms + 1000, 42);
        let updated = state.update(&remote);

        // Updated HLC should be >= remote
        assert!(updated >= remote);
        assert_eq!(updated.timestamp_ms, remote.timestamp_ms);
        // Counter should be remote + 1
        assert_eq!(updated.counter, remote.counter + 1);

        // Subsequent ticks should be even greater
        let local2 = state.tick();
        assert!(local2 > updated);
    }

    #[test]
    fn test_update_with_old_remote() {
        let state = NodeHLCState::new("test-node".to_string());

        let local1 = state.tick();

        // Simulate remote timestamp in the past
        let remote = HLC::new(local1.timestamp_ms.saturating_sub(1000), 10);
        let updated = state.update(&remote);

        // Updated HLC should be >= local
        assert!(updated >= local1);
    }

    #[test]
    fn test_with_initial() {
        let initial = HLC::new(1000, 42);
        let state = NodeHLCState::with_initial("test-node".to_string(), initial);

        let current = state.current();
        assert_eq!(current.timestamp_ms, 1000);
        assert_eq!(current.counter, 42);

        // Next tick should be greater
        let next = state.tick();
        assert!(next > initial);
    }

    #[test]
    fn test_current() {
        let state = NodeHLCState::new("test-node".to_string());

        let hlc1 = state.tick();
        let current = state.current();

        // Current should reflect last tick
        assert!(current >= hlc1);
    }

    #[test]
    fn test_node_id() {
        let state = NodeHLCState::new("my-node".to_string());
        assert_eq!(state.node_id(), "my-node");
    }

    #[test]
    fn test_concurrent_ticks() {
        let state = Arc::new(NodeHLCState::new("test-node".to_string()));
        let num_threads = 10;
        let ticks_per_thread = 1000;

        let mut handles = vec![];

        for _ in 0..num_threads {
            let state_clone = Arc::clone(&state);
            let handle = thread::spawn(move || {
                let mut last = None;
                for _ in 0..ticks_per_thread {
                    let hlc = state_clone.tick();
                    if let Some(prev) = last {
                        assert!(hlc > prev, "Monotonicity violated");
                    }
                    last = Some(hlc);
                }
                last.unwrap()
            });
            handles.push(handle);
        }

        let mut results = vec![];
        for handle in handles {
            results.push(handle.join().unwrap());
        }

        // All results should be unique and monotonic isn't guaranteed across threads
        // but we should have generated num_threads * ticks_per_thread unique timestamps
        // Verify no panics occurred (which would indicate monotonicity violations)
    }

    #[test]
    fn test_concurrent_updates() {
        let state = Arc::new(NodeHLCState::new("test-node".to_string()));
        let num_threads = 10;

        let mut handles = vec![];

        for i in 0..num_threads {
            let state_clone = Arc::clone(&state);
            let handle = thread::spawn(move || {
                // Each thread updates with a different remote timestamp
                let remote = HLC::new(1000 + (i as u64 * 100), i as u64);
                state_clone.update(&remote)
            });
            handles.push(handle);
        }

        let mut results = vec![];
        for handle in handles {
            results.push(handle.join().unwrap());
        }

        // Final state should be >= all remote timestamps
        let final_state = state.current();
        for result in results {
            assert!(final_state >= result);
        }
    }

    #[test]
    fn test_validate() {
        let state = NodeHLCState::new("test-node".to_string());
        assert!(state.validate().is_ok());

        // Validation should succeed even after ticks
        state.tick();
        state.tick();
        assert!(state.validate().is_ok());
    }

    #[test]
    fn test_mixed_operations() {
        let state = Arc::new(NodeHLCState::new("test-node".to_string()));
        let num_threads = 8;

        let mut handles = vec![];

        for i in 0..num_threads {
            let state_clone = Arc::clone(&state);
            let handle = thread::spawn(move || {
                let mut local_hlcs = vec![];

                for j in 0..100 {
                    if (i + j) % 3 == 0 {
                        // Tick
                        local_hlcs.push(state_clone.tick());
                    } else {
                        // Update from simulated remote
                        let remote = HLC::new(1000 + (j as u64 * 10), j as u64);
                        local_hlcs.push(state_clone.update(&remote));
                    }
                }

                // Verify local monotonicity
                for window in local_hlcs.windows(2) {
                    assert!(window[1] > window[0], "Local monotonicity violated");
                }

                local_hlcs.last().unwrap().clone()
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        // Should complete without panics
    }
}
