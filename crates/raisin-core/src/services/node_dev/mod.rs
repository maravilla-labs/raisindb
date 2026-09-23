// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The node-development surface: RaisinDB's hierarchy as a structured
//! workspace an agent (or any client) can develop in safely.
//!
//! `repository / branch / workspace / path` plays the role a filesystem plays
//! for a coding agent — branches are worktrees, nodes are typed files, node
//! ids are identity across moves — and this module gives it the properties
//! that make a filesystem agent reliable, over the EXISTING node, transaction,
//! branch and revision APIs (no new storage):
//!
//! - canonical locators `{repository, branch, workspace, path, node_id,
//!   revision}` ([`NodeLocator`]);
//! - scoped working roots with no path escape, doubling as per-root,
//!   per-operation grants ([`WorkRoot`], [`root::Roots`]);
//! - typed stat / list / read / diff / watch ([`read`]);
//! - atomic multi-node changesets with dry-run, review and commit, stale
//!   revision detection returning a conflict RESULT, exact receipts, and
//!   idempotency keys ([`changeset`]);
//! - branch diff / merge / discard with conflict reporting ([`branch`]).
//!
//! A changeset record is an ordinary node in `raisin:system` at
//! `/changesets/<xx>/<id>` on the changeset's own branch, written IN THE SAME
//! TRANSACTION as the changes it commits, so "committed" and "recorded" can
//! never disagree, and it replicates exactly like any other node. Commits on a
//! branch are serialized through [`ExclusiveSections`], which a server backs
//! with the same keyed mutex + distributed lease the flow runtime uses.

pub mod branch;
pub mod changeset;
mod changeset_apply;
mod changeset_move;
mod changeset_plan;
mod changeset_store;
pub mod changeset_types;
pub mod dispatch;
pub mod envelope;
pub mod read;
pub mod root;
pub mod types;

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use raisin_storage::{transactional::TransactionalStorage, Storage};

pub use changeset_types::*;
/// The wire contract this surface speaks (`raisin.tool-result/1`).
pub use raisin_agent_contract as contract;
pub use types::*;

/// A held exclusive section; dropping it leaves the section.
pub type SectionGuard = Box<dyn Send + Sync>;

/// Cluster-wide exclusive sections, keyed by an opaque string.
///
/// The service needs exactly one guarantee from it: two commits holding the
/// same key never overlap, on any node. [`LocalSections`] gives it within one
/// process; a server installs an implementation backed by `raisin-locks`.
#[async_trait]
pub trait ExclusiveSections: Send + Sync {
    /// Enter the section for `key`, waiting briefly; `busy` when another
    /// node holds it past the budget.
    async fn enter(&self, key: &str) -> DevResult<SectionGuard>;
}

/// In-process keyed mutex.
#[derive(Default)]
pub struct LocalSections {
    locks: std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

#[async_trait]
impl ExclusiveSections for LocalSections {
    async fn enter(&self, key: &str) -> DevResult<SectionGuard> {
        let lock = {
            let mut map = self
                .locks
                .lock()
                .map_err(|_| NodeDevError::new(500, "internal", "section map poisoned"))?;
            map.entry(key.to_string()).or_default().clone()
        };
        Ok(Box::new(lock.lock_owned().await))
    }
}

/// Where a call operates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevScope {
    /// Tenant.
    pub tenant: String,
    /// Repository.
    pub repo: String,
    /// Branch.
    pub branch: String,
}

impl DevScope {
    /// Build one.
    pub fn new(tenant: &str, repo: &str, branch: &str) -> Self {
        Self {
            tenant: tenant.to_string(),
            repo: repo.to_string(),
            branch: branch.to_string(),
        }
    }

    /// Storage scope for one workspace.
    pub fn storage<'a>(&'a self, ws: &'a str) -> raisin_storage::scope::StorageScope<'a> {
        raisin_storage::scope::StorageScope::new(&self.tenant, &self.repo, &self.branch, ws)
    }

    /// Permission scope for one workspace.
    pub fn permission<'a>(&'a self, ws: &'a str) -> raisin_models::permissions::PermissionScope {
        raisin_models::permissions::PermissionScope::new(ws, &self.branch)
    }
}

/// The node-development service over any transactional storage.
pub struct NodeDevService<S: Storage + TransactionalStorage> {
    pub(crate) storage: Arc<S>,
    pub(crate) sections: Arc<dyn ExclusiveSections>,
}

impl<S: Storage + TransactionalStorage> NodeDevService<S> {
    /// A service serializing commits in-process only.
    pub fn new(storage: Arc<S>) -> Self {
        Self::with_sections(storage, Arc::new(LocalSections::default()))
    }

    /// A service with a cluster-aware section provider.
    pub fn with_sections(storage: Arc<S>, sections: Arc<dyn ExclusiveSections>) -> Self {
        Self { storage, sections }
    }

    /// The storage.
    pub fn storage(&self) -> &Arc<S> {
        &self.storage
    }
}

#[cfg(test)]
mod tests;
