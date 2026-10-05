//! Persistence for compound-index build state.
//!
//! Mirrors `crate::spatial_state` — same column family, same msgpack encoding,
//! same fail-closed contract. Kept as its own module rather than folded in
//! because the two answer different questions with different keys, and the
//! spatial one is already the larger of the two.

mod build_cas;
mod fork;
mod marker;
#[cfg(test)]
mod marker_tests;
mod store;
mod workspace_reconcile;

pub use fork::ForkInheritance;
pub use marker::StaleScope;
pub use store::{compound_state_key, read_state, CompoundStateStore};
pub use workspace_reconcile::{
    forget_undeclared_workspace_index, reconcile_workspace_declarations,
};

/// Env switch for the AUTOMATIC rebuild of an index whose state record is an
/// older FORMAT (`CompoundIndexState::is_format_upgrade`) — plan Phase 13f,
/// owner decision of 2026-10-05. Default ON: after an upgrade such indexes are
/// rebuilt in the background by the `compound_builds` repair chain (one
/// branch at a time, disk precheck, paced, per-node state, re-queued after a
/// checkpoint ingest), and their queries scan until then. `0`/`false`/`off`/
/// `no` turns it off: such an index is then left to an admin
/// `REBUILD … compound` (or `reindex/start` with `index_types: ["compound"]`),
/// as before Phase 13f.
pub const FORMAT_REBUILD_ENV: &str = "RAISIN_COMPOUND_FORMAT_REBUILD";

/// TEST override of [`format_rebuild_enabled`] (0: none, 1: on, 2: off) —
/// the environment is process-wide and shared by parallel tests.
static FORMAT_REBUILD_OVERRIDE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// TEST hook: force [`format_rebuild_enabled`] (`None`: back to the
/// environment). Process-wide.
#[doc(hidden)]
pub fn override_format_rebuild(enabled: Option<bool>) {
    FORMAT_REBUILD_OVERRIDE.store(
        match enabled {
            None => 0,
            Some(true) => 1,
            Some(false) => 2,
        },
        std::sync::atomic::Ordering::SeqCst,
    );
}

/// Whether [`FORMAT_REBUILD_ENV`] is on (unset: on).
pub fn format_rebuild_enabled() -> bool {
    match FORMAT_REBUILD_OVERRIDE.load(std::sync::atomic::Ordering::SeqCst) {
        1 => return true,
        2 => return false,
        _ => {}
    }
    std::env::var(FORMAT_REBUILD_ENV)
        .map(|v| {
            !matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "off" | "no"
            )
        })
        .unwrap_or(true)
}
