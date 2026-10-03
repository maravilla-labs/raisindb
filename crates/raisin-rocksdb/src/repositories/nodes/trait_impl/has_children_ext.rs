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

//! Helper for computing `has_children` on node lists.
//!
//! Several `NodeRepository` trait methods share the same pattern:
//! call an internal `_impl` method, then optionally iterate results
//! to populate `has_children`.  This module extracts that shared loop
//! so the main dispatcher stays thin.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;

use crate::repositories::nodes::NodeRepositoryImpl;

impl NodeRepositoryImpl {
    /// Populate `has_children` for every node in the list.
    ///
    /// Runs the `has_children` existence probe for each node and stores the
    /// result in `node.has_children`.  This is intentionally a separate
    /// helper so the main trait dispatcher does not repeat the same loop
    /// in `list_by_type`, `list_all`, and `list_children`.
    ///
    /// The node is in hand, so its path answers the root check — the
    /// id-only `NodeRepository::has_children` would have to look it up.
    pub(crate) async fn populate_has_children(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        nodes: &mut [Node],
        max_revision: Option<&HLC>,
    ) -> Result<()> {
        for node in nodes.iter_mut() {
            let has_children = self.probe_has_children(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &node.id,
                Some(&node.path),
                max_revision,
            )?;
            node.has_children = Some(has_children);
        }
        Ok(())
    }
}
