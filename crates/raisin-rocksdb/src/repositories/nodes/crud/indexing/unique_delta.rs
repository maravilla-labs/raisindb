//! The UNIQUE_INDEX delta (plan Phase 7 item 4): tombstone the claims whose
//! `(node_type, property, value hash)` changed, and re-put EVERY claim the new
//! version holds.
//!
//! Claims are deliberately NOT skipped when unchanged (0–2 per node, so the
//! saving is negligible). Nothing rebuilds or verifies UNIQUE_INDEX — the
//! `property_index` rebuild certifies PROPERTY_INDEX only — and two sources of
//! holes exist that only a re-put heals: the replication apply path writes no
//! claims at all (a replicated user holds none on the replica), and a schema
//! change that adds `unique: true` leaves every existing node without its
//! claim while the claim looks "unchanged" against the stored version. A
//! skipped claim was then never written, and a second node could take the
//! value (`unique_claim_reput_after_replicated_create`). Re-putting also keeps
//! a claim at every version's own revision, so a write committed BELOW a
//! stored successor cannot mask the successor's claim with a tombstone.

use super::unique_guard::{held_by_other_at, CommitClaims};
use crate::indexing::IndexCtx;
use crate::repositories::hash_property_value;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::{WriteBatch, DB};
use std::collections::BTreeSet;

/// One UNIQUE_INDEX claim: `(node_type, property, value hash)` — the key
/// minus its revision.
pub(crate) type UniqueClaim = (String, String, String);

/// A node version and the `unique: true` property names of its NodeType.
#[derive(Clone, Copy)]
pub(crate) struct UniqueSide<'a> {
    pub(crate) node: &'a Node,
    pub(crate) properties: &'a [String],
}

/// The claims `side` holds.
pub(crate) fn unique_claims(side: UniqueSide<'_>) -> BTreeSet<UniqueClaim> {
    side.properties
        .iter()
        .filter_map(|name| {
            side.node.properties.get(name).map(|value| {
                (
                    side.node.node_type.clone(),
                    name.clone(),
                    hash_property_value(value),
                )
            })
        })
        .collect()
}

/// Write the UNIQUE_INDEX change from `old` to `new` at `revision`: the
/// [`write_unique_ends`] half, then the [`write_unique_puts`] half.
///
/// Every claim `new` holds is re-put (see the module doc for why none is
/// skipped); claims only `old` held are tombstoned. `in_place` (a reused
/// `versionable=false` revision): a write lands on the claim's newest entry
/// when that entry is above `revision` and this node's own (plan item 8).
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_unique_delta(
    batch: &mut WriteBatch,
    db: &DB,
    ctx: &IndexCtx<'_>,
    old: Option<UniqueSide<'_>>,
    new: UniqueSide<'_>,
    revision: &HLC,
    in_place: bool,
) -> Result<()> {
    write_unique_ends(batch, db, ctx, old, new, revision, in_place, None)?;
    write_unique_puts(batch, db, ctx, new, revision, in_place)
}

/// The tombstone half of [`write_unique_delta`]: end the claims only `old`
/// held. A claim another node owns is left alone — as of the tombstone's
/// revision in committed state, or in this commit (`held`); see
/// `unique_guard`. A writer that stages several nodes into ONE batch writes
/// every node's ends before any node's puts (cross-branch promotion), so a
/// value handed from one node to another in the batch is never erased by the
/// node giving it up.
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_unique_ends(
    batch: &mut WriteBatch,
    db: &DB,
    ctx: &IndexCtx<'_>,
    old: Option<UniqueSide<'_>>,
    new: UniqueSide<'_>,
    revision: &HLC,
    in_place: bool,
    held: Option<&CommitClaims>,
) -> Result<()> {
    let cf_unique = cf_handle(db, cf::UNIQUE_INDEX)?;
    let old_claims = old.map(unique_claims).unwrap_or_default();
    let new_claims = unique_claims(new);
    let node_id = new.node.id.as_str();
    for claim in old_claims.difference(&new_claims) {
        let at = claim_revision(db, cf_unique, ctx, claim, node_id, revision, in_place)?;
        end_claim(batch, db, ctx, claim, node_id, &at, held)?;
    }
    Ok(())
}

/// The put half of [`write_unique_delta`]: re-put every claim `new` holds.
pub(crate) fn write_unique_puts(
    batch: &mut WriteBatch,
    db: &DB,
    ctx: &IndexCtx<'_>,
    new: UniqueSide<'_>,
    revision: &HLC,
    in_place: bool,
) -> Result<()> {
    let cf_unique = cf_handle(db, cf::UNIQUE_INDEX)?;
    let node_id = new.node.id.as_str();
    for claim in &unique_claims(new) {
        let at = claim_revision(db, cf_unique, ctx, claim, node_id, revision, in_place)?;
        batch.put_cf(cf_unique, claim_key(ctx, claim, &at), node_id.as_bytes());
    }
    Ok(())
}

/// End, at `at` (the first stored successor's revision), every claim `new`
/// holds that `next` (that successor; `None` = a delete) does not — the
/// out-of-order half of an apply landing below a stored version, so the lower
/// version's claim is not live at HEAD.
pub(crate) fn end_claims_at(
    batch: &mut WriteBatch,
    db: &DB,
    ctx: &IndexCtx<'_>,
    new: UniqueSide<'_>,
    next: Option<UniqueSide<'_>>,
    at: &HLC,
) -> Result<()> {
    end_claims_at_held(batch, db, ctx, new, next, at, None)
}

/// [`end_claims_at`], also sparing the claims another node of this commit
/// holds (`held`).
pub(crate) fn end_claims_at_held(
    batch: &mut WriteBatch,
    db: &DB,
    ctx: &IndexCtx<'_>,
    new: UniqueSide<'_>,
    next: Option<UniqueSide<'_>>,
    at: &HLC,
    held: Option<&CommitClaims>,
) -> Result<()> {
    let kept = next.map(unique_claims).unwrap_or_default();
    for claim in unique_claims(new).difference(&kept) {
        end_claim(batch, db, ctx, claim, &new.node.id, at, held)?;
    }
    Ok(())
}

/// Tombstone `claim` at `at` for `node_id`, unless another node holds that
/// very key (committed at `at`, or in this commit; see `unique_guard`).
///
/// THE place a UNIQUE claim is ended: every claim-ending tombstone — the
/// delta's ends, the out-of-order ends, the delete tombstoner
/// (`tombstone_unique_entries`), the commit-time correction — goes through
/// it, so the ownership rule has one body.
pub(crate) fn end_claim(
    batch: &mut WriteBatch,
    db: &DB,
    ctx: &IndexCtx<'_>,
    claim: &UniqueClaim,
    node_id: &str,
    at: &HLC,
    held: Option<&CommitClaims>,
) -> Result<()> {
    if held.is_some_and(|held| held.held_by_other(ctx, claim, at, node_id))
        || held_by_other_at(db, ctx, claim, node_id, at)?
    {
        return Ok(());
    }
    let cf_unique = cf_handle(db, cf::UNIQUE_INDEX)?;
    batch.put_cf(cf_unique, claim_key(ctx, claim, at), keys::TOMBSTONE_VALUE);
    Ok(())
}

pub(crate) fn claim_key(
    ctx: &IndexCtx<'_>,
    (node_type, property, hash): &UniqueClaim,
    at: &HLC,
) -> Vec<u8> {
    keys::unique_index_key_versioned(
        ctx.tenant_id,
        ctx.repo_id,
        ctx.branch,
        ctx.workspace,
        node_type,
        property,
        hash,
        at,
    )
}

fn claim_revision(
    db: &DB,
    cf_unique: &rocksdb::ColumnFamily,
    ctx: &IndexCtx<'_>,
    (node_type, property, hash): &UniqueClaim,
    node_id: &str,
    revision: &HLC,
    in_place: bool,
) -> Result<HLC> {
    if !in_place {
        return Ok(*revision);
    }
    let prefix = keys::unique_index_value_prefix(
        ctx.tenant_id,
        ctx.repo_id,
        ctx.branch,
        ctx.workspace,
        node_type,
        property,
        hash,
    );
    Ok(
        match crate::mvcc_read::newest_at_or_before(db, cf_unique, &prefix, None)? {
            Some((newest, owner)) if newest > *revision && owner == node_id.as_bytes() => newest,
            _ => *revision,
        },
    )
}

impl crate::repositories::NodeRepositoryImpl {
    /// The `unique: true` property names of `node_type` on `branch` (empty
    /// when the type does not resolve: no type, no unique claims).
    pub(crate) async fn unique_property_names(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        node_type: &str,
    ) -> Result<Vec<String>> {
        // Through the one definitions cache (`indexing::compound::defs`), which
        // the replication apply path reads without a NodeType read.
        Ok(crate::indexing::compound::defs::resolve(
            &self.db,
            self.node_type_repo.as_ref(),
            raisin_storage::BranchScope::new(tenant_id, repo_id, branch),
            node_type,
        )
        .await?
        .unique
        .clone())
    }

    /// Stage the UNIQUE_INDEX change from `old` to `new` (see
    /// [`write_unique_delta`]), resolving both sides' unique properties.
    /// `half` selects the ends, the puts, or both.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn add_unique_delta_to_batch(
        &self,
        batch: &mut WriteBatch,
        old: Option<&Node>,
        new: &Node,
        ctx: &IndexCtx<'_>,
        revision: &HLC,
        in_place: bool,
        half: UniqueHalf,
    ) -> Result<()> {
        let old_props = match old {
            Some(old) => {
                self.unique_property_names(ctx.tenant_id, ctx.repo_id, ctx.branch, &old.node_type)
                    .await?
            }
            None => Vec::new(),
        };
        let new_props = self
            .unique_property_names(ctx.tenant_id, ctx.repo_id, ctx.branch, &new.node_type)
            .await?;
        if old_props.is_empty() && new_props.is_empty() {
            return Ok(());
        }
        let old = old.map(|node| UniqueSide {
            node,
            properties: &old_props,
        });
        let new = UniqueSide {
            node: new,
            properties: &new_props,
        };
        if half != UniqueHalf::Puts {
            write_unique_ends(batch, &self.db, ctx, old, new, revision, in_place, None)?;
        }
        if half != UniqueHalf::Ends {
            write_unique_puts(batch, &self.db, ctx, new, revision, in_place)?;
        }
        Ok(())
    }
}

/// Which half of a UNIQUE delta `add_unique_delta_to_batch`
/// stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UniqueHalf {
    /// Ends and puts, in that order (a batch staging one node).
    Both,
    /// Only the claims the node gives up (a multi-node batch's first pass).
    Ends,
    /// Only the claims the node holds (its second pass).
    Puts,
}
