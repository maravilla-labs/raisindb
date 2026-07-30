//! Spatial index repository trait for geohash-based geospatial queries.
//!
//! This module defines the interface for spatial indexing operations using
//! geohash-based indexing for PostGIS-compatible ST_* queries.

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::{GeoJson, SpatialCoverMode, SpatialPolicy};

/// Entry from spatial index scan
#[derive(Debug, Clone)]
pub struct SpatialIndexEntry {
    /// Node ID
    pub node_id: String,
    /// Geohash at which this entry was indexed
    pub geohash: String,
    /// Revision at which this entry was created
    pub revision: HLC,
}

/// Result from a proximity query
#[derive(Debug, Clone)]
pub struct ProximityResult {
    /// Node ID
    pub node_id: String,
    /// Distance from query point in meters
    pub distance_meters: f64,
    /// The node's geometry centroid (lon, lat)
    pub centroid: (f64, f64),
    /// The geometry's WGS84 bounding box `[min_lon, min_lat, max_lon, max_lat]`.
    ///
    /// Read straight out of the index value, so a caller can apply an envelope
    /// filter without fetching the node. Collapses to the centroid for entries
    /// written before the value record carried a bbox.
    pub bbox: [f64; 4],
    /// Altitude extent in metres, when the geometry carries a Z ordinate.
    pub z_range: Option<(f64, f64)>,
    /// The SRID the geometry was stored in (index keys are always 4326).
    pub srid: u32,
    /// The discriminator bucket value (e.g. a floor label), when configured.
    pub bucket: Option<String>,
}

/// A filter applied to index candidates **before** any node record is fetched.
///
/// Every field is optional and every field is a *superset* filter: it may only
/// reject candidates that provably cannot match. The original SQL predicate
/// always stays in the residual filter, so a wrong filter here can never turn
/// into a wrong answer — only a missed optimisation.
#[derive(Debug, Clone, Default)]
pub struct SpatialPreFilter {
    /// Require `SpatialEntry.bucket == Some(value)`.
    ///
    /// Entries written before the value record existed carry no bucket and are
    /// never rejected, because rejecting them would drop a real match.
    pub bucket_eq: Option<String>,
    /// Require the candidate's bounding box to intersect this envelope
    /// (`[min_lon, min_lat, max_lon, max_lat]`).
    pub bbox: Option<[f64; 4]>,
}

impl SpatialPreFilter {
    /// Whether this filter would reject nothing (the common case).
    pub fn is_noop(&self) -> bool {
        self.bucket_eq.is_none() && self.bbox.is_none()
    }
}

/// What the local spatial index can actually answer for one (workspace, property).
///
/// **Fails closed on purpose.** The previous `has_spatial_index()` returned a
/// hardcoded `true` on the grounds that "the spatial_index CF is always present",
/// and the planner then stripped the `ST_DWithin` predicate from the residual
/// filter — so an unpopulated index returned ZERO ROWS with no fallback and no
/// warning. Any answer other than [`SpatialAvailability::Ready`] must make the
/// planner keep the predicate and take an ordinary access path.
#[derive(Debug, Clone, PartialEq)]
pub enum SpatialAvailability {
    /// The index is built and may be trusted for the precisions listed.
    Ready {
        /// Precisions the index was **actually built at** (not the configured
        /// set). Query planning must use these, so reconfiguring can never make
        /// data invisible.
        precisions: Vec<usize>,
        /// Highest revision covered by the build.
        built_through: HLC,
        /// The node property whose value is stored as the entry's discriminator
        /// bucket, when configured.
        bucket_property: Option<String>,
    },
    /// No index state record exists: either a brand-new deployment or data
    /// written before this release. The planner must fall back to a scan with the
    /// spatial predicate retained, and a reindex should be scheduled.
    NotBuilt,
    /// A state record exists but could not be used. Carries the reason so
    /// `EXPLAIN` can print it.
    Unusable(String),
}

impl SpatialAvailability {
    /// Whether the index may be trusted as a complete access path.
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }

    /// The precisions to plan against, or an empty slice when not ready.
    pub fn precisions(&self) -> &[usize] {
        match self {
            Self::Ready { precisions, .. } => precisions,
            _ => &[],
        }
    }
}

/// How far a spatial index build has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpatialBuildPhase {
    /// Fully built and queryable.
    Ready,
    /// A build is in progress; `indexed_precisions` still describes the OLD
    /// entry set.
    ///
    /// That set is complete only while the build is NOT changing the policy —
    /// a rebuild under a new precision set tombstones and re-emits node by node,
    /// so mid-build the entries are a mixture of both. Narrowing the answer to
    /// what is provably complete is the backend's job; the RocksDB backend does
    /// it in `spatial_state::resolve::availability_in_rebuild_window`, which
    /// serves the overlap of the two precision sets and reports `Unusable`
    /// (planner falls back to a scan) when there is none.
    Building,
    /// Known to need a build (e.g. discovered by the integrity sweep).
    NotBuilt,
}

/// The **local, per-node** record of what this instance's spatial index bytes
/// actually contain for one (workspace, property).
///
/// Deliberately distinct from the *configured* policy, which is replicated data
/// living on the NodeType / WorkspaceConfig. Conflating the two is the trap:
/// queries must read this (reality) while writes reconcile towards the
/// configuration (intent). That split is what makes reconfiguration unable to
/// make data invisible, and what makes a cluster mid-rollout return identical
/// result sets at differing latency.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SpatialIndexState {
    /// Record format version.
    pub v: u8,
    /// The policy hash the current entries were produced under.
    pub policy_hash: u64,
    /// Precisions the entries exist at.
    pub precisions: Vec<usize>,
    /// Highest revision covered.
    pub built_through: HLC,
    /// Configured discriminator property, mirrored here so the sync write path
    /// can resolve it without an async schema load.
    pub bucket_property: Option<String>,
    /// Configured cover mode, mirrored for the same reason.
    pub cover: SpatialCoverMode,
    /// Build phase.
    pub phase: SpatialBuildPhase,
    /// Resume cursor (last NODES key processed) for a `Building` phase.
    pub cursor: Option<Vec<u8>>,
    /// Progress counter.
    pub nodes_indexed: u64,
}

impl SpatialIndexState {
    /// Current record format version.
    pub const VERSION: u8 = 1;

    /// A fresh `Ready` record for a policy that has just been applied.
    pub fn ready(policy: &SpatialPolicy, built_through: HLC) -> Self {
        Self {
            v: Self::VERSION,
            policy_hash: policy.policy_hash(),
            precisions: policy.precisions.clone(),
            built_through,
            bucket_property: policy.bucket_property.clone(),
            cover: policy.cover,
            phase: SpatialBuildPhase::Ready,
            cursor: None,
            nodes_indexed: 0,
        }
    }

    /// Reconstruct the policy these entries were written under.
    ///
    /// This is what lets the sync write and tombstone paths agree on the cell set
    /// without loading a NodeType: the state record is the local cache of the
    /// resolved policy.
    pub fn policy(&self) -> SpatialPolicy {
        SpatialPolicy {
            precisions: self.precisions.clone(),
            srid: None,
            bucket_property: self.bucket_property.clone(),
            cover: self.cover,
        }
    }

    /// The availability this record implies, considering the record ALONE.
    ///
    /// A caller that can also see the *configured* policy must reconcile the two
    /// before trusting this — see [`SpatialBuildPhase::Building`].
    pub fn availability(&self) -> SpatialAvailability {
        if self.v != Self::VERSION {
            return SpatialAvailability::Unusable(format!(
                "spatial index state record version {} is not supported (expected {})",
                self.v,
                Self::VERSION
            ));
        }
        if self.precisions.is_empty() {
            return SpatialAvailability::Unusable(
                "spatial index state record lists no precisions".to_string(),
            );
        }
        match self.phase {
            // `Building` still describes a complete OLD entry set, so it is
            // queryable. Only `NotBuilt` forces the fallback.
            SpatialBuildPhase::Ready | SpatialBuildPhase::Building => SpatialAvailability::Ready {
                precisions: self.precisions.clone(),
                built_through: self.built_through,
                bucket_property: self.bucket_property.clone(),
            },
            SpatialBuildPhase::NotBuilt => SpatialAvailability::NotBuilt,
        }
    }
}

/// Read-only access to the local spatial index state.
///
/// The SQL planner's index catalog holds one of these. When it holds `None` — unit
/// tests, the memory backend — the answer must be [`SpatialAvailability::NotBuilt`],
/// never an optimistic `Ready`.
pub trait SpatialStateSource: Send + Sync {
    /// What the local index can answer for this (workspace, property).
    fn spatial_availability(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property: &str,
    ) -> SpatialAvailability;
}

/// Spatial index repository trait for geospatial queries.
///
/// This trait defines the interface for geohash-based spatial indexing,
/// supporting PostGIS-compatible ST_DWithin and k-NN queries.
///
/// # Implementation Notes
///
/// Implementations use geohash multi-precision indexing at the precisions the
/// resolved [`SpatialPolicy`] specifies. All methods are MVCC-aware with revision
/// support, and all query methods are required to be **complete**: a returned set
/// may contain false positives (the caller re-filters) but must never omit a
/// matching node.
pub trait SpatialIndexRepository: Send + Sync {
    /// Index a geometry property for a node at every precision in `policy`.
    #[allow(clippy::too_many_arguments)]
    fn index_geometry(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        property_name: &str,
        geometry: &GeoJson,
        revision: &HLC,
        policy: &SpatialPolicy,
        bucket: Option<&str>,
    ) -> Result<()>;

    /// Tombstone the index entries for `old_geometry`.
    ///
    /// Takes the OLD geometry rather than scanning for the node's entries: the
    /// cells are *derived*, so this is O(precisions) with zero reads instead of
    /// O(all geometries in the workspace).
    fn unindex_geometry(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node_id: &str,
        property_name: &str,
        old_geometry: &GeoJson,
        revision: &HLC,
        policy: &SpatialPolicy,
    ) -> Result<()>;

    /// Find nodes within a given distance of a point (ST_DWithin).
    ///
    /// # Completeness
    /// Returns every node whose indexed position is within `radius_meters`, or an
    /// error if the index cannot cover the requested radius. It must never
    /// silently return a partial set.
    ///
    /// # Returns
    /// Nodes within the radius, sorted by distance (nearest first)
    #[allow(clippy::too_many_arguments)]
    fn find_within_radius(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property_name: &str,
        center_lon: f64,
        center_lat: f64,
        radius_meters: f64,
        max_revision: &HLC,
        limit: usize,
        precisions: &[usize],
        prefilter: &SpatialPreFilter,
    ) -> Result<Vec<ProximityResult>>;

    /// Find k nearest neighbors to a point (k-NN query).
    ///
    /// Uses expanding-ring search with a sound stopping rule: expansion continues
    /// until the k-th best distance is no greater than the distance to the nearest
    /// unexplored ring boundary. "Found enough candidates" is NOT a valid stopping
    /// condition — a nearer node can live one ring further out.
    #[allow(clippy::too_many_arguments)]
    fn find_nearest(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        property_name: &str,
        center_lon: f64,
        center_lat: f64,
        k: usize,
        max_revision: &HLC,
        precisions: &[usize],
        prefilter: &SpatialPreFilter,
    ) -> Result<Vec<ProximityResult>>;
}
