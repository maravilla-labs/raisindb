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

//! Application of replicated workspace records.
//!
//! Split out of `workspace_branch_ops` because, unlike branches and tags, a
//! workspace update has to resolve a conflict before it writes — see
//! `workspace_lww`.

use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_models::workspace::Workspace;
use raisin_replication::Operation;
use rocksdb::ColumnFamily;

use super::super::db_helpers::{delete_key, serialize_and_write};
use super::workspace_lww::{effective_mtime, workspace_lww_decision, WorkspaceLww};
use super::OperationApplicator;

impl OperationApplicator {
    /// Read the workspace record currently stored under `key`, if any.
    ///
    /// A corrupt or unreadable value is reported as "nothing stored" rather than
    /// as an error: the incoming record is then applied, which repairs the key.
    /// Failing the operation instead would leave the bad value in place forever
    /// and stall the peer's replication stream on a permanent error.
    fn read_stored_workspace(&self, cf: &ColumnFamily, key: &[u8]) -> Option<Workspace> {
        match self.db.get_cf(cf, key) {
            Ok(Some(bytes)) => match rmp_serde::from_slice::<Workspace>(&bytes) {
                Ok(ws) => Some(ws),
                Err(e) => {
                    tracing::warn!(
                        "Stored workspace record is undeserializable ({}); treating the \
                         incoming replicated record as newer",
                        e
                    );
                    None
                }
            },
            Ok(None) => None,
            Err(e) => {
                tracing::warn!("Failed to read stored workspace for LWW comparison: {}", e);
                None
            }
        }
    }

    /// Apply a workspace update operation
    ///
    /// Last-write-wins by the record's own modification time: an out-of-order or
    /// replayed peer message carrying an OLDER workspace is dropped rather than
    /// reverting a newer config. See `workspace_lww` for why the comparator is
    /// shaped this way and what it does not cover.
    pub(in crate::replication::application) async fn apply_update_workspace(
        &self,
        tenant_id: &str,
        repo_id: &str,
        workspace_id: &str,
        workspace: &Workspace,
        op: &Operation,
    ) -> Result<()> {
        tracing::info!(
            "📥 Applying workspace update: {}/{}/{} from node {}",
            tenant_id,
            repo_id,
            workspace_id,
            op.cluster_node_id
        );

        let key = keys::workspace_key(tenant_id, repo_id, workspace_id);
        let cf = cf_handle(&self.db, cf::WORKSPACES)?;

        let stored = self.read_stored_workspace(cf, &key);
        if workspace_lww_decision(workspace, stored.as_ref()) == WorkspaceLww::RejectOlder {
            tracing::warn!(
                "⏪ Ignoring older UpdateWorkspace: incoming mtime {:?} < stored {:?} ({}/{}/{})",
                effective_mtime(workspace),
                stored.as_ref().map(effective_mtime),
                tenant_id,
                repo_id,
                workspace_id
            );
            return Ok(());
        }

        // NAMED serialization, matching `WorkspaceRepositoryImpl::put`. This is not
        // a style choice: `InitialNodeStructure` / `InitialChild` have hand-written
        // deserializers that only accept named fields, so a workspace written with
        // the compact (array-encoded) helper comes back as
        // "InitialChild cannot be null" on the very next read — and because
        // `WorkspaceRepository::list` deserializes every workspace in the repo, ONE
        // such record makes every SQL statement in that repo fail. The bug was
        // latent only for as long as nothing produced an UpdateWorkspace.
        serialize_and_write(
            &self.db,
            cf,
            key,
            workspace,
            &format!(
                "apply_update_workspace_{}/{}/{}",
                tenant_id, repo_id, workspace_id
            ),
        )?;

        // Peers must see a workspace event, or everything that keys off one (WS
        // subscriptions, config caches, policy reconciliation) stays blind to a
        // replicated config change.
        //
        // ALWAYS `Updated`, never `Created`, even for a workspace this node has
        // never seen: `WorkspaceStructureInitHandler` reacts to `Created` by
        // materialising the workspace's `initial_structure` nodes, and those nodes
        // are themselves replicating from the origin node. Emitting `Created` here
        // would have every receiving node build its own copy of the same root
        // folders under fresh ids — duplicates at identical paths.
        self.event_bus.publish(raisin_events::Event::Workspace(
            raisin_events::WorkspaceEvent {
                tenant_id: tenant_id.to_string(),
                repository_id: repo_id.to_string(),
                workspace: workspace_id.to_string(),
                kind: raisin_events::WorkspaceEventKind::Updated,
                metadata: None,
            },
        ));

        tracing::info!(
            "✅ Workspace applied successfully: {}/{}/{}",
            tenant_id,
            repo_id,
            workspace_id
        );
        Ok(())
    }

    /// Apply a workspace delete operation
    pub(in crate::replication::application) async fn apply_delete_workspace(
        &self,
        tenant_id: &str,
        repo_id: &str,
        workspace_id: &str,
        op: &Operation,
    ) -> Result<()> {
        tracing::info!(
            "📥 Applying workspace delete: {}/{}/{} from node {}",
            tenant_id,
            repo_id,
            workspace_id,
            op.cluster_node_id
        );

        let key = keys::workspace_key(tenant_id, repo_id, workspace_id);
        let cf = cf_handle(&self.db, cf::WORKSPACES)?;

        delete_key(
            &self.db,
            cf,
            key,
            &format!(
                "apply_delete_workspace_{}/{}/{}",
                tenant_id, repo_id, workspace_id
            ),
        )?;

        tracing::info!(
            "✅ Workspace deleted successfully: {}/{}/{}",
            tenant_id,
            repo_id,
            workspace_id
        );
        Ok(())
    }
}
