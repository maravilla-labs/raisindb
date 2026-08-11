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

//! Capture for secret-store versions.
//!
//! # The lane is the whole ordering story
//!
//! Unlike [`oauth_ops`](super::oauth_ops), which captures deployment-wide auth
//! state on the `system`/`main` pseudo repo, a secret is captured on the node's
//! **real** `(tenant, repo, branch)`. `OperationCapture` keeps one vector clock
//! per `(tenant, repo)`, so capturing the secret on the same lane as the node
//! that references it — and BEFORE it, in the same commit — is what makes the
//! receiving side's causal delivery buffer hold the node snapshot until the
//! secret has landed. Move this to the `system` lane and the two operations
//! become causally unrelated: the guarantee does not weaken, it disappears.
//!
//! # Never plaintext
//!
//! [`ReplicatedSecret`] is built from the stored record, whose `ciphertext` is
//! the sealed envelope. The plaintext never existed on this side of the vaulter.

use super::OperationCapture;
use raisin_error::Result;
use raisin_replication::{OpType, Operation, ReplicatedSecret};

use crate::secret_store::StoredSecret;

/// The wire form of a stored version.
pub fn replicated_secret(stored: &StoredSecret) -> ReplicatedSecret {
    ReplicatedSecret {
        ciphertext: stored.record.ciphertext.clone(),
        key_id: stored.record.key_id,
        version: stored.record.version,
        revision: stored.revision,
        created_at: stored.record.created_at.clone(),
        created_by: stored.record.created_by.clone(),
        rotated_at: stored.record.rotated_at.clone(),
        owner_node: stored.record.owner_node.clone(),
        owner_field: stored.record.owner_field.clone(),
        deleted: stored.record.deleted,
    }
}

impl OperationCapture {
    /// Capture one secret version — a mint, a rotation, a promotion copy or a
    /// tombstone; the record's `deleted` flag distinguishes them, so there is
    /// one operation for the whole lifecycle.
    pub async fn capture_upsert_secret(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        stored: &StoredSecret,
        actor: String,
    ) -> Result<Operation> {
        self.capture_operation(
            tenant_id,
            repo_id,
            branch,
            OpType::UpsertSecret {
                name: stored.name.clone(),
                secret: replicated_secret(stored),
            },
            actor,
            None,
            false,
        )
        .await
    }
}
