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
pub use items::{
    child_external_id, default_mapping, Change, ChangesPage, ExternalItem, ListPage, MappedChild,
    MappedItem, MappedNode, CHILD_ID_SEP,
};
pub use mount::{MountConfig, SyncConfig, WriteConfig};
pub use paths::{parse_iso_epoch, passes_filters, resolve_path_template};
pub use props::build_properties;
pub(crate) use props::{parse_object, parse_object_checked};
pub use run::SyncRun;
pub use state::{
    DrainSummary, MountState, PendingDelete, ReconcileSummary, WritebackBlock,
    BULK_REVISION_THRESHOLD, MAX_FAILED_ITEMS, MAX_PENDING_DELETES, MAX_RUN_HISTORY,
};

/// The workspace that holds integration/mount configuration in every repo.
pub const SYSTEM_WORKSPACE: &str = "raisin:system";

/// Actor stamped on every sync-materialized write. Downstream writeback
/// triggers filter on this to avoid feedback loops.
///
/// It is stamped in two independent places, and both are needed: as the
/// transaction's raw actor (which becomes `RevisionMeta.actor`) and as the
/// identity of the transaction's auth context via
/// `AuthContext::system_as(SYNC_ACTOR)` (which becomes the node's
/// `created_by` / `updated_by`, and which takes precedence over the raw actor
/// in `put_node` / `add_node`). Setting only the former left every synced node
/// attributed to `"system"`.
///
/// Re-exported from raisin-models rather than defined here: the raisin-core
/// write paths shield engine-owned `__` properties from every OTHER actor, so
/// the string they compare against and the string the engine writes as must be
/// one definition or the shield would lock the engine out of its own metadata.
pub use raisin_models::nodes::SYNC_ACTOR;

/// Default number of consecutive failures before a mount is marked degraded.
pub const DEGRADE_THRESHOLD: u64 = 5;

/// Default per-sync item cap when a mount does not specify one.
pub const DEFAULT_MAX_ITEMS: u64 = 500;

/// Default poll interval (seconds) when a mount omits `interval_seconds`.
pub const DEFAULT_INTERVAL_SECS: u64 = 300;

/// Default number of items written per transaction.
pub const DEFAULT_BATCH_SIZE: usize = 1000;

/// Default number of item rejections tolerated before a run gives up.
///
/// Only trips while NOTHING has been written (see `SyncBatcher::flush`), so a
/// handful of bad items in an otherwise healthy mount never reaches it.
pub const DEFAULT_MAX_ITEM_FAILURES: usize = 50;

/// Default byte budget for one batch.
///
/// A batch commits as ONE revision, which the replication layer captures as a
/// single un-decomposed `ApplyRevision` holding a full snapshot of every node in
/// it. No catch-up/replay path decomposes a stored operation, so a record larger
/// than the 10 MB transport frame cap would permanently wedge a peer's sync.
/// 1000 mail items with 20 KB bodies would be ~20 MB — this budget is what makes
/// a large `batch_size` safe, so raise the two together or not at all.
pub const DEFAULT_BATCH_MAX_BYTES: usize = 4 * 1024 * 1024;
