//! Who OWNS a UNIQUE_INDEX claim (plan Phase 13a).
//!
//! A claim key is `(node_type, property, value hash, revision)` with the owner
//! in the VALUE, so two nodes taking turns on one value at one revision write
//! ONE key — and the later write wins. Every claim-ending tombstone therefore
//! asks two questions before it is written:
//!
//! - **committed state** ([`held_by_other_at`]): is there already an entry AT
//!   the tombstone's exact revision naming a DIFFERENT live node? Then that
//!   key is the other node's claim (a replica that applied the taker of a
//!   promotion's hand-over before its giver), and it is left alone;
//! - **this commit** ([`CommitClaims`]): does another node of the SAME batch
//!   claim the value at that revision? A batch is unreadable until written,
//!   so a commit-time correction appended after a promotion's puts would
//!   otherwise erase the claim its replacement had just staged.
//!
//! The committed check is deliberately limited to a same-revision KEY
//! COLLISION. Skipping because some other node's claim sits BELOW the
//! tombstone made the outcome depend on which peer ops a replica had already
//! applied — two replicas holding the same ops ended with different claims
//! (`unique_claim_convergence_test`). A tombstone that lands over another
//! node's older claim is what every node writes, so the replicas converge.
//!
//! [`owned_unique_names`] answers the inverse question for a writer that has
//! no definitions (the replication apply path with a cold definitions cache):
//! which of a version's properties hold a claim THIS node has held. It needs
//! no NodeType read, so it is safe on the apply path, and it is exact — a
//! claim exists only for a property some version declared `unique: true`.

use super::unique_delta::{claim_key, UniqueClaim};
use crate::indexing::IndexCtx;
use crate::repositories::hash_property_value;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::DB;
use std::collections::{HashMap, HashSet};

/// The UNIQUE claims one commit's final versions hold, by full claim key
/// (workspace and revision included) -> owner. Built under the commit lock
/// from the definitions cache; a type that is cold there contributes none
/// and is remembered as cold, so a correction needing it writes no UNIQUE
/// tombstone either ([`Self::any_cold`]) — even if the cache warms in
/// between.
///
/// A merge resolution also records here the claims the merged branches'
/// newest entries give to OTHER nodes (`merge::merged_view`), so neither the
/// resolution nor its commit-time correction ends them.
#[derive(Debug, Default, Clone)]
pub(crate) struct CommitClaims {
    claims: HashMap<Vec<u8>, String>,
    cold_types: HashSet<String>,
}

impl CommitClaims {
    /// Record `claims`, held by `node_id` at `revision` in `ctx`'s workspace.
    pub(crate) fn hold<'c>(
        &mut self,
        ctx: &IndexCtx<'_>,
        claims: impl IntoIterator<Item = &'c UniqueClaim>,
        revision: &HLC,
        node_id: &str,
    ) {
        for claim in claims {
            self.claims
                .insert(claim_key(ctx, claim, revision), node_id.to_string());
        }
    }

    /// Record that `node_type`'s definitions were cold when the claims were
    /// collected: its claims (if any) are missing from this set.
    pub(crate) fn mark_cold(&mut self, node_type: &str) {
        self.cold_types.insert(node_type.to_string());
    }

    /// Whether any of `types` was cold when the claims were collected.
    pub(crate) fn any_cold<'t>(&self, types: impl IntoIterator<Item = &'t str>) -> bool {
        !self.cold_types.is_empty() && types.into_iter().any(|t| self.cold_types.contains(t))
    }

    /// Fold `other`'s claims and cold types into this set.
    pub(crate) fn extend(&mut self, other: &CommitClaims) {
        self.claims
            .extend(other.claims.iter().map(|(k, v)| (k.clone(), v.clone())));
        self.cold_types.extend(other.cold_types.iter().cloned());
    }

    /// Whether another node of the commit holds `claim` at `at`.
    pub(crate) fn held_by_other(
        &self,
        ctx: &IndexCtx<'_>,
        claim: &UniqueClaim,
        at: &HLC,
        node_id: &str,
    ) -> bool {
        self.claims
            .get(&claim_key(ctx, claim, at))
            .is_some_and(|owner| owner != node_id)
    }
}

/// The UNIQUE_INDEX key prefix of `claim` (every revision of it).
pub(crate) fn claim_prefix(
    ctx: &IndexCtx<'_>,
    (node_type, property, hash): &UniqueClaim,
) -> Vec<u8> {
    keys::unique_index_value_prefix(
        ctx.tenant_id,
        ctx.repo_id,
        ctx.branch,
        ctx.workspace,
        node_type,
        property,
        hash,
    )
}

/// Whether committed state holds, at exactly `at`, a live claim of `claim`
/// naming a node other than `node_id` — the one key a tombstone at `at`
/// would overwrite. See the module doc for why only that key counts.
pub(crate) fn held_by_other_at(
    db: &DB,
    ctx: &IndexCtx<'_>,
    claim: &UniqueClaim,
    node_id: &str,
    at: &HLC,
) -> Result<bool> {
    let cf_unique = cf_handle(db, cf::UNIQUE_INDEX)?;
    Ok(crate::mvcc_read::newest_at_or_before_with(
        db,
        cf_unique,
        &claim_prefix(ctx, claim),
        Some(at),
        |revision, value| {
            revision == *at
                && !value.is_empty()
                && !keys::is_tombstone_value(value)
                && value != node_id.as_bytes()
        },
    )?
    .unwrap_or(false))
}

/// The top-level properties of `node` whose claim — `(node.node_type, name,
/// hash of its value)` — `node.id` has held at some revision at or before
/// `at` (any entry naming it, not just the newest).
///
/// The definitions-free way to find every claim a version holds: used where
/// the `unique: true` names are not known without a NodeType read (a
/// replicated delete; a replicated upsert with the definitions cache cold).
/// Asking about the node's OWN entry, not the newest one, keeps the answer
/// independent of which other nodes' ops this replica has applied: another
/// node's claim at the same or a lower revision must not hide this one's
/// (a promotion's taker applied before its giver; a concurrent duplicate).
/// One seek per top-level property, walking only that value's entries.
pub(crate) fn owned_unique_names(
    db: &DB,
    ctx: &IndexCtx<'_>,
    node: &Node,
    at: &HLC,
) -> Result<Vec<String>> {
    let cf_unique = cf_handle(db, cf::UNIQUE_INDEX)?;
    let mut names = Vec::new();
    for (name, value) in &node.properties {
        let claim = (
            node.node_type.clone(),
            name.clone(),
            hash_property_value(value),
        );
        let held = crate::mvcc_read::any_at_or_before(
            db,
            cf_unique,
            &claim_prefix(ctx, &claim),
            at,
            |_, v| v == node.id.as_bytes(),
        )?;
        if held {
            names.push(name.clone());
        }
    }
    names.sort();
    Ok(names)
}
