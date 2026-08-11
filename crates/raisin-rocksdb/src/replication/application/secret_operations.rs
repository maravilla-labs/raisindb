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

//! Applying a replicated secret version.
//!
//! Writes the peer's record VERBATIM at the peer's revision: same ciphertext,
//! same `key_id`, same ordinal, same key. Three consequences, all wanted:
//!
//! - **No keyring is needed here.** This node may not even hold the master key
//!   the origin sealed under; that only matters at reveal time, where the store
//!   reports `UnknownKeyId` and names the missing id.
//! - **Redelivery is idempotent.** The key is the origin's, so a second delivery
//!   rewrites the same bytes rather than appending a duplicate version.
//! - **A pinned `secret://name@N` resolves to the same bytes on every node.**

use raisin_error::Result;
use raisin_replication::{Operation, ReplicatedSecret};

use super::super::super::secret_store::{lifecycle, SecretRecord, SecretScope};
use super::applicator::OperationApplicator;

pub(super) async fn apply_upsert_secret(
    applicator: &OperationApplicator,
    name: &str,
    secret: &ReplicatedSecret,
    op: &Operation,
) -> Result<()> {
    let scope = SecretScope::new(&op.tenant_id, &op.repo_id, &op.branch);
    let record = SecretRecord {
        ciphertext: secret.ciphertext.clone(),
        key_id: secret.key_id,
        version: secret.version,
        created_at: secret.created_at.clone(),
        created_by: secret.created_by.clone(),
        rotated_at: secret.rotated_at.clone(),
        owner_node: secret.owner_node.clone(),
        owner_field: secret.owner_field.clone(),
        deleted: secret.deleted,
    };

    tracing::debug!(
        tenant = %op.tenant_id,
        repo = %op.repo_id,
        branch = %op.branch,
        name = %name,
        version = secret.version,
        deleted = secret.deleted,
        "📥 Applying secret version"
    );

    lifecycle::write_record(applicator.db(), &scope, name, &secret.revision, &record)?;
    Ok(())
}
