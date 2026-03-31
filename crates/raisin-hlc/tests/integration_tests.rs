// SPDX-License-Identifier: BSL-1.1

//! Integration tests for HLC implementation
//!
//! These tests verify the correctness and performance characteristics of the
//! HLC implementation under various scenarios.

use raisin_hlc::{NodeHLCState, HLC};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

#[test]
fn test_monotonicity_single_thread() {
    let state = NodeHLCState::new("node-1".to_string());
    let mut prev = state.tick();

    for _ in 0..10_000 {
        let current = state.tick();
        assert!(
            current > prev,
            "Monotonicity violated: {} not > {}",
            current,
            prev
        );
        prev = current;
    }
}

#[test]
fn test_monotonicity_multi_thread() {
    let state = Arc::new(NodeHLCState::new("node-1".to_string()));
    let num_threads = 10;
    let iterations = 1_000;

    let mut handles = vec![];

    for _ in 0..num_threads {
        let state = Arc::clone(&state);
        let handle = thread::spawn(move || {
            let mut prev = None;

            for _ in 0..iterations {
                let current = state.tick();

                if let Some(p) = prev {
                    assert!(
                        current > p,
                        "Thread-local monotonicity violated: {} not > {}",
                        current,
                        p
                    );
                }

                prev = Some(current);
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().expect("Thread panicked");
    }
}

#[test]
fn test_ordering_properties() {
    let hlc1 = HLC::new(1000, 0);
    let hlc2 = HLC::new(1000, 1);
    let hlc3 = HLC::new(1001, 0);

    // Test all comparison operators
    assert!(hlc1 < hlc2);
    assert!(hlc2 < hlc3);
    assert!(hlc1 < hlc3);

    assert!(hlc2 > hlc1);
    assert!(hlc3 > hlc2);
    assert!(hlc3 > hlc1);

    assert!(hlc1 <= hlc1);
    assert!(hlc1 <= hlc2);

    assert!(hlc2 >= hlc2);
    assert!(hlc2 >= hlc1);

    assert_eq!(hlc1, hlc1);
    assert_ne!(hlc1, hlc2);
}

#[test]
fn test_descending_encoding_order() {
    let test_cases = vec![
        (HLC::new(1000, 0), HLC::new(2000, 0)),
        (HLC::new(1000, 0), HLC::new(1000, 1)),
        (HLC::new(0, 0), HLC::new(1, 0)),
        (HLC::new(0, 0), HLC::new(0, 1)),
        (HLC::new(u64::MAX - 1, 0), HLC::new(u64::MAX, 0)),
    ];

    for (older, newer) in test_cases {
        let older_bytes = older.encode_descending();
        let newer_bytes = newer.encode_descending();

        assert!(newer > older, "Expected {} > {}", newer, older);

        assert!(
            newer_bytes < older_bytes,
            "Descending encoding violated: newer HLC should have smaller bytes. \
             newer={} (bytes={:?}), older={} (bytes={:?})",
            newer,
            &newer_bytes[..],
            older,
            &older_bytes[..]
        );
    }
}

#[test]
fn test_encoding_roundtrip_comprehensive() {
    let test_cases = vec![
        HLC::new(0, 0),
        HLC::new(1, 0),
        HLC::new(0, 1),
        HLC::new(1, 1),
        HLC::new(u64::MAX, 0),
        HLC::new(0, u64::MAX),
        HLC::new(u64::MAX, u64::MAX),
        HLC::new(1705843009213693952, 42),
        HLC::new(1000000000, 999999999),
    ];

    for original in test_cases {
        let encoded = original.encode_descending();
        assert_eq!(encoded.len(), 16, "Encoding should be 16 bytes");

        let decoded = HLC::decode_descending(&encoded).expect("Decoding should succeed");

        assert_eq!(
            original, decoded,
            "Roundtrip failed: original={}, decoded={}",
            original, decoded
        );
    }
}

#[test]
fn test_string_format_roundtrip() {
    let test_cases = vec![
        HLC::new(0, 0),
        HLC::new(1, 2),
        HLC::new(1705843009213693952, 42),
        HLC::new(u64::MAX, u64::MAX),
    ];

    for original in test_cases {
        let string = original.to_string();
        let parsed: HLC = string.parse().expect("Parsing should succeed");

        assert_eq!(
            original, parsed,
            "String roundtrip failed: original={}, string='{}', parsed={}",
            original, string, parsed
        );
    }
}

#[test]
fn test_replication_scenario() {
    // Simulate 3-node replication scenario
    let node1 = Arc::new(NodeHLCState::new("node-1".to_string()));
    let node2 = Arc::new(NodeHLCState::new("node-2".to_string()));
    let node3 = Arc::new(NodeHLCState::new("node-3".to_string()));

    // Node 1 generates an operation
    let op1 = node1.tick();

    // Node 2 receives and processes the operation
    let op2 = node2.update(&op1);
    assert!(op2 >= op1, "Node 2 should advance past node 1's timestamp");

    // Node 2 generates its own operation
    let op3 = node2.tick();
    assert!(op3 > op2, "Subsequent operation should be newer");

    // Node 3 receives both operations
    let op4 = node3.update(&op1);
    let op5 = node3.update(&op3);

    assert!(op4 >= op1, "Node 3 should incorporate node 1's timestamp");
    assert!(op5 >= op3, "Node 3 should incorporate node 2's timestamp");
    assert!(op5 > op4, "Later update should produce newer timestamp");

    // All nodes should now have consistent ordering
    let final1 = node1.current();
    let final2 = node2.current();
    let final3 = node3.current();

    // The node with the most updates should have the highest timestamp
    assert!(final3 >= final2);
    assert!(final2 >= final1);
}

#[test]
fn test_concurrent_stress() {
    let state = Arc::new(NodeHLCState::new("stress-node".to_string()));
    let num_threads = 16;
    let iterations = 10_000;
    let barrier = Arc::new(Barrier::new(num_threads));

    let mut handles = vec![];

    for thread_id in 0..num_threads {
        let state = Arc::clone(&state);
        let barrier = Arc::clone(&barrier);

        let handle = thread::spawn(move || {
            // Synchronize start to maximize contention
            barrier.wait();

            let mut generated = vec![];

            for i in 0..iterations {
                let hlc = if i % 3 == 0 {
                    // Mix of ticks and updates
                    state.tick()
                } else {
                    let remote = HLC::new(1000 + (i as u64), thread_id as u64);
                    state.update(&remote)
                };

                generated.push(hlc);
            }

            // Verify thread-local monotonicity
            for window in generated.windows(2) {
                assert!(
                    window[1] > window[0],
                    "Thread {} monotonicity violated at {:?}",
                    thread_id,
                    window
                );
            }

            generated
        });

        handles.push(handle);
    }

    let mut all_hlcs = vec![];
    for handle in handles {
        let hlcs = handle.join().expect("Thread panicked");
        all_hlcs.extend(hlcs);
    }

    // Verify we generated expected number of unique timestamps
    println!("Generated {} total HLCs", all_hlcs.len());
    assert_eq!(all_hlcs.len(), num_threads * iterations);
}

#[test]
fn test_u128_conversion() {
    let test_cases = vec![
        HLC::new(0, 0),
        HLC::new(1, 0),
        HLC::new(0, 1),
        HLC::new(u64::MAX, 0),
        HLC::new(0, u64::MAX),
        HLC::new(u64::MAX, u64::MAX),
        HLC::new(1705843009213693952, 42),
    ];

    for original in test_cases {
        let numeric = original.as_u128();
        let decoded = HLC::from_u128(numeric);

        assert_eq!(
            original, decoded,
            "u128 roundtrip failed: original={}, numeric={}, decoded={}",
            original, numeric, decoded
        );

        // Verify the bit layout
        assert_eq!((numeric >> 64) as u64, original.timestamp_ms);
        assert_eq!((numeric & 0xFFFFFFFFFFFFFFFF) as u64, original.counter);
    }
}

#[test]
fn test_sorting() {
    let mut hlcs = vec![
        HLC::new(1000, 5),
        HLC::new(500, 0),
        HLC::new(1000, 0),
        HLC::new(2000, 1),
        HLC::new(1000, 3),
        HLC::new(2000, 0),
    ];

    hlcs.sort();

    // Verify sorted order
    for window in hlcs.windows(2) {
        assert!(window[0] <= window[1], "Sorting violated: {:?}", window);
    }

    assert_eq!(hlcs[0], HLC::new(500, 0));
    assert_eq!(hlcs[1], HLC::new(1000, 0));
    assert_eq!(hlcs[2], HLC::new(1000, 3));
    assert_eq!(hlcs[3], HLC::new(1000, 5));
    assert_eq!(hlcs[4], HLC::new(2000, 0));
    assert_eq!(hlcs[5], HLC::new(2000, 1));
}

#[test]
fn test_hash_consistency() {
    use std::collections::HashMap;

    let hlc1 = HLC::new(1000, 42);
    let hlc2 = HLC::new(1000, 42);
    let hlc3 = HLC::new(1000, 43);

    let mut map = HashMap::new();
    map.insert(hlc1, "value1");
    map.insert(hlc2, "value2"); // Should overwrite value1

    assert_eq!(map.len(), 1);
    assert_eq!(map.get(&hlc1), Some(&"value2"));
    assert_eq!(map.get(&hlc2), Some(&"value2"));
    assert_eq!(map.get(&hlc3), None);
}

#[test]
fn test_serialization_json() {
    let hlc = HLC::new(1705843009213693952, 42);

    let json = serde_json::to_string(&hlc).expect("Serialization failed");
    let deserialized: HLC = serde_json::from_str(&json).expect("Deserialization failed");

    assert_eq!(hlc, deserialized);
}

#[test]
fn test_clock_advancement() {
    let state = NodeHLCState::new("node-1".to_string());

    // Generate initial timestamp
    let hlc1 = state.tick();

    // Small delay to ensure wall clock advances
    thread::sleep(Duration::from_millis(2));

    let hlc2 = state.tick();

    // Timestamp should have advanced
    assert!(hlc2.timestamp_ms >= hlc1.timestamp_ms);

    if hlc2.timestamp_ms == hlc1.timestamp_ms {
        // Same millisecond - counter should increment
        assert_eq!(hlc2.counter, hlc1.counter + 1);
    } else {
        // Different millisecond - counter may reset
        assert!(hlc2.timestamp_ms > hlc1.timestamp_ms);
    }
}

#[test]
fn test_update_causality() {
    let state = NodeHLCState::new("node-1".to_string());

    let local1 = state.tick();

    // Remote timestamp from the "future"
    let remote = HLC::new(local1.timestamp_ms + 5000, 10);

    let updated = state.update(&remote);

    // After update, local clock should have jumped forward
    assert!(updated.timestamp_ms >= remote.timestamp_ms);

    // Subsequent local operations should maintain this forward progress
    let local2 = state.tick();
    assert!(local2 > updated);
    assert!(local2.timestamp_ms >= remote.timestamp_ms);
}

#[test]
fn test_uniqueness_under_contention() {
    let state = Arc::new(NodeHLCState::new("node-1".to_string()));
    let num_threads = 8;
    let iterations = 5_000;

    let mut handles = vec![];
    let barrier = Arc::new(Barrier::new(num_threads));

    for _ in 0..num_threads {
        let state = Arc::clone(&state);
        let barrier = Arc::clone(&barrier);

        let handle = thread::spawn(move || {
            barrier.wait(); // Synchronize for maximum contention

            let mut hlcs = Vec::with_capacity(iterations);
            for _ in 0..iterations {
                hlcs.push(state.tick());
            }
            hlcs
        });

        handles.push(handle);
    }

    let mut all_hlcs = HashSet::new();

    for handle in handles {
        let hlcs = handle.join().expect("Thread panicked");
        for hlc in hlcs {
            // Every HLC should be unique
            assert!(all_hlcs.insert(hlc), "Duplicate HLC generated: {}", hlc);
        }
    }

    assert_eq!(all_hlcs.len(), num_threads * iterations);
    println!(
        "Generated {} unique HLCs across {} threads",
        all_hlcs.len(),
        num_threads
    );
}

#[test]
fn test_validation() {
    let state = NodeHLCState::new("node-1".to_string());

    // Initial validation should pass
    assert!(state.validate().is_ok());

    // After ticks, validation should still pass
    for _ in 0..100 {
        state.tick();
    }

    assert!(state.validate().is_ok());
}

#[test]
fn test_continuous_generation() {
    let state = Arc::new(NodeHLCState::new("node-1".to_string()));
    let running = Arc::new(AtomicBool::new(true));
    let num_threads = 4;

    let mut handles = vec![];

    for _ in 0..num_threads {
        let state = Arc::clone(&state);
        let running = Arc::clone(&running);

        let handle = thread::spawn(move || {
            let mut count = 0;
            let mut prev = None;

            while running.load(Ordering::Relaxed) {
                let hlc = state.tick();

                if let Some(p) = prev {
                    assert!(hlc > p, "Monotonicity violated");
                }

                prev = Some(hlc);
                count += 1;
            }

            count
        });

        handles.push(handle);
    }

    // Run for a short duration
    thread::sleep(Duration::from_millis(100));
    running.store(false, Ordering::Relaxed);

    let mut total = 0;
    for handle in handles {
        total += handle.join().expect("Thread panicked");
    }

    println!(
        "Generated {} HLCs in 100ms across {} threads",
        total, num_threads
    );
    assert!(total > 0, "Should have generated some HLCs");
}

#[test]
fn test_encoding_stability() {
    // Verify that encoding is deterministic
    let hlc = HLC::new(1705843009213693952, 42);

    let enc1 = hlc.encode_descending();
    let enc2 = hlc.encode_descending();

    assert_eq!(enc1, enc2, "Encoding should be deterministic");
}
