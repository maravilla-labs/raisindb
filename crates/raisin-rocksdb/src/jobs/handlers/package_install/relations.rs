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

//! Install phase: graph relations (`relations.yaml`).
//!
//! Runs after the content phase, so both endpoints of a relation can be
//! resolved by `(workspace, path)` on the target branch, whatever ids the
//! content got (a package written by hand has none, and an install keeps the
//! id of a node already at a path). A user home's path is its stable key, so
//! users need no special case.
//!
//! Modes keep their meaning for relations:
//! - every mode ADDS a relation the target does not have yet;
//! - `skip` leaves an existing relation of the same `(from, type, to)` alone;
//! - `sync` and `overwrite` set its weight to the package's;
//! - no mode removes a relation (the format has no notion of removal).
//!
//! An install is idempotent: a relation that is already there is counted as
//! `relations_existing`, not created again and not an error. A relation whose
//! endpoint does not resolve is counted as `relations_unresolved` and named in
//! the job log; it does not fail the install (the content is in place).

use super::content_types::InstallStats;
use super::handler::PackageInstallHandler;
use super::types::InstallMode;
use raisin_error::{Error, Result};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::RelationRef;
use raisin_packages::{PackageRelation, PackageRelations, RELATIONS_FILE};
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::{NodeRepository, RelationRepository, Storage, StorageScope};
use std::io::{Cursor, Read};
use zip::ZipArchive;

/// What happened to one package relation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RelationOutcome {
    /// Added.
    Created,
    /// Already present with the package's weight, or left alone (`skip`).
    Existing,
    /// Present, weight set to the package's (`sync` / `overwrite`).
    Updated,
}

/// Decide what to do with a relation whose current weight on the target is
/// `existing` (`None`: no such relation) under `mode`.
pub(super) fn decide(
    existing: Option<Option<f32>>,
    wanted: Option<f32>,
    mode: InstallMode,
) -> RelationOutcome {
    match existing {
        None => RelationOutcome::Created,
        Some(w) if w == wanted || mode == InstallMode::Skip => RelationOutcome::Existing,
        Some(_) => RelationOutcome::Updated,
    }
}

impl<S: Storage + TransactionalStorage> PackageInstallHandler<S> {
    /// Install `relations.yaml`, if the package has one.
    pub(super) async fn install_package_relations(
        &self,
        archive: &mut ZipArchive<Cursor<&Vec<u8>>>,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        install_mode: InstallMode,
        stats: &mut InstallStats,
    ) -> Result<()> {
        let yaml = match archive.by_name(RELATIONS_FILE) {
            Ok(mut file) => {
                let mut s = String::new();
                file.read_to_string(&mut s)
                    .map_err(|e| Error::Validation(format!("{RELATIONS_FILE}: {e}")))?;
                s
            }
            Err(_) => return Ok(()),
        };
        let doc = PackageRelations::parse(&yaml)
            .map_err(|e| Error::Validation(format!("{RELATIONS_FILE}: {e}")))?;
        if doc.relations.is_empty() {
            return Ok(());
        }

        let tx = self.storage.begin_context().await?;
        tx.set_tenant_repo(tenant_id, repo_id)?;
        tx.set_branch(branch)?;
        tx.set_actor("package-install")?;
        tx.set_message(&format!(
            "Package install relations ({} items)",
            doc.relations.len()
        ))?;
        tx.set_auth_context(AuthContext::system())?;
        tx.set_content_import(true)?;
        tx.set_retired_keys_override(stats.allow_retired_user_keys)?;

        let mut wrote = false;
        for rel in &doc.relations {
            // An endpoint in a retired home (a deleted person) is skipped.
            if self
                .retired_home_key(tenant_id, &rel.from.workspace, &rel.from.path, stats)?
                .is_some()
                || self
                    .retired_home_key(tenant_id, &rel.to.workspace, &rel.to.path, stats)?
                    .is_some()
            {
                stats.relations_skipped_retired += 1;
                continue;
            }
            match self
                .install_one_relation(tx.as_ref(), tenant_id, repo_id, branch, rel, install_mode)
                .await?
            {
                Some(RelationOutcome::Created) => {
                    stats.relations_created += 1;
                    wrote = true;
                }
                Some(RelationOutcome::Updated) => {
                    stats.relations_updated += 1;
                    wrote = true;
                }
                Some(RelationOutcome::Existing) => stats.relations_existing += 1,
                None => {
                    stats.relations_unresolved += 1;
                    stats.relations_unresolved_list.push(format!(
                        "{}:{} -[{}]-> {}:{}",
                        rel.from.workspace,
                        rel.from.path,
                        rel.relation_type,
                        rel.to.workspace,
                        rel.to.path
                    ));
                }
            }
        }
        if wrote {
            tx.commit().await?;
        } else {
            tx.rollback().await?;
        }
        if stats.relations_unresolved > 0 {
            tracing::warn!(
                unresolved = stats.relations_unresolved,
                first = ?stats.relations_unresolved_list.first(),
                "Package relations with an endpoint that does not exist were skipped"
            );
        }
        Ok(())
    }

    /// Resolve and write one relation; `None` when an endpoint is missing.
    async fn install_one_relation(
        &self,
        tx: &dyn TransactionalContext,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        rel: &PackageRelation,
        install_mode: InstallMode,
    ) -> Result<Option<RelationOutcome>> {
        let nodes = self.storage.nodes();
        let Some(source) = nodes
            .get_by_path(
                StorageScope::new(tenant_id, repo_id, branch, &rel.from.workspace),
                &rel.from.path,
                None,
            )
            .await?
        else {
            return Ok(None);
        };
        let Some(target) = nodes
            .get_by_path(
                StorageScope::new(tenant_id, repo_id, branch, &rel.to.workspace),
                &rel.to.path,
                None,
            )
            .await?
        else {
            return Ok(None);
        };

        let existing = self
            .storage
            .relations()
            .get_outgoing_relations(
                StorageScope::new(tenant_id, repo_id, branch, &rel.from.workspace),
                &source.id,
                None,
            )
            .await?
            .into_iter()
            .find(|r| {
                r.target == target.id
                    && r.workspace == rel.to.workspace
                    && r.relation_type == rel.relation_type
            })
            .map(|r| r.weight);

        let outcome = decide(existing, rel.weight, install_mode);
        if outcome != RelationOutcome::Existing {
            tx.add_relation(
                &rel.from.workspace,
                &source.id,
                &source.node_type,
                RelationRef::new(
                    target.id.clone(),
                    rel.to.workspace.clone(),
                    target.node_type.clone(),
                    rel.relation_type.clone(),
                    rel.weight,
                ),
            )
            .await?;
        }
        Ok(Some(outcome))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_add_missing_and_only_sync_overwrite_reweigh() {
        for mode in [InstallMode::Skip, InstallMode::Sync, InstallMode::Overwrite] {
            assert_eq!(decide(None, Some(1.0), mode), RelationOutcome::Created);
            assert_eq!(
                decide(Some(Some(1.0)), Some(1.0), mode),
                RelationOutcome::Existing,
                "{mode:?}: a duplicate is not an error and not rewritten"
            );
        }
        assert_eq!(
            decide(Some(Some(0.5)), Some(1.0), InstallMode::Skip),
            RelationOutcome::Existing
        );
        assert_eq!(
            decide(Some(Some(0.5)), Some(1.0), InstallMode::Sync),
            RelationOutcome::Updated
        );
        assert_eq!(
            decide(Some(None), Some(1.0), InstallMode::Overwrite),
            RelationOutcome::Updated
        );
    }
}
