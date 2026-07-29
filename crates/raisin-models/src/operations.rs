// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Types of operations that can be performed on nodes
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OperationType {
    /// Node was moved to a new parent or path
    Move {
        /// Original path before move
        from_path: String,
        /// Original parent ID before move
        from_parent_id: String,
        /// New path after move
        to_path: String,
        /// New parent ID after move
        to_parent_id: String,
    },
    /// Node was copied from another node
    Copy {
        /// Source node ID that was copied
        source_id: String,
        /// Source node path at time of copy
        source_path: String,
        /// Destination path where copy was created
        destination_path: String,
    },
    /// Node was renamed
    Rename {
        /// Original name before rename
        old_name: String,
        /// New name after rename
        new_name: String,
    },
    /// Node was reordered within its parent
    Reorder {
        /// Original fractional index
        old_index: String,
        /// New fractional index
        new_index: String,
    },
}

/// Metadata about a node operation performed in a revision
///
/// This structure captures the details of what operation was performed
/// on a node, similar to how TranslationMeta captures translation operations.
/// This enables time-travel queries and audit trails.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationMeta {
    /// The type of operation and its specific details
    pub operation: OperationType,

    /// Revision number where this operation occurred
    pub revision: raisin_hlc::HLC,

    /// Parent revision (for tracking operation history)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_revision: Option<raisin_hlc::HLC>,

    /// When the operation was performed
    pub timestamp: DateTime<Utc>,

    /// Who performed the operation (user ID, system, etc.)
    pub actor: String,

    /// Descriptive message about why the operation was performed
    pub message: String,

    /// Whether this was a system-initiated operation
    #[serde(default)]
    pub is_system: bool,

    /// Node ID that this operation was performed on
    pub node_id: String,

    /// The non-human principal that *initiated* the operation, in the
    /// [`agent_identity`](crate::auth::agent_identity) vocabulary
    /// (`trigger:/triggers/x`, `flow:/flows/y@trigger:/triggers/x`, ...).
    ///
    /// `None` for a direct human write. Carried alongside `actor` -- never
    /// instead of it -- so a replicated operation can record the same
    /// actor+agent pair the transaction path resolves from its `AuthContext`.
    ///
    /// MUST stay LAST, `#[serde(default)]`, and WITHOUT
    /// `skip_serializing_if`. This struct is persisted with
    /// `rmp_serde::to_vec` (see
    /// `raisin-rocksdb/src/repositories/revisions/trait_impl.rs`), which encodes
    /// a struct as a POSITIONAL array: skipping a field shortens the array and
    /// every later field is then read at the wrong index (`is_system` lands in
    /// `message` -> "invalid type: boolean `false`, expected a string").
    /// Trailing + `default` is what lets an 8-element blob written before this
    /// field existed still deserialize.
    #[serde(default)]
    pub agent: Option<String>,
}

impl OperationMeta {
    /// Attach the non-human initiator that caused this operation.
    ///
    /// Provenance only -- it records *what* drove the write, and never changes
    /// who the write was authorized as (`actor` is left untouched).
    pub fn with_agent(mut self, agent: Option<String>) -> Self {
        self.agent = agent.filter(|a| !a.trim().is_empty());
        self
    }

    /// Create a new OperationMeta for a move operation
    #[allow(clippy::too_many_arguments)]
    pub fn new_move(
        node_id: String,
        from_path: String,
        from_parent_id: String,
        to_path: String,
        to_parent_id: String,
        revision: &raisin_hlc::HLC,
        parent_revision: Option<&raisin_hlc::HLC>,
        actor: String,
        message: String,
    ) -> Self {
        Self {
            operation: OperationType::Move {
                from_path,
                from_parent_id,
                to_path,
                to_parent_id,
            },
            revision: *revision,
            parent_revision: parent_revision.copied(),
            timestamp: Utc::now(),
            actor,
            message,
            is_system: false,
            node_id,
            agent: None,
        }
    }

    /// Create a new OperationMeta for a copy operation
    #[allow(clippy::too_many_arguments)]
    pub fn new_copy(
        node_id: String,
        source_id: String,
        source_path: String,
        destination_path: String,
        revision: &raisin_hlc::HLC,
        parent_revision: Option<&raisin_hlc::HLC>,
        actor: String,
        message: String,
    ) -> Self {
        Self {
            operation: OperationType::Copy {
                source_id,
                source_path,
                destination_path,
            },
            revision: *revision,
            parent_revision: parent_revision.copied(),
            timestamp: Utc::now(),
            actor,
            message,
            is_system: false,
            node_id,
            agent: None,
        }
    }

    /// Create a new OperationMeta for a rename operation
    pub fn new_rename(
        node_id: String,
        old_name: String,
        new_name: String,
        revision: &raisin_hlc::HLC,
        parent_revision: Option<&raisin_hlc::HLC>,
        actor: String,
        message: String,
    ) -> Self {
        Self {
            operation: OperationType::Rename { old_name, new_name },
            revision: *revision,
            parent_revision: parent_revision.copied(),
            timestamp: Utc::now(),
            actor,
            message,
            is_system: false,
            node_id,
            agent: None,
        }
    }

    /// Create a new OperationMeta for a reorder operation
    pub fn new_reorder(
        node_id: String,
        old_index: String,
        new_index: String,
        revision: &raisin_hlc::HLC,
        parent_revision: Option<&raisin_hlc::HLC>,
        actor: String,
        message: String,
    ) -> Self {
        Self {
            operation: OperationType::Reorder {
                old_index,
                new_index,
            },
            revision: *revision,
            parent_revision: parent_revision.copied(),
            timestamp: Utc::now(),
            actor,
            message,
            is_system: false,
            node_id,
            agent: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// NOTE the `Some(parent)`: `parent_revision` carries a PRE-EXISTING
    /// `skip_serializing_if = "Option::is_none"` that has the very same
    /// positional hazard documented on `agent`, so an `OperationMeta` with
    /// `parent_revision: None` does not survive a MessagePack roundtrip even at
    /// clean HEAD (it fails with "Failed to parse HLC from '<timestamp>'").
    /// That bug is untouched here -- fixing it changes the on-disk encoding of
    /// every already-persisted blob and needs a migration. Every real write path
    /// passes `Some(...)` (see `ordering/reorder.rs`, `queries/tree_ops`), which
    /// is why it has never surfaced.
    fn sample() -> OperationMeta {
        OperationMeta::new_reorder(
            "node-1".to_string(),
            "a".to_string(),
            "b".to_string(),
            &raisin_hlc::HLC::new(2, 0),
            Some(&raisin_hlc::HLC::new(1, 0)),
            "alice".to_string(),
            "reorder".to_string(),
        )
    }

    /// `OperationMeta` is persisted with `rmp_serde::to_vec`, which encodes a
    /// struct as a POSITIONAL array. A field that is skipped when empty (or
    /// inserted anywhere but the end) shifts every later field, and the type
    /// error surfaces far away -- `is_system: false` read as `message: String`.
    ///
    /// This is not hypothetical: adding `agent` mid-struct with
    /// `skip_serializing_if` broke all five reorder/move revision-metadata
    /// tests with exactly that error.
    #[test]
    fn messagepack_roundtrips_with_and_without_an_agent() {
        for meta in [
            sample(),
            sample().with_agent(Some("trigger:/triggers/t".to_string())),
        ] {
            let bytes = rmp_serde::to_vec(&meta).expect("serializes");
            let back: OperationMeta = rmp_serde::from_slice(&bytes).expect("deserializes");
            assert_eq!(back, meta);
        }
    }

    /// The field count must not change with `agent`, or the encoding is
    /// positional-unstable again.
    #[test]
    fn the_encoded_field_count_does_not_depend_on_the_agent() {
        let without = rmp_serde::to_vec(&sample()).unwrap();
        let with = rmp_serde::to_vec(&sample().with_agent(Some("trigger:/t".to_string()))).unwrap();
        // First byte of a fixarray encodes its length; both must declare the same.
        assert_eq!(
            without[0], with[0],
            "an absent agent must still occupy its slot"
        );
    }

    /// A blob written before `agent` existed (one element short) must still
    /// load -- which only holds while `agent` is LAST and `#[serde(default)]`.
    #[test]
    fn a_blob_written_before_the_agent_field_still_deserializes() {
        #[derive(serde::Serialize)]
        struct LegacyOperationMeta {
            operation: OperationType,
            revision: raisin_hlc::HLC,
            parent_revision: Option<raisin_hlc::HLC>,
            timestamp: DateTime<Utc>,
            actor: String,
            message: String,
            is_system: bool,
            node_id: String,
        }

        let legacy = LegacyOperationMeta {
            operation: OperationType::Reorder {
                old_index: "a".to_string(),
                new_index: "b".to_string(),
            },
            revision: raisin_hlc::HLC::new(1, 0),
            parent_revision: Some(raisin_hlc::HLC::new(1, 0)),
            timestamp: Utc::now(),
            actor: "alice".to_string(),
            message: "reorder".to_string(),
            is_system: false,
            node_id: "node-1".to_string(),
        };

        let bytes = rmp_serde::to_vec(&legacy).expect("serializes");
        let back: OperationMeta = rmp_serde::from_slice(&bytes).expect("legacy blob must load");
        assert_eq!(back.actor, "alice");
        assert_eq!(back.message, "reorder");
        assert_eq!(back.node_id, "node-1");
        assert_eq!(back.agent, None);
    }

    /// `with_agent` is provenance only: it must never disturb the actor, and a
    /// blank marker must be treated as absent rather than stored as "".
    #[test]
    fn with_agent_leaves_the_actor_alone_and_rejects_blanks() {
        let meta = sample().with_agent(Some("   ".to_string()));
        assert_eq!(meta.agent, None);
        assert_eq!(meta.actor, "alice");

        let meta = sample().with_agent(Some("flow:/flows/x".to_string()));
        assert_eq!(meta.agent.as_deref(), Some("flow:/flows/x"));
        assert_eq!(
            meta.actor, "alice",
            "attribution must not rewrite the actor"
        );
    }
}
