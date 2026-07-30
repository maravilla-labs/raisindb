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

//! Last-write-wins guard for replicated workspace records.
//!
//! # Why this exists
//!
//! `apply_update_workspace` used to be a blind overwrite. That was harmless only
//! for as long as nothing ever *produced* an `OpType::UpdateWorkspace` — the
//! applier existed with no producer anywhere in the tree. The moment
//! `WorkspaceRepositoryImpl::put` started capturing the operation, a blind
//! overwrite became actively dangerous: peer messages arrive out of order (retry,
//! catch-up replay, a reconnecting node draining its backlog), so an *older*
//! workspace record would routinely land on top of a newer one and silently
//! revert a config change — including the spatial precision policy, whose whole
//! cluster fan-out mechanism is this operation. Reverting a config is strictly
//! worse than never replicating it, because the local write appears to succeed.
//!
//! # The comparator
//!
//! The comparator is derived from the record itself, exactly like
//! `apply_update_branch` compares the incoming `head` against the stored one and
//! ignores anything older. A workspace is not MVCC-versioned — one key, one live
//! value — so there is no stored revision to compare against; what the record
//! does carry is `updated_at` (falling back to `created_at` for records written
//! before anyone stamped an update). `WorkspaceRepositoryImpl::put` stamps
//! `updated_at` on every update so that comparator is always populated and always
//! moves forward on the writing node.
//!
//! Ties (identical effective mtime) apply, matching `apply_update_branch`'s
//! `incoming < current` test. A tie needs two writes in the same nanosecond on
//! two different nodes; the cost of getting it wrong is one config write ordered
//! by arrival instead of by clock, and the alternative — rejecting on tie — is
//! not convergent either, since each node would then keep its own version.
//!
//! # Known limitation
//!
//! This is wall-clock last-write-wins, so it inherits wall-clock skew: a node
//! whose clock runs ahead can win a conflict it should have lost. Workspace
//! records are not MVCC-versioned and carry no HLC to compare instead; giving
//! them one is a stored-format change, and it would only narrow the window
//! rather than close it, since an HLC's physical component is the same wall
//! clock. Closing it properly means ordering workspace writes by the same
//! revision machinery nodes use — worth doing before workspace config becomes
//! load-bearing for anything beyond index policy.

use raisin_models::timestamp::StorageTimestamp;
use raisin_models::workspace::Workspace;

/// Outcome of comparing an incoming replicated workspace against the stored one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::replication::application) enum WorkspaceLww {
    /// The incoming record is at least as new as the stored one — write it.
    Apply,
    /// The incoming record is strictly older than the stored one — drop it.
    RejectOlder,
}

/// The effective modification time of a workspace record.
///
/// `updated_at` is `None` for a workspace that has never been updated since it
/// was created, and for records written before updates were stamped at all, so
/// `created_at` is the fallback rather than an error.
pub(in crate::replication::application) fn effective_mtime(ws: &Workspace) -> StorageTimestamp {
    ws.updated_at.unwrap_or(ws.created_at)
}

/// Decide whether a replicated workspace record may overwrite the stored one.
///
/// `stored` is `None` when the workspace does not exist locally yet, which always
/// applies — a first sight of a workspace is never a conflict.
pub(in crate::replication::application) fn workspace_lww_decision(
    incoming: &Workspace,
    stored: Option<&Workspace>,
) -> WorkspaceLww {
    let Some(stored) = stored else {
        return WorkspaceLww::Apply;
    };

    if effective_mtime(incoming) < effective_mtime(stored) {
        WorkspaceLww::RejectOlder
    } else {
        WorkspaceLww::Apply
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use raisin_models::workspace::WorkspaceConfig;

    fn ts(secs: i64) -> StorageTimestamp {
        Utc.timestamp_opt(secs, 0).unwrap().into()
    }

    /// A workspace whose `created_at` is `created` and whose `updated_at` is
    /// `updated`, carrying `marker` in its description so the two sides of a
    /// conflict are distinguishable.
    fn ws(marker: &str, created: i64, updated: Option<i64>) -> Workspace {
        Workspace {
            name: "places".to_string(),
            description: Some(marker.to_string()),
            allowed_node_types: vec![],
            allowed_root_node_types: vec![],
            depends_on: vec![],
            initial_structure: None,
            created_at: ts(created),
            updated_at: updated.map(ts),
            config: WorkspaceConfig::default(),
        }
    }

    #[test]
    fn first_sight_of_a_workspace_applies() {
        assert_eq!(
            workspace_lww_decision(&ws("incoming", 100, None), None),
            WorkspaceLww::Apply
        );
    }

    /// THE regression this guard exists for: an out-of-order peer message
    /// carrying an older workspace must not revert the newer stored config.
    #[test]
    fn older_incoming_must_not_clobber_newer_stored() {
        let stored = ws("newer", 100, Some(500));
        let incoming = ws("older", 100, Some(300));
        assert_eq!(
            workspace_lww_decision(&incoming, Some(&stored)),
            WorkspaceLww::RejectOlder
        );
    }

    #[test]
    fn newer_incoming_wins() {
        let stored = ws("older", 100, Some(300));
        let incoming = ws("newer", 100, Some(500));
        assert_eq!(
            workspace_lww_decision(&incoming, Some(&stored)),
            WorkspaceLww::Apply
        );
    }

    /// Reapplying the identical operation (redelivery) must stay a no-op rather
    /// than being rejected as "not newer" — apply is idempotent here.
    #[test]
    fn identical_mtime_applies_idempotently() {
        let stored = ws("same", 100, Some(500));
        let incoming = ws("same", 100, Some(500));
        assert_eq!(
            workspace_lww_decision(&incoming, Some(&stored)),
            WorkspaceLww::Apply
        );
    }

    /// A record that has never been updated compares by `created_at`, so a
    /// freshly created workspace does not lose to a never-updated older one.
    #[test]
    fn falls_back_to_created_at_when_never_updated() {
        let stored = ws("stored", 500, None);
        let older = ws("older", 100, None);
        let newer = ws("newer", 900, None);

        assert_eq!(
            workspace_lww_decision(&older, Some(&stored)),
            WorkspaceLww::RejectOlder
        );
        assert_eq!(
            workspace_lww_decision(&newer, Some(&stored)),
            WorkspaceLww::Apply
        );
    }

    /// An updated incoming record beats a stored record that was only ever
    /// created — the mixed case, where the two sides read different fields.
    #[test]
    fn updated_incoming_beats_created_only_stored() {
        let stored = ws("created-only", 400, None);
        let incoming = ws("updated", 100, Some(600));
        assert_eq!(
            workspace_lww_decision(&incoming, Some(&stored)),
            WorkspaceLww::Apply
        );
    }

    /// ...and the reverse: a stored record updated *after* the incoming one was
    /// created must survive.
    #[test]
    fn created_only_incoming_loses_to_updated_stored() {
        let stored = ws("updated", 100, Some(600));
        let incoming = ws("created-only", 400, None);
        assert_eq!(
            workspace_lww_decision(&incoming, Some(&stored)),
            WorkspaceLww::RejectOlder
        );
    }
}
