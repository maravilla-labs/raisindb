//! Persistence for compound-index build state.
//!
//! Mirrors `crate::spatial_state` — same column family, same msgpack encoding,
//! same fail-closed contract. Kept as its own module rather than folded in
//! because the two answer different questions with different keys, and the
//! spatial one is already the larger of the two.

mod fork;
mod marker;
#[cfg(test)]
mod marker_tests;
mod store;

pub use store::{compound_state_key, read_state, CompoundStateStore};

/// Env flag letting the sweeps (boot, schema events, cold-request drains)
/// rebuild an index whose state record is an older FORMAT
/// (`CompoundIndexState::is_format_upgrade`). Default OFF: such an index is
/// served by a scan until an admin runs `REBUILD … compound` — per node, at a
/// time of the operator's choosing — so a format bump never rebuilds every
/// index on every node at boot (repair discipline: admin-triggered, disk
/// precheck). `1`/`true`/`on`/`yes` turn it on, e.g. one node at a time.
pub const FORMAT_REBUILD_ENV: &str = "RAISIN_COMPOUND_FORMAT_REBUILD";

/// Whether [`FORMAT_REBUILD_ENV`] is on.
pub fn format_rebuild_enabled() -> bool {
    std::env::var(FORMAT_REBUILD_ENV).is_ok_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "on" | "yes"
        )
    })
}
