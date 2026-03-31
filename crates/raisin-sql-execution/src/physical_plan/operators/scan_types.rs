//! Support types for physical scan operators.
//!
//! Contains enumerations and parameter structs used by scan variants
//! in the PhysicalPlan enum.

use std::fmt;

/// Reason why a TableScan was chosen instead of an index scan
///
/// This is used for EXPLAIN output to help users understand query execution.
#[derive(Debug, Clone)]
pub enum ScanReason {
    /// No indexes are configured/available in the catalog
    NoIndexAvailable,

    /// Indexes exist but none match the query predicates
    NoMatchingIndex { available: Vec<String> },

    /// Query predicates cannot be pushed to any available index
    UnsupportedPredicate { details: String },

    /// Required index exists but is not enabled
    IndexNotEnabled { index_name: String },
}

impl fmt::Display for ScanReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScanReason::NoIndexAvailable => write!(f, "no index available"),
            ScanReason::NoMatchingIndex { available } => {
                write!(f, "no matching index (available: {})", available.join(", "))
            }
            ScanReason::UnsupportedPredicate { details } => {
                write!(f, "unsupported predicate: {}", details)
            }
            ScanReason::IndexNotEnabled { index_name } => {
                write!(f, "{} not enabled", index_name)
            }
        }
    }
}

/// Vector distance metric for similarity search
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorDistanceMetric {
    /// L2 (Euclidean) distance - pgvector <-> operator
    L2,
    /// Cosine distance - pgvector <=> operator
    /// Computed as 1 - cosine_similarity
    /// Assumes vectors are normalized
    Cosine,
    /// Inner product (negative dot product) - pgvector <#> operator
    InnerProduct,
    /// Manhattan (L1) distance
    Manhattan,
    /// Hamming distance (for binary vectors)
    Hamming,
}

impl fmt::Display for VectorDistanceMetric {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VectorDistanceMetric::L2 => write!(f, "L2"),
            VectorDistanceMetric::Cosine => write!(f, "Cosine"),
            VectorDistanceMetric::InnerProduct => write!(f, "InnerProduct"),
            VectorDistanceMetric::Manhattan => write!(f, "Manhattan"),
            VectorDistanceMetric::Hamming => write!(f, "Hamming"),
        }
    }
}

impl VectorDistanceMetric {
    /// Convert to the HNSW engine's DistanceMetric type.
    pub fn to_hnsw_metric(&self) -> raisin_hnsw::DistanceMetric {
        match self {
            VectorDistanceMetric::L2 => raisin_hnsw::DistanceMetric::L2,
            VectorDistanceMetric::Cosine => raisin_hnsw::DistanceMetric::Cosine,
            VectorDistanceMetric::InnerProduct => raisin_hnsw::DistanceMetric::InnerProduct,
            VectorDistanceMetric::Manhattan => raisin_hnsw::DistanceMetric::Manhattan,
            VectorDistanceMetric::Hamming => raisin_hnsw::DistanceMetric::Hamming,
        }
    }
}

/// Index lookup type for IndexLookupJoin
#[derive(Debug, Clone)]
pub enum IndexLookupType {
    /// Lookup by node ID (uses NodeIdScan internally)
    ById,
    /// Lookup by path (uses PathIndexScan internally)
    ByPath,
}

/// Parameters for index lookup in IndexLookupJoin
#[derive(Debug, Clone)]
pub struct IndexLookupParams {
    /// Type of index lookup
    pub lookup_type: IndexLookupType,
    /// Tenant ID for storage access
    pub tenant_id: String,
    /// Repository ID
    pub repo_id: String,
    /// Branch name
    pub branch: String,
    /// Workspace for the lookup
    pub workspace: String,
    /// Table name (typically "nodes")
    pub table: String,
    /// Optional table alias
    pub alias: Option<String>,
    /// Optional column projection
    pub projection: Option<Vec<String>>,
}
