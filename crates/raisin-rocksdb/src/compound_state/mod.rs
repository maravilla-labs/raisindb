//! Persistence for compound-index build state.
//!
//! Mirrors `crate::spatial_state` — same column family, same msgpack encoding,
//! same fail-closed contract. Kept as its own module rather than folded in
//! because the two answer different questions with different keys, and the
//! spatial one is already the larger of the two.

mod store;

pub use store::{compound_state_key, read_state, CompoundStateStore};
