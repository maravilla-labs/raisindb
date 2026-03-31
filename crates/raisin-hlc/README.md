# raisin-hlc

High-performance Hybrid Logical Clock (HLC) implementation for RaisinDB distributed timestamps.

## Overview

This crate provides a lock-free implementation of Hybrid Logical Clocks for distributed timestamp generation in RaisinDB. HLC combines physical wall-clock time with logical counters to provide causally-consistent timestamps across distributed nodes without requiring clock synchronization.

## Features

- **Lock-free timestamp generation**: Uses atomic CAS operations, no mutexes
- **High performance**: <30ns for tick(), <30ns for update()
- **Descending lexicographic encoding**: Optimized for RocksDB range scans (newest first)
- **Full ordering semantics**: Implements `Ord`, `PartialOrd`, `Eq`, `PartialEq`
- **String serialization**: Human-readable format for APIs (`"timestamp-counter"`)
- **Binary serialization**: Efficient 16-byte encoding for storage
- **Thread-safe**: All operations are safe for concurrent use

## Performance

Measured on Apple Silicon M-series (your results may vary):

| Operation | Time | Notes |
|-----------|------|-------|
| `tick()` | ~23ns | Single-threaded timestamp generation |
| `update()` | ~27ns | Update from remote timestamp |
| `encode_descending()` | ~2ns | Binary encoding |
| `decode_descending()` | ~1ns | Binary decoding |
| `to_string()` | ~72ns | String serialization |
| `from_str()` | ~37ns | String parsing |
| Comparison | <1ns | Equality and ordering |

Compared to baseline atomic u64 increment (~1.8ns), HLC tick is ~15x slower but provides distributed causality guarantees.

## Usage

### Basic Timestamp Generation

```rust
use raisin_hlc::{HLC, NodeHLCState};

// Create HLC state for a node
let state = NodeHLCState::new("node-1".to_string());

// Generate monotonic timestamps
let hlc1 = state.tick();
let hlc2 = state.tick();
assert!(hlc2 > hlc1);
```

### Replication Scenario

```rust
use raisin_hlc::{HLC, NodeHLCState};

let node1 = NodeHLCState::new("node-1".to_string());
let node2 = NodeHLCState::new("node-2".to_string());

// Node 1 generates an operation
let op_hlc = node1.tick();

// Node 2 receives and processes the operation
let updated_hlc = node2.update(&op_hlc);

// Node 2's clock now reflects causal ordering
assert!(updated_hlc >= op_hlc);

// Subsequent operations on Node 2 maintain causality
let next_hlc = node2.tick();
assert!(next_hlc > updated_hlc);
```

### Encoding for RocksDB

```rust
use raisin_hlc::HLC;

let hlc = HLC::new(1705843009213693952, 42);

// Encode for storage (descending order - newest first)
let key_suffix = hlc.encode_descending();

// Decode from storage
let decoded = HLC::decode_descending(&key_suffix).unwrap();
assert_eq!(hlc, decoded);
```

### String Serialization

```rust
use raisin_hlc::HLC;

let hlc = HLC::new(1705843009213693952, 42);

// Convert to string for APIs
let string = hlc.to_string(); // "1705843009213693952-42"

// Parse from string
let parsed: HLC = string.parse().unwrap();
assert_eq!(hlc, parsed);
```

## Architecture

### HLC Structure

```rust
pub struct HLC {
    pub timestamp_ms: u64,  // Physical wall clock time (milliseconds since UNIX epoch)
    pub counter: u64,       // Logical counter for same-millisecond events
}
```

### Ordering Rules

HLCs are ordered lexicographically:
1. First by `timestamp_ms` (ascending)
2. Then by `counter` (ascending)

This ensures a total order across all nodes.

### Lock-free State

```rust
pub struct NodeHLCState {
    last_timestamp: AtomicU64,
    last_counter: AtomicU64,
    node_id: String,
}
```

All operations use atomic compare-and-swap (CAS) loops, ensuring thread safety without locks.

### Tick Algorithm

```
1. Read current wall clock time
2. Compare with last HLC:
   - If wall_clock > last_timestamp: use wall_clock, reset counter to 0
   - If wall_clock = last_timestamp: keep timestamp, increment counter
   - If wall_clock < last_timestamp: keep last_timestamp, increment counter
3. Update atomically using CAS
4. Retry on contention
```

### Update Algorithm (Replication)

```
1. Read wall clock, local HLC, and remote HLC
2. Compute new timestamp: max(wall_clock, local_timestamp, remote_timestamp)
3. Determine counter:
   - If new_timestamp == remote_timestamp: use remote.counter + 1
   - Else if new_timestamp == local_timestamp: use local.counter + 1
   - Otherwise: reset to 0
4. Update atomically using CAS
5. Retry on contention
```

## Encoding Format

### Binary Encoding (16 bytes)

```
[0..8]  NOT(timestamp_ms) in big-endian
[8..16] NOT(counter) in big-endian
```

Bitwise NOT is applied to achieve descending lexicographic order - newer timestamps have smaller byte values and sort first in RocksDB range scans.

### String Format

```
{timestamp_ms}-{counter}
```

Example: `1705843009213693952-42`

## Clock Skew Detection

The implementation monitors for clock skew:

- If remote timestamp is >5s ahead of wall clock, a warning is logged
- If wall clock jumps >5s forward, `validate()` returns error

This helps detect system time changes and network issues.

## Testing

Run tests:
```bash
cargo test --package raisin-hlc
```

Run benchmarks:
```bash
cargo bench --package raisin-hlc
```

## Design Decisions

### Why u64 instead of i64?

- Simpler encoding (no sign bit handling)
- Sufficient range (~584 million years from 1970)
- Matches RocksDB's natural u64 handling

### Why descending encoding?

RaisinDB queries typically want newest data first. Descending encoding allows efficient reverse iteration:
- Seek to prefix
- Iterate forward to get newest-to-oldest order
- No need for expensive reverse scans

### Why NOT separate persistence?

HLC state is NOT persisted by default in this crate. Persistence is handled at a higher level (in `raisin-rocksdb`) because:
- Avoids coupling to specific storage backends
- Allows batching of persistence with other state
- Keeps this crate focused and dependency-light

Applications should persist HLC state periodically (e.g., every 10s or 1000 ops).

## Safety Guarantees

1. **Monotonicity**: Each `tick()` returns strictly greater HLC than previous
2. **Causality**: After `update(remote)`, all subsequent ticks > remote
3. **Thread Safety**: All operations are lock-free and thread-safe
4. **Overflow Safety**: Counter overflow behavior is defined (wraps)

## Limitations

- **Clock Skew**: Assumes reasonable clock synchronization (<5s drift)
- **Counter Overflow**: If counter hits u64::MAX, it wraps (extremely unlikely)
- **No Built-in Persistence**: Applications must handle persistence
- **Single-Node State**: Each node maintains independent HLC state

## Integration with RaisinDB

This HLC implementation replaces centralized u64 revision counters:

```rust
// Old approach (centralized counter)
let revision = revision_repo.allocate_revision(); // u64

// New approach (distributed HLC)
let hlc = hlc_state.tick(); // HLC
```

HLC serves dual purpose:
1. MVCC revision for versioned key encoding
2. Replication timestamp for operation ordering

This unification simplifies the architecture and enables masterless replication.

## References

- [Hybrid Logical Clocks (HLC) Paper](https://cse.buffalo.edu/tech-reports/2014-04.pdf)
- [Logical Physical Clocks](https://martinfowler.com/articles/patterns-of-distributed-systems/hybrid-clock.html)

## License

BSL-1.1 - See [LICENSE](../../LICENSE) for details.
