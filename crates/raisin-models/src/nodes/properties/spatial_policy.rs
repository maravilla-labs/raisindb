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

//! How a geometry-valued property is indexed: the *declared intent*.
//!
//! # Intent vs. reality — the distinction that keeps this safe
//!
//! This module holds **replicated intent** only: what the user asked for. It is
//! part of the NodeType / Workspace records, so it converges across the cluster
//! like any other data.
//!
//! What is actually on a given node's disk is *local* state (a
//! `SpatialIndexState` record in the `INDEX_STATUS` column family, owned by
//! `raisin-rocksdb`). Queries read the local state; writes maintain
//! `configured ∪ indexed` while the two differ. Conflating the two is the trap:
//! reading intent at query time would make a mid-rollout cluster answer the same
//! question differently on different nodes.
//!
//! Nothing about spatial indexing may come from `config.toml` or an environment
//! variable. If it is not replicated data, two nodes can disagree.
//!
//! # Precision is a performance knob, never a correctness knob
//!
//! Changing [`SpatialPolicy::precisions`] must never change which rows a query
//! returns — only how many candidate cells are scanned. The cell-planning
//! algorithm in `raisin-rocksdb` guarantees that by always choosing a cell size
//! *at least* as large as the query radius (or ring-expanding when it cannot),
//! so a sub-metre query against a coarse index over-fetches and post-filters
//! rather than silently returning nothing.

use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::schema::PropertyValueSchema;

/// Geohash precisions written for every indexed geometry, coarsest first.
///
/// Approximate cell radii: 2 → 1250 km, 4 → 39 km, 6 → 1.2 km, 7 → 153 m,
/// 8 → 38 m, 9 → 4.8 m, 10 → 1.2 m, 11 → 0.15 m. That spans global down to
/// sub-metre indoor work.
///
/// **Cost, stated plainly:** 8 keys per geometry-valued property per revision,
/// up from the 5 of `[4,5,6,7,8]` — i.e. 1.6× the spatial-index key count. Total
/// per-node write cost rises much less than 1.6×, because `SPATIAL_INDEX` is one
/// of roughly eight column families a node write touches.
pub const INDEX_PRECISIONS_DEFAULT: &[usize] = &[2, 4, 6, 7, 8, 9, 10, 11];

/// Hard ceiling on cells scanned for one spatial predicate.
///
/// Beyond this the planner must report the index as *not covering* and fall back
/// to a scan with the predicate retained as a residual filter — slow and correct
/// rather than fast and truncated.
pub const MAX_SCAN_CELLS: usize = 1024;

/// Hard ceiling on cells written per precision when covering a non-point
/// geometry's extent ([`SpatialCoverMode::Extent`]).
///
/// This is the one place the write budget can blow past 2×: a country-sized
/// polygon would otherwise want thousands of cells per precision.
pub const MAX_COVER_CELLS: usize = 32;

/// Version of the write-time SRID normaliser, folded into
/// [`SpatialPolicy::policy_hash`].
///
/// Bump this whenever the tier-1 projection maths changes, so existing entries
/// are recognised as stale and reindexed instead of being silently mixed with
/// entries produced under a different convention.
///
/// **This must stay equal to `raisin_proj::NORMALIZER_VERSION`.** It is duplicated
/// rather than imported because `raisin-models` is depended on by every crate
/// including the thin clients and must not pull in `geo`. `raisin-geometry`, which
/// depends on both, asserts the equality in a test.
pub const SPATIAL_NORMALIZER_VERSION: u32 = 1;

/// Valid geohash precision range.
pub const PRECISION_RANGE: std::ops::RangeInclusive<usize> = 1..=12;

/// Normalise a concrete geometry **property path** into the key a spatial policy
/// is configured under.
///
/// # Why a normalisation step exists at all
///
/// A geometry can live anywhere in a node's property tree, and the index key
/// embeds the *concrete* dot path the value was found at
/// (`venue.geo`, `hero.map_pin`, `stops.0.geo` — see
/// `raisin_rocksdb::indexing::spatial_walk`). For an object or an element that is
/// exactly the modelled field name and needs no translation. For an **array** it
/// is not: `stops.0.geo`, `stops.1.geo`, `stops.2.geo` … are unboundedly many
/// distinct paths for ONE modelled field, and no operator could configure a
/// precision set for each.
///
/// So array indices — and only array indices — collapse into the preceding
/// segment as `[]` when resolving policy:
///
/// | concrete path   | policy key      |
/// |-----------------|-----------------|
/// | `location`      | `location`      |
/// | `venue.geo`     | `venue.geo`     |
/// | `hero.map_pin`  | `hero.map_pin`  |
/// | `stops.0.geo`   | `stops[].geo`   |
/// | `tags.3`        | `tags[]`        |
/// | `stops[].geo`   | `stops[].geo`   |
///
/// **The index key still uses the concrete path.** Only policy lookup normalises.
///
/// # This must be the ONLY implementation
///
/// It is called from both sides of the same question: the configuration surface
/// (`SpatialWorkspaceSchema::scope` / `set_scope` / `clear_scope` and
/// [`resolve_spatial_policy`]) and the local index-state record
/// (`raisin_rocksdb::spatial_state::spatial_state_key`, which the query planner's
/// availability check goes through). If the two sides normalised differently the
/// planner would decide an indexed field is unindexed and take the fallback scan
/// forever — correct results, permanently bad performance, and no error anywhere.
///
/// Idempotent: normalising an already-normalised key returns it unchanged.
pub fn policy_key_for_path(path: &str) -> String {
    // Fast path: no numeric segment can exist without a dot.
    if !path.contains('.') {
        return path.to_string();
    }

    let mut segments: Vec<String> = Vec::new();
    for segment in path.split('.') {
        let is_index = !segment.is_empty() && segment.bytes().all(|b| b.is_ascii_digit());
        match (is_index, segments.last_mut()) {
            (true, Some(previous)) => previous.push_str("[]"),
            // A leading numeric segment cannot be an array index of anything, so
            // it is kept verbatim rather than silently dropped.
            _ => segments.push(segment.to_string()),
        }
    }
    segments.join(".")
}

/// Whether a property path names a set of geometries rather than exactly one.
///
/// The `[]` wildcard is only ever written by a *query*; the walker never produces
/// one. A wildcard path can be evaluated per row but can NOT drive a single
/// cell-ring index scan, because each concrete array element occupies its own
/// index-key namespace — hence this predicate, which the planner uses to refuse
/// the index path rather than scan an empty prefix.
pub fn is_wildcard_property_path(path: &str) -> bool {
    path.contains("[]")
}

/// Which cells a non-point geometry occupies in the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum SpatialCoverMode {
    /// Index the geometry's centroid only. The default, and what RaisinDB has
    /// always done. Correct — bounding-box pushdown is an *envelope* filter and
    /// the original predicate is retained as a residual filter — but a large
    /// polygon is only found via cells near its centroid, so a query over its
    /// far interior must fall back to a wider scan.
    #[default]
    Centroid,
    /// Also index the cells covering the geometry's bounding box, capped at
    /// [`MAX_COVER_CELLS`] per precision. Better selectivity, more write cost.
    Extent,
}

/// Declared spatial indexing intent for one property.
///
/// Every field is optional so that the precedence chain in
/// [`resolve_spatial_policy`] can fill gaps from a broader scope.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct SpatialPropertySchema {
    /// Geohash precisions to index at. `None` inherits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub precisions: Option<Vec<usize>>,

    /// Default SRID for values of this property that carry no explicit `srid`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub srid: Option<u32>,

    /// Name of a sibling property whose value discriminates candidates inside a
    /// cell — a building floor/level label being the motivating case.
    ///
    /// A floor is a **discrete ordinal label** ("L2", "B1", "M"), not a
    /// coordinate: two shops on "level 2" of different terminals have different
    /// altitudes, and a shop and the void above it share an altitude but not a
    /// level. So it is an ordinary property, recorded in the index *value* rather
    /// than the key, and used to reject candidates before the per-candidate node
    /// fetch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bucket_property: Option<String>,

    /// Centroid-only or extent cover. `None` inherits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<SpatialCoverMode>,
}

/// Workspace-level spatial defaults, with per-property overrides.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct SpatialWorkspaceSchema {
    /// Applies to every geometry property in the workspace.
    #[serde(default)]
    pub default: SpatialPropertySchema,

    /// Overrides keyed by property name.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub properties: HashMap<String, SpatialPropertySchema>,
}

impl SpatialWorkspaceSchema {
    /// The declaration for one scope: `Some(property)` is the per-property
    /// override, `None` is the workspace default.
    ///
    /// Returns an empty declaration rather than `Option`, because "declared
    /// nothing" and "declared all-inherit" are the same thing to
    /// [`resolve_spatial_policy`].
    /// Nested paths are normalised through [`policy_key_for_path`], so a caller
    /// may pass either the configured spelling (`stops[].geo`) or a concrete one
    /// (`stops.3.geo`) and reach the same declaration.
    pub fn scope(&self, property: Option<&str>) -> SpatialPropertySchema {
        match property {
            Some(name) => self
                .properties
                .get(&policy_key_for_path(name))
                .cloned()
                .unwrap_or_default(),
            None => self.default.clone(),
        }
    }

    /// Replace the declaration for one scope.
    pub fn set_scope(&mut self, property: Option<&str>, settings: SpatialPropertySchema) {
        match property {
            Some(name) => {
                self.properties.insert(policy_key_for_path(name), settings);
            }
            None => self.default = settings,
        }
    }

    /// Remove the declaration for one scope, reverting to the next scope in the
    /// precedence chain.
    pub fn clear_scope(&mut self, property: Option<&str>) {
        match property {
            Some(name) => {
                self.properties.remove(&policy_key_for_path(name));
            }
            None => self.default = SpatialPropertySchema::default(),
        }
    }

    /// Whether this schema declares nothing at all.
    ///
    /// A caller storing the schema should drop it entirely when this is true, so a
    /// reset leaves no empty husk on the record.
    pub fn is_inert(&self) -> bool {
        self.properties.is_empty() && self.default == SpatialPropertySchema::default()
    }

    /// Apply a scoped edit and return the schema to persist, or `None` when nothing
    /// is left to declare.
    ///
    /// The ONE place a scoped spatial-config edit is expressed, so the SQL engine and
    /// the storage layer cannot drift on what `RESET` means.
    pub fn edited(
        existing: Option<Self>,
        property: Option<&str>,
        settings: Option<SpatialPropertySchema>,
    ) -> Option<Self> {
        let mut schema = existing.unwrap_or_default();
        match settings {
            Some(s) => schema.set_scope(property, s),
            None => schema.clear_scope(property),
        }
        if schema.is_inert() {
            None
        } else {
            Some(schema)
        }
    }
}

/// The fully resolved policy for one (workspace, property) pair.
///
/// Produced only by [`resolve_spatial_policy`], so all call sites agree.
#[derive(Debug, Clone, PartialEq)]
pub struct SpatialPolicy {
    /// Precisions to write, sorted **descending** (finest first), deduped, each
    /// within [`PRECISION_RANGE`].
    pub precisions: Vec<usize>,
    /// Schema-default SRID for unlabelled values, if any.
    pub srid: Option<u32>,
    /// Discriminator property recorded in the index value.
    pub bucket_property: Option<String>,
    /// Whether non-point geometries are indexed by extent.
    pub cover: SpatialCoverMode,
}

impl Default for SpatialPolicy {
    fn default() -> Self {
        SpatialPolicy {
            precisions: sorted_precisions(INDEX_PRECISIONS_DEFAULT.to_vec()),
            srid: None,
            bucket_property: None,
            cover: SpatialCoverMode::Centroid,
        }
    }
}

impl SpatialPolicy {
    /// True when non-point geometries should be covered by extent.
    pub fn covers_non_point(&self) -> bool {
        self.cover == SpatialCoverMode::Extent
    }

    /// The finest (largest) configured precision.
    pub fn finest(&self) -> usize {
        self.precisions.first().copied().unwrap_or(8)
    }

    /// The coarsest (smallest) configured precision.
    pub fn coarsest(&self) -> usize {
        self.precisions.last().copied().unwrap_or(8)
    }

    /// A stable fingerprint of everything that affects the *bytes* this policy
    /// produces, including [`SPATIAL_NORMALIZER_VERSION`].
    ///
    /// Stamped into every index entry and into the local index-state record, so
    /// a configuration change schedules a reindex rather than silently mixing
    /// two conventions, and a cross-node skew is visible to an operator.
    ///
    /// FNV-1a over a canonical byte encoding — deliberately hand-rolled so the
    /// value is stable across compiler and std-library versions, which
    /// `DefaultHasher` explicitly is not.
    pub fn policy_hash(&self) -> u64 {
        let mut h = Fnv::new();
        h.write_u32(SPATIAL_NORMALIZER_VERSION);
        h.write_u32(self.precisions.len() as u32);
        for p in &self.precisions {
            h.write_u32(*p as u32);
        }
        h.write_u32(self.srid.unwrap_or(0));
        match &self.bucket_property {
            None => h.write_u32(0),
            Some(name) => {
                h.write_u32(1);
                h.write_bytes(name.as_bytes());
            }
        }
        h.write_u32(match self.cover {
            SpatialCoverMode::Centroid => 0,
            SpatialCoverMode::Extent => 1,
        });
        h.finish()
    }
}

/// Resolve the effective policy for a property.
///
/// Precedence, highest first:
/// 1. the NodeType property schema's `spatial` block,
/// 2. the workspace's per-property override,
/// 3. the workspace default,
/// 4. the server constants.
///
/// Each *field* resolves independently, so a NodeType can set precisions while
/// inheriting the workspace's bucket property.
///
/// `property_name` may be a nested dot path (`venue.geo`, `stops.3.geo`); it is
/// normalised through [`policy_key_for_path`] before the per-property override is
/// looked up. Resolution is **exact-match only** — `venue` does NOT supply a
/// policy for `venue.geo`. Prefix inheritance would need a longest-match rule
/// that the admin surface, the state record, the rebuild job and the planner
/// would each have to implement identically, and any disagreement strands entries.
///
/// This is the only function permitted to compute a [`SpatialPolicy`]: five call
/// sites depend on them agreeing.
pub fn resolve_spatial_policy(
    property_schema: Option<&PropertyValueSchema>,
    workspace_spatial: Option<&SpatialWorkspaceSchema>,
    property_name: &str,
) -> SpatialPolicy {
    let policy_key = policy_key_for_path(property_name);
    let from_type = property_schema.and_then(|s| s.spatial.as_ref());
    let from_ws_prop = workspace_spatial.and_then(|w| w.properties.get(&policy_key));
    let from_ws_default = workspace_spatial.map(|w| &w.default);

    // Ordered highest-precedence first; `find_map` takes the first that sets the
    // field, so a partially specified narrow scope still inherits the rest.
    let layers = [from_type, from_ws_prop, from_ws_default];
    let pick = |f: &dyn Fn(&SpatialPropertySchema) -> bool| -> Option<&SpatialPropertySchema> {
        layers.iter().flatten().copied().find(|s| f(s))
    };

    let precisions = pick(&|s| s.precisions.as_ref().is_some_and(|p| !p.is_empty()))
        .and_then(|s| s.precisions.clone())
        .unwrap_or_else(|| INDEX_PRECISIONS_DEFAULT.to_vec());

    SpatialPolicy {
        precisions: sorted_precisions(precisions),
        srid: pick(&|s| s.srid.is_some()).and_then(|s| s.srid),
        bucket_property: pick(&|s| s.bucket_property.is_some())
            .and_then(|s| s.bucket_property.clone()),
        cover: pick(&|s| s.cover.is_some())
            .and_then(|s| s.cover)
            .unwrap_or_default(),
    }
}

/// Canonicalize a precision list: clamp to [`PRECISION_RANGE`], dedupe, sort
/// finest-first, and fall back to the default if nothing valid remains.
///
/// Canonicalization has to happen here rather than at the parser, because
/// [`SpatialPolicy::policy_hash`] must not differ merely because two operators
/// wrote the same set in a different order.
pub fn sorted_precisions(mut precisions: Vec<usize>) -> Vec<usize> {
    precisions.retain(|p| PRECISION_RANGE.contains(p));
    precisions.sort_unstable_by(|a, b| b.cmp(a));
    precisions.dedup();
    if precisions.is_empty() {
        precisions = INDEX_PRECISIONS_DEFAULT.to_vec();
        precisions.sort_unstable_by(|a, b| b.cmp(a));
    }
    precisions
}

/// FNV-1a 64. Small, dependency-free, and — unlike `DefaultHasher` — documented
/// to produce the same value forever, which a persisted fingerprint requires.
pub(crate) struct Fnv(u64);

impl Fnv {
    pub(crate) fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    pub(crate) fn write_bytes(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= *b as u64;
            self.0 = self.0.wrapping_mul(0x100_0000_01b3);
        }
    }
    pub(crate) fn write_u32(&mut self, v: u32) {
        self.write_bytes(&v.to_be_bytes());
    }
    pub(crate) fn finish(self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests;
