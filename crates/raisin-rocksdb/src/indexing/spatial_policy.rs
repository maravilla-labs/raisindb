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

use super::IndexCtx;
use crate::spatial_state::SpatialStateStore;
use raisin_models::nodes::properties::{PropertyValue, SpatialPolicy};
use raisin_models::nodes::Node;
use std::collections::HashMap;

/// The resolved spatial policy for each geometry property of a node.
#[derive(Debug, Clone, Default)]
pub struct NodeSpatialPolicies {
    /// What the write must EMIT: configured ∪ indexed while the two differ.
    per_property: HashMap<String, SpatialPolicy>,
    /// What the configuration DECLARES, kept separately so the state record a
    /// first write creates is stamped with intent rather than with the migration
    /// union.
    configured: HashMap<String, SpatialPolicy>,
    default: SpatialPolicy,
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
        }
    }

    /// Resolve the policy to WRITE for one property.
    pub fn for_property(&self, property: &str) -> &SpatialPolicy {
        self.per_property.get(property).unwrap_or(&self.default)
    }

    /// The DECLARED policy for one property, i.e. configured intent without the
    /// migration union. Falls back to the write policy when the property was not
    /// resolved (the two are identical whenever nothing is in flight).
    pub fn configured_for_property(&self, property: &str) -> &SpatialPolicy {
        self.configured
            .get(property)
            .unwrap_or_else(|| self.for_property(property))
    }

    /// Resolve every geometry property's write policy from the CONFIGURED policy.
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
    pub fn for_write(store: &SpatialStateStore, ctx: &IndexCtx<'_>, node: &Node) -> Self {
        let mut per_property = HashMap::new();
        let mut configured = HashMap::new();
        for (name, value) in &node.properties {
            if !matches!(value, PropertyValue::Geometry(_)) {
                continue;
            }
            let resolved =
                store.write_policy(ctx.tenant_id, ctx.repo_id, ctx.branch, ctx.workspace, name);
            per_property.insert(name.clone(), resolved.write);
            configured.insert(name.clone(), resolved.configured);
        }
        Self {
            per_property,
            configured,
            default: SpatialPolicy::default(),
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
