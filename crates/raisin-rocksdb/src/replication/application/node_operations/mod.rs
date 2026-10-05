//! Shared helpers for the node operation handlers of replication.
//!
//! The handlers themselves are `OperationApplicator` methods in
//! `applicator/` (`crdt_ops`, `legacy_node_ops`, `move_node_ops`). This module
//! used to hold a second, never-dispatched copy of each — `create_node`,
//! `set_property`, `move_rename`, `delete_node`, `snapshot_ops` — which drifted
//! from the live ones (no NODE_PATH, the full `Node` blob) and was removed with
//! plan Phase 10b, when every live node record went through the one record
//! writer.

mod event_helpers;

pub(super) use event_helpers::{emit_node_event, EventAttribution};
