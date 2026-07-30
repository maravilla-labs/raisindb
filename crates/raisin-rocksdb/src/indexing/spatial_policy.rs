//! Resolving the spatial policy for the geometry properties of ONE node, once,
//! before the batch lock is taken.
//!
//! Split out of [`super::spatial`] because it answers a different question from
//! the writer: the writer turns a policy into keys, this decides *which* policy.
//! Keeping them together also pushed that file past the 300-line limit.
//!
//! Resolution needs the workspace configuration, which is an ordinary RocksDB
//! point lookup, so it happens here in the caller rather than inside the
//! synchronous writer — the writer must stay callable from the replication apply
//! path, which cannot await and must not read.
//!
//! # Nested paths
//!
//! Policies are keyed by the walker's dot path ([`super::spatial_walk`]), so a
//! geometry at `venue.geo` gets `venue.geo`'s policy and a geometry at
//! `stops.3.geo` gets the policy configured under `stops[].geo` — the array-index
//! normalisation lives in `raisin_models::nodes::properties::policy_key_for_path`
//! and is applied by the state store's key builder, so exactly one implementation
//! serves both the configuration surface and the query planner.

use super::spatial::{TombstonePrecisions, ALL_PRECISIONS};
use super::spatial_walk::walk_geometries;
use super::IndexCtx;
use crate::spatial_state::SpatialStateStore;
use raisin_models::nodes::properties::{sorted_precisions, SpatialPolicy};
use raisin_models::nodes::Node;
use std::collections::HashMap;

/// The resolved spatial policy for each geometry property path of a node.
#[derive(Debug, Clone, Default)]
pub struct NodeSpatialPolicies {
    /// What the write must EMIT: configured ∪ indexed while the two differ.
    per_property: HashMap<String, SpatialPolicy>,
    /// What the configuration DECLARES, kept separately so the state record a
    /// first write creates is stamped with intent rather than with the migration
    /// union.
    configured: HashMap<String, SpatialPolicy>,
    default: SpatialPolicy,
    /// Whether the local index-state record was actually consulted while building
    /// this map. Only then is the write policy's precision set a trustworthy
    /// upper bound on what is physically present — see
    /// [`Self::tombstone_precisions`].
    state_consulted: bool,
}

impl NodeSpatialPolicies {
    /// Every geometry property uses the default policy.
    pub fn all_default() -> Self {
        Self::default()
    }

    /// Build from an explicit per-property map plus a workspace default.
    pub fn new(per_property: HashMap<String, SpatialPolicy>, default: SpatialPolicy) -> Self {
        Self {
            configured: per_property.clone(),
            per_property,
            default,
            state_consulted: false,
        }
    }

    /// Resolve the policy to WRITE for one property path.
    pub fn for_property(&self, property: &str) -> &SpatialPolicy {
        self.per_property.get(property).unwrap_or(&self.default)
    }

    /// The DECLARED policy for one property path, i.e. configured intent without
    /// the migration union. Falls back to the write policy when the path was not
    /// resolved (the two are identical whenever nothing is in flight).
    pub fn configured_for_property(&self, property: &str) -> &SpatialPolicy {
        self.configured
            .get(property)
            .unwrap_or_else(|| self.for_property(property))
    }

    /// The precision set a tombstone pass for `property` must cover.
    ///
    /// # Why this is not simply all twelve precisions
    ///
    /// It used to be. `tombstone_spatial_property` widened unconditionally to
    /// `1..=12`, so a position update cost 8 puts + **12** tombstones = 20
    /// spatial-index key writes. For static places that is a one-off on edit and
    /// nobody notices. For a tracked vehicle reporting every few seconds it is the
    /// dominant write cost in the system, and it made the per-property precision
    /// knob almost useless: dropping to a two-precision tracking profile took the
    /// cost from 20 to 14, a 30% saving where the configuration promised 4×.
    ///
    /// The bound is available for free. `resolve_write_policy` already computes
    /// the write policy as `configured ∪ indexed`, where `indexed` comes from the
    /// state record and is exactly "the precisions that currently hold entries".
    /// Nothing outside that union can have been written by any path that
    /// respected the record, so tombstoning it is complete. A tracking profile of
    /// `[6, 8]` therefore costs 2 puts + 2 tombstones = **4 writes per position
    /// update**, a 5× reduction.
    ///
    /// # When the bound is NOT taken
    ///
    /// When no state record was consulted ([`Self::all_default`],
    /// [`Self::new`] — the replication apply path and tests) there is no evidence
    /// about what is physically present, so this falls back to
    /// [`ALL_PRECISIONS`]. Under-tombstoning leaves a live stale entry, which is
    /// the one failure this subsystem must never have; over-tombstoning only
    /// costs writes.
    pub fn tombstone_precisions<'a>(
        &self,
        property: &str,
        policy: &'a SpatialPolicy,
    ) -> TombstonePrecisions<'a> {
        if !self.state_consulted {
            return TombstonePrecisions::every(policy);
        }
        let mut union = policy.precisions.clone();
        union.extend_from_slice(&self.configured_for_property(property).precisions);
        TombstonePrecisions::bounded(policy, sorted_precisions(union))
    }

    /// Resolve every geometry path's write policy from the CONFIGURED policy.
    ///
    /// Walks the whole property tree, so a geometry nested in an `Element`,
    /// `Object` or `Array` gets its own policy rather than silently inheriting the
    /// workspace default.
    ///
    /// # The bug this replaces
    ///
    /// This used to read the policy straight out of the local state record and
    /// only consult the configuration when no record existed. The state record
    /// is a cache of what the index was last BUILT under, so an operator who
    /// changed the precision set through the admin surface changed nothing:
    /// every later write re-emitted the old precisions and re-confirmed the old
    /// record. Configuration is intent and wins; the state record contributes
    /// only the precisions that already hold entries, which are unioned in so a
    /// row written mid-migration stays findable under both sets.
    ///
    /// See [`crate::spatial_state::resolve`] for the union rule and for what a
    /// query is allowed to trust while the two disagree.
    ///
    /// # Cost
    ///
    /// Two point lookups per DISTINCT geometry path. An array of many elements
    /// therefore costs one pair of lookups per element — the store's cache
    /// absorbs the state read, and both collapse onto one `stops[].geo` key, so
    /// the reads are cache hits after the first.
    pub fn for_write(store: &SpatialStateStore, ctx: &IndexCtx<'_>, node: &Node) -> Self {
        let mut per_property = HashMap::new();
        let mut configured = HashMap::new();
        for (path, _) in walk_geometries(&node.properties) {
            let resolved =
                store.write_policy(ctx.tenant_id, ctx.repo_id, ctx.branch, ctx.workspace, &path);
            per_property.insert(path.clone(), resolved.write);
            configured.insert(path, resolved.configured);
        }
        Self {
            per_property,
            configured,
            default: SpatialPolicy::default(),
            state_consulted: true,
        }
    }

    /// Legacy name for [`Self::for_write`].
    ///
    /// Retained only because several write paths still call it; it no longer
    /// resolves from local state, and the name should be retired once those call
    /// sites are updated.
    pub fn from_local_state(store: &SpatialStateStore, ctx: &IndexCtx<'_>, node: &Node) -> Self {
        Self::for_write(store, ctx, node)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unresolved_policies_tombstone_at_every_precision() {
        let policies = NodeSpatialPolicies::all_default();
        let policy = SpatialPolicy::default();
        let widened = policies.tombstone_precisions("location", &policy);
        assert_eq!(widened.precisions, ALL_PRECISIONS.to_vec());
    }

    /// The tracking-profile win: with the state record consulted, a two-precision
    /// policy tombstones two precisions, not twelve.
    #[test]
    fn resolved_policies_bound_tombstones_to_configured_union_indexed() {
        let tracking = SpatialPolicy {
            precisions: sorted_precisions(vec![6, 8]),
            ..SpatialPolicy::default()
        };
        let mut per_property = HashMap::new();
        per_property.insert("position".to_string(), tracking.clone());
        let mut policies = NodeSpatialPolicies::new(per_property, SpatialPolicy::default());
        policies.state_consulted = true;

        let widened = policies.tombstone_precisions("position", &tracking);
        assert_eq!(widened.precisions, vec![8, 6]);
    }

    /// A migration window: the write policy is `configured ∪ indexed`, and the
    /// tombstone set must cover both so an entry written under either is shadowed.
    #[test]
    fn tombstone_set_covers_both_sides_of_a_policy_change() {
        let indexed_union = SpatialPolicy {
            precisions: sorted_precisions(vec![6, 8, 9, 10]),
            ..SpatialPolicy::default()
        };
        let configured_only = SpatialPolicy {
            precisions: sorted_precisions(vec![6, 8]),
            ..SpatialPolicy::default()
        };
        let mut policies = NodeSpatialPolicies::new(
            HashMap::from([("position".to_string(), configured_only)]),
            SpatialPolicy::default(),
        );
        policies.state_consulted = true;

        let widened = policies.tombstone_precisions("position", &indexed_union);
        assert_eq!(widened.precisions, vec![10, 9, 8, 6]);
    }
}
