//! Local, per-instance record of what the spatial index bytes on THIS disk
//! contain — as distinct from the replicated *configured* policy.
//!
//! # Why the split matters
//!
//! `configured_precisions` is user intent and lives in replicated records (the
//! NodeType field schema and `WorkspaceConfig.spatial`). `indexed_precisions` is
//! local reality and lives here, in the `INDEX_STATUS` column family beside the
//! existing `prop_index` watermark.
//!
//! - **Queries read this** (reality), so changing the configuration can never
//!   make already-indexed data invisible.
//! - **Writes reconcile towards the configuration**, indexing at
//!   `configured ∪ indexed` while the two differ so a row written mid-migration
//!   is findable under both sets.
//! - **A missing record means `NotBuilt`**, which forces the planner onto a scan
//!   with the spatial predicate retained. Slow and correct, never fast and wrong.
//!
//! The record is also the local cache of the *resolved* policy, which is what lets
//! the synchronous write paths (replication apply, repository `add`) agree with
//! the asynchronous transaction path on the cell set without loading a NodeType
//! inside a `WriteBatch`.

pub mod admin;
pub mod configured;
pub mod resolve;
mod store;

pub use admin::{BuildEnqueuer, SpatialAdminStore};
pub use configured::{configured_policy, workspace_spatial_schema};
pub use resolve::{
    availability_in_rebuild_window, resolve_write_policy, union_policy, BuildTrigger,
    WritePolicyResolution,
};
pub use store::{policy_for_property, read_state, spatial_state_key, SpatialStateStore};
