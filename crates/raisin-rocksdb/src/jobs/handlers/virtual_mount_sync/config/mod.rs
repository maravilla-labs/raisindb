//! Typed configuration for the virtual-mount sync engine.
//!
//! Loads and validates `raisin:VirtualMount` and `raisin:Integration` nodes into
//! strongly-typed structs, defines the normalized external-item / change shapes
//! adapters return, and provides the built-in Rust default mapping and the
//! include/exclude glob filter.

mod integration;
mod items;
mod mount;
mod paths;
mod props;
mod run;
mod state;

pub use integration::{ConnectedAccount, IntegrationConfig};
pub use items::{default_mapping, Change, ChangesPage, ExternalItem, ListPage, MappedNode};
pub use mount::{MountConfig, SyncConfig, WriteConfig};
pub use paths::{parse_iso_epoch, passes_filters, resolve_path_template};
pub use props::build_properties;
pub(crate) use props::{parse_object, parse_object_checked};
pub use run::SyncRun;
pub use state::{MountState, MAX_RUN_HISTORY};

/// The workspace that holds integration/mount configuration in every repo.
pub const SYSTEM_WORKSPACE: &str = "raisin:system";

/// Actor stamped on every sync-materialized write. Downstream writeback
/// triggers filter on this to avoid feedback loops.
pub const SYNC_ACTOR: &str = "virtual-mount-sync";

/// Default number of consecutive failures before a mount is marked degraded.
pub const DEGRADE_THRESHOLD: u64 = 5;

/// Default per-sync item cap when a mount does not specify one.
pub const DEFAULT_MAX_ITEMS: u64 = 500;

/// Default poll interval (seconds) when a mount omits `interval_seconds`.
pub const DEFAULT_INTERVAL_SECS: u64 = 300;

/// Default number of items written per transaction.
pub const DEFAULT_BATCH_SIZE: usize = 1000;

/// Default byte budget for one batch.
///
/// A batch commits as ONE revision, which the replication layer captures as a
/// single un-decomposed `ApplyRevision` holding a full snapshot of every node in
/// it. No catch-up/replay path decomposes a stored operation, so a record larger
/// than the 10 MB transport frame cap would permanently wedge a peer's sync.
/// 1000 mail items with 20 KB bodies would be ~20 MB — this budget is what makes
/// a large `batch_size` safe, so raise the two together or not at all.
pub const DEFAULT_BATCH_MAX_BYTES: usize = 4 * 1024 * 1024;
