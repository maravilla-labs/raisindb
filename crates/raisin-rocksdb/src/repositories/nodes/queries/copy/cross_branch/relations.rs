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

//! Carry a promoted node's OUTGOING edges to the target branch.
//!
//! Edges are branch-scoped and live in their own keyspace (the packed
//! adjacency list in `cf::RELATION_INDEX`), not in the node blob — so a copy
//! that wrote the node and nothing else promoted a page whose links, tags and
//! authorship edges silently stayed behind. Every consumer then had to replay
//! them by hand (`RELATE IN BRANCH …` per edge), which can only ever ADD: an
//! edge removed on the source stayed live on the target forever.
//!
//! The target's list is REPLACED by the source's, at the copy's revision, so a
//! re-promotion converges on exactly the source's edges — removals included —
//! and MVCC readers below that revision still see the old list.

use super::CopyScope;
use crate::repositories::nodes::NodeRepositoryImpl;
use crate::repositories::relations::helpers::{
    get_relation_cf, read_packed_relations_at, serialize_compact_relations, CompactRelation,
};
use raisin_error::Result;
use raisin_models::nodes::RelationRef;
use raisin_replication::OpType;
use rocksdb::WriteBatch;

fn same_edge(a: &CompactRelation, b: &CompactRelation) -> bool {
    a.relation_type == b.relation_type
        && a.target_id == b.target_id
        && a.target_workspace == b.target_workspace
}

fn identical(a: &CompactRelation, b: &CompactRelation) -> bool {
    same_edge(a, b) && a.target_node_type == b.target_node_type && a.weight == b.weight
}

impl NodeRepositoryImpl {
    /// Stage the source node's packed adjacency list onto the target branch.
    ///
    /// Reads the source AS OF `scope.source_max_revision` (the pin, or the
    /// source head), so a pinned promotion carries the edges that existed at
    /// the pinned state, not today's.
    ///
    /// A source with NO packed record at all is left alone on the target:
    /// nodes whose edges were written as legacy per-edge keys (pre-packed
    /// creates) read as "no list" here, and clearing the target on that
    /// evidence would drop edges we cannot see. A source with a record —
    /// even an empty one — is authoritative.
    ///
    /// Replication: the node snapshot in the `ApplyRevision` op carries no
    /// edges (peers write per-edge keys from `node.relations`, which readers
    /// ignore once a packed list exists), so the delta is captured as
    /// `AddRelation` / `RemoveRelation` ops after the commit — the same ops a
    /// `RELATE` / `UNRELATE` on the target branch would produce.
    pub(super) fn copy_relations_to_batch(
        &self,
        batch: &mut WriteBatch,
        node_id: &str,
        scope: &CopyScope<'_>,
        relation_ops: &mut Vec<OpType>,
    ) -> Result<()> {
        let Some(src) = read_packed_relations_at(
            &self.db,
            scope.source_max_revision,
            scope.tenant_id,
            scope.repo_id,
            scope.source_branch,
            scope.workspace,
            node_id,
        )?
        else {
            return Ok(());
        };
        let dst = read_packed_relations_at(
            &self.db,
            scope.revision,
            scope.tenant_id,
            scope.repo_id,
            scope.target_branch,
            scope.workspace,
            node_id,
        )?
        .unwrap_or_default();

        let unchanged =
            src.len() == dst.len() && src.iter().all(|s| dst.iter().any(|d| identical(s, d)));
        if unchanged {
            return Ok(());
        }

        let key = crate::keys::node_adjacency_key_versioned(
            scope.tenant_id,
            scope.repo_id,
            scope.target_branch,
            scope.workspace,
            node_id,
            scope.revision,
        );
        batch.put_cf(
            get_relation_cf(&self.db)?,
            key,
            serialize_compact_relations(&src)?,
        );

        if !self.operation_capture.is_enabled() {
            return Ok(());
        }
        for s in &src {
            if dst.iter().any(|d| identical(s, d)) {
                continue;
            }
            relation_ops.push(OpType::AddRelation {
                source_id: node_id.to_string(),
                source_workspace: scope.workspace.to_string(),
                relation_type: s.relation_type.clone(),
                target_id: s.target_id.clone(),
                target_workspace: s.target_workspace.clone(),
                relation: RelationRef::new(
                    s.target_id.clone(),
                    s.target_workspace.clone(),
                    s.target_node_type.clone(),
                    s.relation_type.clone(),
                    s.weight,
                ),
            });
        }
        for d in &dst {
            if src.iter().any(|s| same_edge(s, d)) {
                continue;
            }
            relation_ops.push(OpType::RemoveRelation {
                source_id: node_id.to_string(),
                source_workspace: scope.workspace.to_string(),
                relation_type: d.relation_type.clone(),
                target_id: d.target_id.clone(),
                target_workspace: d.target_workspace.clone(),
            });
        }
        Ok(())
    }
}
