// SPDX-License-Identifier: BSL-1.1

//! How an HNSW index learns what shape it is.
//!
//! The shape of a vector index is not a property of the process. It is a
//! property of the *partition* — of the embedder whose vectors it holds.
//! `nomic-embed-text` is 768 wide, `text-embedding-3-small` is 1536, `bge-m3`
//! is 1024; the metric a query uses must be the metric the graph was built
//! under; and an i8 index is a quarter the size of an f32 one at the same
//! width. Baking any of those into the engine at construction time means every
//! tenant whose configuration disagrees gets an index that rejects every vector
//! its own job handler produces, with the vector already durably written to
//! `cf::EMBEDDINGS` and nothing in the index. The symptom is "vector queries
//! return zero rows while the embedding count climbs".
//!
//! So the engine does not hold a shape. It holds a *resolver*, consulted once
//! per index load (i.e. on a cache miss, not per vector), and it creates a
//! missing index at whatever that resolver reports. The construction-time
//! number survives only as [`FALLBACK_DIMENSIONS`], used when a tenant has no
//! embedding config at all.
//!
//! # One resolver, not three
//!
//! It is deliberately [`IndexSpecResolver`] and not `EmbeddingDimsResolver` +
//! `MetricResolver` + `QuantizationResolver`. Those would be three reads of the
//! same row that can disagree, which is this codebase's dominant bug class —
//! and the metric had *already* drifted that way before this trait existed:
//! `TenantEmbeddingConfig::distance_metric` was settable, rendered by `SHOW`,
//! parsed by `ALTER EMBEDDING CONFIG` and consumed at QUERY time, while the
//! index was always BUILT with `DistanceMetric::default()`. A tenant could
//! configure a query metric its own graph had never been built under. The spec
//! an index is created with is now the spec its queries are answered under,
//! because there is one struct and one read.
//!
//! The implementation lives in `raisin-rocksdb`
//! (`TenantEmbeddingSpecResolver`) and reads the *same* `TenantEmbeddingConfig`
//! that the `EmbeddingGenerate` job handler and `REBUILD VECTOR INDEX` already
//! read — one source of truth, so they cannot drift.

use crate::partition::PartitionId;
use crate::types::{DistanceMetric, HnswParams, QuantizationType};

/// Width used for an index whose tenant has no embedding configuration.
///
/// Historically this was the engine's only width. It is OpenAI's
/// `text-embedding-3-small`/`ada-002` size, kept so that a tenant that never
/// called `ALTER EMBEDDING CONFIG` behaves exactly as before.
pub const FALLBACK_DIMENSIONS: usize = 1536;

/// Everything that decides how one index's graph is BUILT.
///
/// Persisted in the `.hnsw.meta` sidecar, so an index that outlives a config
/// change is still read with the shape it was written with — a reload compares
/// the two and says so loudly rather than answering with nonsense.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IndexSpec {
    /// Vector width.
    pub dimensions: usize,

    /// Distance metric. **Must** be the metric queries use; see the module doc.
    pub metric: DistanceMetric,

    /// Graph connectivity / expansion, and the scalar kind vectors are stored
    /// at.
    ///
    /// Quantization lives HERE and only here rather than as a fourth field
    /// beside `params.quantization`: two copies of one setting is exactly the
    /// mirrored state that drifts. Read it through
    /// [`IndexSpec::quantization`].
    pub params: HnswParams,
}

impl IndexSpec {
    /// A spec at `dimensions`, with everything else defaulted.
    pub fn new(dimensions: usize) -> Self {
        Self {
            dimensions,
            metric: DistanceMetric::default(),
            params: HnswParams::default(),
        }
    }

    /// Set the distance metric.
    pub fn with_metric(mut self, metric: DistanceMetric) -> Self {
        self.metric = metric;
        self
    }

    /// Set the scalar kind vectors are stored at.
    ///
    /// `Int8` is a config change only: usearch casts the caller's `&[f32]` into
    /// the index's scalar kind on insert and `scalar_words() == dimensions` for
    /// f32/f16/i8, so add and search signatures are untouched. The cast
    /// L2-normalises and scales to ±127 and assumes a dot-product-like metric —
    /// which is why the metric is pinned in the same struct rather than
    /// resolved separately.
    pub fn with_quantization(mut self, quantization: QuantizationType) -> Self {
        self.params.quantization = quantization;
        self
    }

    /// Set graph parameters (connectivity / expansion), preserving the
    /// quantization already on this spec.
    pub fn with_params(mut self, params: HnswParams) -> Self {
        let quantization = self.params.quantization;
        self.params = params;
        self.params.quantization = quantization;
        self
    }

    /// The scalar kind vectors are stored at.
    pub fn quantization(&self) -> QuantizationType {
        self.params.quantization
    }
}

/// Resolves the configured shape of one `(tenant, repo, branch, partition)`
/// index.
///
/// Returning `None` means "no configuration exists"; the engine then falls back
/// to [`FALLBACK_DIMENSIONS`] with default metric and quantization. Returning
/// an error is not modelled on purpose: a resolver that cannot answer must not
/// make an otherwise healthy index unloadable, so implementations log and
/// return `None`.
pub trait IndexSpecResolver: Send + Sync {
    /// The configured shape, or `None` when the tenant has no config.
    fn spec_for(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &PartitionId,
    ) -> Option<IndexSpec>;

    /// The partition a caller with no embedder identity of its own should read.
    ///
    /// The SQL query path is the case that matters: it embeds the query text
    /// through the tenant's configured provider and then has to ask *which
    /// index that vector belongs in*. Answering it here, off the same config
    /// row, is what keeps the read side and the write side on one partition.
    /// `None` when the tenant has no embedding configuration.
    fn default_text_partition(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Option<PartitionId>;
}
