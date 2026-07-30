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

//! Capture of workspace operations.
//!
//! Separate from `schema_ops` because a workspace is not a versioned schema
//! object: it is repo-scoped, carries no revision, and resolves conflicts from
//! its own `updated_at` on the apply side.

use raisin_error::Result;
use raisin_replication::{OpType, Operation};

use super::core::OperationCapture;

impl OperationCapture {
    /// Capture an UpdateWorkspace operation
    ///
    /// A workspace record is repo-scoped, not branch-scoped — the applier writes
    /// `keys::workspace_key(tenant, repo, name)` and ignores `op.branch` — so the
    /// branch is carried purely as routing context, exactly like
    /// `capture_update_repository` does.
    ///
    /// The workspace is not MVCC-versioned, so no revision is attached; conflicts
    /// are resolved on the apply side from the record's own `updated_at`
    /// (`replication/application/applicator/workspace_lww.rs`). Nothing here may
    /// be sent that has not first been stamped by the write path, or the guard on
    /// the receiving node has nothing to compare.
    pub async fn capture_update_workspace(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        workspace_id: String,
        workspace: raisin_models::workspace::Workspace,
        actor: String,
    ) -> Result<Operation> {
        self.capture_operation(
            tenant_id,
            repo_id,
            branch,
            OpType::UpdateWorkspace {
                workspace_id,
                workspace,
            },
            actor,
            None,
            false,
        )
        .await
    }
}
