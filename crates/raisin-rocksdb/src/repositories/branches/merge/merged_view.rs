//! What the target holds for an id-less key once the merge has finished —
//! the question a resolution must ask before ending a superseded version's
//! old path or UNIQUE claim at the merge revision M.
//!
//! PATH_INDEX and UNIQUE_INDEX keys carry no node id, so a resolution's
//! tombstone at M lands on the same key as whatever else maps that path or
//! claims that value at M, and masks every entry below M. Two things put such
//! entries on the target that the resolution must not end:
//!
//! - **other resolutions at M**: each is its own commit at the SAME
//!   revision, so a node moved into the path (or handed the value) by an
//!   earlier resolution is committed at M before this one runs;
//! - **the merge copy**: `copy_branch_indexes` runs AFTER the resolutions
//!   and replays the source's entries at their original revisions, below M
//!   (a source node created into a path another node vacated, or taking the
//!   value it gave up), never overwriting a key the target holds.
//!
//! So the merged view of a key is the newer of the target's newest entry at
//! or before M and the source's at or before its HEAD, the target winning a
//! tie (`indexing::key_owner::newest_across`). An old path is ended only
//! while that view still names the node — the same rule the replicated
//! upsert and promotion use. A UNIQUE claim the view gives to ANOTHER live
//! node is recorded as held by it at M, so neither the resolution's
//! tombstoner nor its commit-time correction ends it.

use super::superseded::Superseded;
use super::unique_props::SchemaDefs;
use crate::indexing::key_owner::{newest_across, path_names_node, KeyView};
use crate::indexing::IndexCtx;
use crate::repositories::nodes::{claim_prefix, unique_claims, CommitClaims, UniqueSide};
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use rocksdb::DB;

/// The target (at or before M) and source (at or before its HEAD) of one
/// merge, in one workspace.
pub(super) struct MergedView<'a> {
    pub target: IndexCtx<'a>,
    pub merge_revision: &'a HLC,
    pub source: IndexCtx<'a>,
    pub source_head: &'a HLC,
}

impl<'a> MergedView<'a> {
    pub(super) fn new(
        target: IndexCtx<'a>,
        merge_revision: &'a HLC,
        (source_branch, source_head): (&'a str, &'a HLC),
    ) -> Self {
        Self {
            target,
            merge_revision,
            source: IndexCtx::new(
                target.tenant_id,
                target.repo_id,
                source_branch,
                target.workspace,
            ),
            source_head,
        }
    }

    fn views(&self, prefix: impl Fn(&IndexCtx<'_>) -> Vec<u8>) -> [KeyView<'a>; 2] {
        [
            KeyView {
                prefix: prefix(&self.target),
                at: self.merge_revision,
            },
            KeyView {
                prefix: prefix(&self.source),
                at: self.source_head,
            },
        ]
    }

    /// Whether `path` still maps to `node_id` once the merge is done, i.e.
    /// whether a tombstone at M ends THIS node's placement.
    pub(super) fn path_names(&self, db: &DB, path: &str, node_id: &str) -> Result<bool> {
        let views = self.views(|ctx| crate::indexing::key_owner::path_prefix(ctx, path));
        path_names_node(db, &views, node_id)
    }

    /// Every UNIQUE claim of a superseded version that the merged view gives
    /// to a live node other than `node_id`, held by that node at M.
    pub(super) fn others_claims(
        &self,
        db: &DB,
        superseded: &[Superseded],
        unique: &SchemaDefs,
        node_id: &str,
    ) -> Result<CommitClaims> {
        let cf_unique = cf_handle(db, cf::UNIQUE_INDEX)?;
        let mut held = CommitClaims::default();
        for old in superseded {
            let claims = unique_claims(UniqueSide {
                node: &old.node,
                properties: unique.of(&old.node.node_type),
            });
            for claim in &claims {
                let views = self.views(|ctx| claim_prefix(ctx, claim));
                if let Some((_, owner)) = newest_across(db, cf_unique, &views)? {
                    if !owner.is_empty()
                        && !keys::is_tombstone_value(&owner)
                        && owner != node_id.as_bytes()
                    {
                        let owner = String::from_utf8_lossy(&owner).into_owned();
                        held.hold(&self.target, [claim], self.merge_revision, &owner);
                    }
                }
            }
        }
        Ok(held)
    }
}
