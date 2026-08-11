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

//! Carrying vaulted secrets across a branch promotion.
//!
//! # The bug this closes
//!
//! A `secret://` reference is branch-AGNOSTIC — its name embeds the node id,
//! which a promotion preserves — while `cf::SECRETS` is branch-SCOPED. Promote a
//! node with an encrypted field and, without this, the target branch holds a
//! node whose reference resolves to nothing. It looks healthy from every angle
//! that matters operationally: reads return the reference verbatim and never
//! resolve it, so the promotion reports success, the node lists fine, and the
//! failure surfaces only when something finally tries to reveal the value.
//!
//! This is the path Studio's branch-based publish/merge takes —
//! `copy_nodes_across_branches`, reached from
//! `raisin-transport-ws/.../branches.rs::handle_branch_copy_nodes` — so the bug
//! is on the workflow that actually ships content, not on a corner case.
//! (`published_at` / `publish_tree` is a different, same-branch mechanism and
//! has nothing to copy.)
//!
//! Branch FORK never had this problem — `BRANCH_CF_REGISTRY` copies `cf::SECRETS`
//! wholesale — which is exactly why the gap in promotion went unnoticed.
//!
//! # Verbatim, and only the referenced version
//!
//! The record is copied byte-for-byte: same ciphertext, same `key_id`, same
//! ordinal, at the SAME storage revision. Three reasons:
//!
//! - re-sealing would need the plaintext and would change the `key_id`;
//! - the crypto context binds tenant and repo but deliberately NOT the branch
//!   (see [`SecretScope`]), so the copied bytes open on the target;
//! - keeping the source revision makes a repeated promotion write the identical
//!   key, i.e. idempotent rather than version-inflating.
//!
//! Only the version the node actually references is copied. The target branch's
//! history starts at the promotion, so earlier versions have nothing there that
//! could reference them — and copying them all would grow linearly with rotation
//! count on every promotion.

use std::collections::HashSet;

use raisin_error::Result;
use raisin_models::nodes::Node;
use rocksdb::WriteBatch;

use super::super::super::super::NodeRepositoryImpl;
use super::CopyScope;
use crate::secret_store::{lifecycle, SecretScope, StoredSecret};

impl NodeRepositoryImpl {
    /// Stage a copy of every secret this node references into the target branch.
    ///
    /// Needs no master keyring — nothing is opened. Joins the promotion's single
    /// WriteBatch, so the secret and the node it belongs to land together or not
    /// at all.
    pub(super) fn stage_secret_copies(
        &self,
        batch: &mut WriteBatch,
        node: &Node,
        scope: &CopyScope<'_>,
        out: &mut Vec<StoredSecret>,
    ) -> Result<()> {
        let refs = lifecycle::secret_refs(node);
        if refs.is_empty() {
            return Ok(());
        }

        let source = SecretScope::new(scope.tenant_id, scope.repo_id, scope.source_branch);
        let target = SecretScope::new(scope.tenant_id, scope.repo_id, scope.target_branch);
        let handle = lifecycle::secrets_cf(&self.db)?;
        let mut seen: HashSet<(String, Option<u64>)> = HashSet::new();

        for (path, reference) in refs {
            if !seen.insert((reference.name.clone(), reference.version)) {
                continue;
            }
            let Some(stored) =
                lifecycle::version_of(&self.db, &source, &reference.name, reference.version)?
            else {
                // A reference with nothing behind it on the SOURCE is already
                // broken; the promotion is not the place to fail over it, and
                // failing would block the whole node set.
                tracing::warn!(
                    node_id = %node.id,
                    field = %path,
                    secret = %reference.name,
                    source_branch = %scope.source_branch,
                    "promoting a node whose secret reference resolves to nothing on the source \
                     branch; the target will not resolve it either"
                );
                continue;
            };

            // Already there at that revision (a repeated promotion): writing the
            // identical bytes is harmless, but skipping keeps the replication
            // capture below quiet on a no-op promotion.
            if lifecycle::version_of(
                &self.db,
                &target,
                &reference.name,
                Some(stored.record.version),
            )?
            .is_some()
            {
                continue;
            }

            lifecycle::write_record_to_batch(
                batch,
                handle,
                &target,
                &stored.name,
                &stored.revision,
                &stored.record,
            )?;
            out.push(stored);
        }

        Ok(())
    }
}
