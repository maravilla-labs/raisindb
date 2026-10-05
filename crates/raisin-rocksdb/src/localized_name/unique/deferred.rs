//! The repository write paths' uniqueness check, carried to their commit
//! step and run again there UNDER THE BRANCH LOCK.
//!
//! The check reads stored state, so it is a check-then-act: two repository
//! writes naming two siblings alike (two HTTP `/translations` PUTs of an
//! auto-translate batch), or one landing between a transaction commit's check
//! and its write, both passed and both stored a claim — under a state record
//! still saying `Ready` with zero collisions. The transaction commit runs its
//! check under the branch lock (`transaction/commit/localized_names.rs`); a
//! repository write now does too, through its `indexing::NodeCommit`:
//! `write_batch_with_head_as` runs it right after taking the lock it already
//! takes, and `NodeCommit::write` takes that lock for it — only when a check
//! is carried, i.e. only where uniqueness is enforced, so every other write
//! stays as lock-free as it was.

use super::super::keys::NameScope;
use super::super::sync::Overrides;
use super::{active, check_unique};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::DB;

/// One node's uniqueness check, to run under the branch lock at commit.
#[derive(Debug, Clone)]
pub(crate) struct NameCheck {
    workspace: String,
    node: Node,
    parent_id: Option<String>,
    revision: HLC,
    overrides: Overrides,
}

impl NameCheck {
    /// The check of `node` (with its staged `overrides`) when the repository
    /// enforces uniqueness on this branch's workspace — run once NOW, so a
    /// refused write fails before it stages anything, and returned to be run
    /// again at the commit step. `None`: nothing is enforced here.
    pub(crate) fn staged(
        db: &DB,
        scope: NameScope<'_>,
        node: &Node,
        parent_id: Option<&str>,
        revision: &HLC,
        overrides: Overrides,
    ) -> Result<Option<Self>> {
        if active(db, scope)?.is_none() {
            return Ok(None);
        }
        check_unique(db, scope, node, parent_id, revision, &overrides)?;
        Ok(Some(Self {
            workspace: scope.workspace.to_string(),
            node: node.clone(),
            parent_id: parent_id.map(str::to_string),
            revision: *revision,
            overrides,
        }))
    }

    /// Run the check against what is stored NOW. Call with the branch record
    /// lock of `(tenant_id, repo_id, branch)` held.
    pub(crate) fn run(&self, db: &DB, tenant_id: &str, repo_id: &str, branch: &str) -> Result<()> {
        check_unique(
            db,
            NameScope::new(tenant_id, repo_id, branch, &self.workspace),
            &self.node,
            self.parent_id.as_deref(),
            &self.revision,
            &self.overrides,
        )
    }
}
